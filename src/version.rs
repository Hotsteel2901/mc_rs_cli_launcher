//! Minecraft metadata, libraries, native archives and content-addressed assets.
use crate::error::{AppError, AppResult};
use crate::{http, progress, util};
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MC_MANIFEST: &str = "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json";

struct LibraryFile {
    url: String,
    path: PathBuf,
    hash: Option<String>,
    native: bool,
}

pub struct VersionManager {
    pub game_dir: PathBuf,
    pub versions_dir: PathBuf,
    pub libraries_dir: PathBuf,
    pub assets_dir: PathBuf,
}

fn read_json(path: &Path) -> AppResult<serde_json::Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn valid_manifest(value: &serde_json::Value) -> bool {
    value["versions"].is_array() && value["latest"]["release"].is_string()
}

fn valid_version(value: &serde_json::Value, id: &str) -> bool {
    value["id"].as_str() == Some(id)
        && value["mainClass"].is_string()
        && value["libraries"].is_array()
        && value["downloads"]["client"]["url"].is_string()
}

impl VersionManager {
    pub fn new(game_dir: &Path) -> Self {
        Self {
            game_dir: game_dir.to_path_buf(),
            versions_dir: game_dir.join("versions"),
            libraries_dir: game_dir.join("libraries"),
            assets_dir: game_dir.join("assets"),
        }
    }

    pub fn fetch_manifest(&self) -> AppResult<serde_json::Value> {
        self.fetch_manifest_inner(true)
    }

    /// Same lookup as [`Self::fetch_manifest`] but silent on stale-cache
    /// fallback, for full-screen frontends that own the terminal.
    pub fn fetch_manifest_result(&self) -> AppResult<serde_json::Value> {
        self.fetch_manifest_inner(false)
    }

    fn fetch_manifest_inner(&self, warn: bool) -> AppResult<serde_json::Value> {
        let path = self.game_dir.join("version_manifest_v2.json");
        let cached = read_json(&path).ok().filter(valid_manifest);
        let fresh = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|time| SystemTime::now().duration_since(time).ok())
            .is_some_and(|age| age.as_secs() < 300);
        if fresh {
            if let Some(ref cache) = cached {
                return Ok(cache.clone());
            }
        }
        let fetched = http::http_get(MC_MANIFEST)
            .map_err(AppError::Http)
            .and_then(|(status, body)| {
                if status != 200 {
                    return Err(AppError::Http(format!(
                        "Version manifest returned HTTP {status}"
                    )));
                }
                let value: serde_json::Value = serde_json::from_slice(&body)?;
                if !valid_manifest(&value) {
                    return Err("Invalid version manifest schema".into());
                }
                util::atomic_write(&path, &body)?;
                Ok(value)
            });
        match (fetched, cached) {
            (Ok(value), _) => Ok(value),
            (Err(error), Some(cache)) => {
                if warn {
                    crate::warn_msg!("Cannot refresh manifest ({error}); using cached metadata.");
                }
                Ok(cache)
            }
            (Err(error), None) => Err(error),
        }
    }

    pub fn get_version_info(
        &self,
        version_id: Option<&str>,
    ) -> AppResult<(String, serde_json::Value)> {
        let mut id = version_id.unwrap_or("latest").to_string();
        util::validate_component(&id)?;
        // Installed versions are usable even when the global manifest is offline/stale.
        if id != "latest" && id != "latest-snapshot" {
            let path = self.versions_dir.join(&id).join(format!("{id}.json"));
            if let Ok(data) = read_json(&path) {
                if valid_version(&data, &id) {
                    return Ok((id, data));
                }
            }
        }
        let manifest = self.fetch_manifest()?;
        if id == "latest" || id == "latest-snapshot" {
            let kind = if id == "latest" {
                "release"
            } else {
                "snapshot"
            };
            id = manifest["latest"][kind]
                .as_str()
                .ok_or("Latest version missing from manifest")?
                .into();
        }
        util::validate_component(&id)?;
        let entry = manifest["versions"]
            .as_array()
            .and_then(|versions| versions.iter().find(|v| v["id"].as_str() == Some(&id)))
            .ok_or_else(|| AppError::Generic(format!("Minecraft version '{id}' not found")))?;
        let path = self.versions_dir.join(&id).join(format!("{id}.json"));
        http::download_file(
            entry["url"].as_str().ok_or("Version metadata has no URL")?,
            &path,
            &id,
            entry["sha1"].as_str(),
            3,
            true,
        )
        .map_err(AppError::Http)?;
        let data = read_json(&path)?;
        if !valid_version(&data, &id) {
            return Err(format!("Invalid version metadata for {id}").into());
        }
        Ok((id, data))
    }

    pub fn download_client_jar(&self, id: &str, data: &serde_json::Value) -> AppResult<PathBuf> {
        util::validate_component(id)?;
        let path = self.versions_dir.join(id).join(format!("{id}.jar"));
        let client = &data["downloads"]["client"];
        http::download_file(
            client["url"].as_str().ok_or("Missing client URL")?,
            &path,
            id,
            client["sha1"].as_str(),
            3,
            true,
        )
        .map_err(AppError::Http)?;
        Ok(path)
    }

    /// Missing rules use the caller's default; an explicit rule list starts denied.
    /// All conditions must match and the last matching action wins.
    pub fn rules_allow(rules: &serde_json::Value, default: bool) -> bool {
        Self::rules_allow_with_features(rules, default, &[])
    }

    pub fn rules_allow_with_features(
        rules: &serde_json::Value,
        default: bool,
        features: &[(&str, bool)],
    ) -> bool {
        let Some(rules) = rules.as_array() else {
            return default;
        };
        let mut allowed = false;
        for rule in rules {
            let mut matches = true;
            if let Some(os) = rule.get("os") {
                if let Some(name) = os["name"].as_str() {
                    matches &= name == util::os_name();
                }
                if let Some(arch) = os["arch"].as_str() {
                    matches &= match arch {
                        "amd64" | "x86_64" => cfg!(target_arch = "x86_64"),
                        "aarch64" | "arm64" => cfg!(target_arch = "aarch64"),
                        "x86" | "i386" => cfg!(target_arch = "x86"),
                        _ => arch == util::os_arch(),
                    };
                }
                if let Some(pattern) = os["version"].as_str() {
                    matches &= regex::Regex::new(pattern)
                        .ok()
                        .is_some_and(|re| re.is_match(util::os_version()));
                }
            }
            if let Some(required) = rule.get("features") {
                matches &= required.as_object().is_some_and(|required| {
                    required.iter().all(|(key, value)| {
                        let actual = features
                            .iter()
                            .find(|(name, _)| *name == key)
                            .map(|(_, value)| *value)
                            .unwrap_or(false);
                        value.as_bool() == Some(actual)
                    })
                });
            }
            if matches {
                allowed = rule["action"].as_str() == Some("allow");
            }
        }
        allowed
    }

    /// Resolve the libraries of a version into files to download and the
    /// classpath jars. Pure path computation, so dry runs can skip downloads.
    fn plan_libraries(
        &self,
        data: &serde_json::Value,
    ) -> AppResult<(Vec<LibraryFile>, Vec<PathBuf>)> {
        let mut downloads = Vec::<LibraryFile>::new();
        let mut jars = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for lib in data["libraries"].as_array().into_iter().flatten() {
            if !Self::rules_allow(&lib["rules"], true) {
                continue;
            }
            let name = lib["name"]
                .as_str()
                .ok_or("Library is missing its Maven name")?;
            let relative = util::maven_rel_path(name)?;
            let mut add = |info: &serde_json::Value,
                           fallback: &str,
                           native: bool,
                           classpath: bool|
             -> AppResult<()> {
                let rel = info["path"].as_str().unwrap_or(fallback);
                let path = util::safe_join(&self.libraries_dir, rel)?;
                if classpath && !jars.contains(&path) {
                    jars.push(path.clone());
                }
                if !seen.insert(path.clone()) {
                    return Ok(());
                }
                let base = lib["url"]
                    .as_str()
                    .unwrap_or("https://libraries.minecraft.net/");
                let url = info["url"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .unwrap_or_else(|| format!("{}/{rel}", base.trim_end_matches('/')));
                downloads.push(LibraryFile {
                    url,
                    path,
                    hash: info["sha1"].as_str().map(String::from),
                    native,
                });
                Ok(())
            };
            let native_artifact = name
                .split(':')
                .nth(3)
                .is_some_and(|c| c.starts_with("natives-"));
            let artifact = &lib["downloads"]["artifact"];
            if !artifact.is_null() || lib.get("downloads").is_none() {
                add(artifact, &relative, native_artifact, true)?;
            }
            if let Some(classifier) = lib["natives"][util::os_name()].as_str() {
                let bits = if cfg!(target_pointer_width = "64") {
                    "64"
                } else {
                    "32"
                };
                let classifier = classifier.replace("${arch}", bits);
                let info = &lib["downloads"]["classifiers"][&classifier];
                if info.is_null() {
                    return Err(format!("Native classifier {classifier} missing for {name}").into());
                }
                let native_rel = util::maven_rel_path(&format!("{name}:{classifier}"))?;
                add(info, &native_rel, true, false)?;
            }
        }
        Ok((downloads, jars))
    }

    /// Classpath jars for a version without touching the network or disk.
    pub fn library_paths(&self, data: &serde_json::Value) -> AppResult<Vec<PathBuf>> {
        Ok(self.plan_libraries(data)?.1)
    }

    pub fn download_libraries(
        &self,
        data: &serde_json::Value,
        natives_dir: &Path,
        max_workers: usize,
    ) -> AppResult<Vec<PathBuf>> {
        let (downloads, jars) = self.plan_libraries(data)?;
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(max_workers.clamp(1, 64))
            .build()
            .map_err(|e| AppError::Generic(e.to_string()))?;
        pool.install(|| {
            downloads.par_iter().try_for_each(|file| {
                http::download_file(
                    &file.url,
                    &file.path,
                    "Library",
                    file.hash.as_deref(),
                    3,
                    false,
                )
            })
        })
        .map_err(AppError::Http)?;
        for file in downloads.iter().filter(|d| d.native) {
            self.extract_natives(&file.path, natives_dir)?;
        }
        Ok(jars)
    }

    /// Only native file basenames are extracted. No archive paths/symlinks are trusted.
    pub fn extract_natives(&self, jar: &Path, natives_dir: &Path) -> AppResult<()> {
        let mut archive = zip::ZipArchive::new(fs::File::open(jar)?).map_err(|e| {
            AppError::Generic(format!("Invalid native archive {}: {e}", jar.display()))
        })?;
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| AppError::Generic(e.to_string()))?;
            if entry.is_dir() || entry.name().starts_with("META-INF/") {
                continue;
            }
            let enclosed = entry
                .enclosed_name()
                .ok_or("Unsafe path in native archive")?;
            let name = enclosed
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or("Invalid native filename")?
                .to_string();
            util::validate_component(&name)?;
            if !matches!(
                Path::new(&name).extension().and_then(|v| v.to_str()),
                Some("dll" | "so" | "dylib" | "jnilib")
            ) {
                continue;
            }
            if entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err("Native archive contains a symlink".into());
            }
            fs::create_dir_all(natives_dir)?;
            let mut temp = tempfile::NamedTempFile::new_in(natives_dir)?;
            std::io::copy(&mut entry, &mut temp)?;
            temp.as_file().sync_all()?;
            temp.persist(natives_dir.join(name)).map_err(|e| e.error)?;
        }
        Ok(())
    }

    pub fn download_assets(
        &self,
        data: &serde_json::Value,
        max_workers: usize,
    ) -> AppResult<String> {
        let index = &data["assetIndex"];
        if index.is_null() {
            return Ok(data["assets"].as_str().unwrap_or("legacy").into());
        }
        let id = index["id"].as_str().ok_or("Missing asset index ID")?;
        util::validate_component(id)?;
        let path = self.assets_dir.join("indexes").join(format!("{id}.json"));
        http::download_file(
            index["url"].as_str().ok_or("Missing asset index URL")?,
            &path,
            "Asset index",
            index["sha1"].as_str(),
            3,
            true,
        )
        .map_err(AppError::Http)?;
        let index_data = read_json(&path)?;
        let objects = index_data["objects"]
            .as_object()
            .ok_or("Asset index has no objects map")?;
        let mut downloads = std::collections::BTreeMap::new();
        let mut samples: Vec<(u64, String)> = Vec::new();
        for object in objects.values() {
            let hash = object["hash"].as_str().ok_or("Missing asset hash")?;
            if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Invalid asset SHA-1".into());
            }
            let size = object["size"].as_u64().unwrap_or(0);
            // Typical small objects make the cheapest, most representative probe.
            if samples.len() < 2 && (8_000..=262_144).contains(&size) {
                samples.push((size, hash.to_string()));
            }
            let dest = self.assets_dir.join("objects").join(&hash[..2]).join(hash);
            downloads.insert(hash, dest);
        }
        if samples.is_empty() {
            samples.extend(downloads.keys().take(2).map(|hash| (0, hash.to_string())));
        }
        let sample_hashes: Vec<String> = samples.into_iter().map(|(_, hash)| hash).collect();
        // In Auto mode measure both origins once per batch instead of always
        // assuming the mirror is faster; the winner gets the whole asset stage.
        let prefer_mirror = match http::source_mode() {
            http::SourceMode::Official => false,
            http::SourceMode::BmclApi => true,
            http::SourceMode::Auto => http::prefer_mirror_for_assets(&sample_hashes),
        };
        let asset_workers = http::asset_worker_count(max_workers, prefer_mirror);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(asset_workers)
            .build()
            .map_err(|e| AppError::Generic(e.to_string()))?;
        let failed: Vec<(&str, PathBuf, String)> = pool.install(|| {
            downloads
                .par_iter()
                .filter_map(|(hash, dest)| {
                    let url = format!(
                        "https://resources.download.minecraft.net/{}/{}",
                        &hash[..2],
                        hash
                    );
                    http::download_asset(&url, dest, "Asset", Some(hash), 3, prefer_mirror)
                        .err()
                        .map(|error| (*hash, dest.clone(), error))
                })
                .collect()
        });
        if progress::is_cancelled() {
            return Err("Download cancelled".into());
        }
        if !failed.is_empty() {
            // Retry only the stragglers: transient TLS resets are the usual
            // cause when many connections are opened at once, and a serial pass
            // is much cheaper than restarting the whole batch.
            let mut hard = Vec::new();
            for (hash, dest, first) in &failed {
                let url = format!(
                    "https://resources.download.minecraft.net/{}/{}",
                    &hash[..2],
                    hash
                );
                if let Err(second) =
                    http::download_asset(&url, dest, "Asset", Some(hash), 4, prefer_mirror)
                {
                    hard.push(format!("{hash}: {first}; retry: {second}"));
                }
            }
            if !hard.is_empty() {
                return Err(AppError::Generic(format!(
                    "{} asset file(s) could not be downloaded; first error: {}",
                    hard.len(),
                    hard[0]
                )));
            }
        }
        // Older clients need named assets as well as the content-addressed store.
        let virtual_assets = index_data["virtual"].as_bool().unwrap_or(false);
        let resources = index_data["map_to_resources"].as_bool().unwrap_or(false);
        if virtual_assets || resources {
            let version_id = data["id"].as_str().ok_or("Missing version ID")?;
            util::validate_component(version_id)?;
            for (name, object) in objects {
                let hash = object["hash"].as_str().ok_or("Missing asset hash")?;
                let source = downloads.get(hash).ok_or("Missing downloaded asset")?;
                for root in [
                    virtual_assets.then(|| self.assets_dir.join("virtual").join(id)),
                    resources.then(|| self.versions_dir.join(version_id).join("resources")),
                ]
                .into_iter()
                .flatten()
                {
                    let dest = util::safe_join(&root, name)?;
                    if !http::cached_file_valid(&dest, Some(hash)) {
                        util::atomic_write(&dest, &fs::read(source)?)?;
                    }
                }
            }
        }
        Ok(id.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rule_lists_start_denied_and_last_match_wins() {
        assert!(VersionManager::rules_allow(&serde_json::Value::Null, true));
        assert!(!VersionManager::rules_allow(&json!([]), true));
        assert!(!VersionManager::rules_allow(
            &json!([{"action":"allow","os":{"name":"nonexistent"}}]),
            true
        ));
        assert!(!VersionManager::rules_allow(
            &json!([{"action":"allow"},{"action":"disallow"}]),
            true
        ));
        assert!(VersionManager::rules_allow(
            &json!([{"action":"disallow"},{"action":"allow"}]),
            false
        ));
    }

    #[test]
    fn os_and_features_are_both_required() {
        let rules = json!([{"action":"allow","os":{"name":util::os_name()},"features":{"has_custom_resolution":true}}]);
        assert!(!VersionManager::rules_allow(&rules, true));
        assert!(VersionManager::rules_allow_with_features(
            &rules,
            true,
            &[("has_custom_resolution", true)]
        ));
        assert!(VersionManager::rules_allow(
            &json!([{"action":"allow","features":{"is_demo_user":false}}]),
            false
        ));
        assert!(!VersionManager::rules_allow(
            &json!([{"action":"allow","os":{"version":"["}}]),
            true
        ));
    }

    #[test]
    fn installed_version_does_not_need_global_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let vm = VersionManager::new(dir.path());
        let data = json!({"id":"1.21.4","mainClass":"Main","libraries":[],"downloads":{"client":{"url":"https://example.org/client.jar"}}});
        util::atomic_write(
            &vm.versions_dir.join("1.21.4/1.21.4.json"),
            data.to_string().as_bytes(),
        )
        .unwrap();
        assert_eq!(vm.get_version_info(Some("1.21.4")).unwrap().1, data);
        assert!(vm.get_version_info(Some("../escape")).is_err());
    }

    #[test]
    fn native_archives_replace_stale_files() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("native.jar");
        let mut writer = zip::ZipWriter::new(fs::File::create(&jar).unwrap());
        writer
            .start_file("nested/test.so", zip::write::FileOptions::default())
            .unwrap();
        writer.write_all(b"new native").unwrap();
        writer.finish().unwrap();
        let natives = dir.path().join("natives");
        util::atomic_write(&natives.join("test.so"), b"old").unwrap();
        VersionManager::new(dir.path())
            .extract_natives(&jar, &natives)
            .unwrap();
        assert_eq!(fs::read(natives.join("test.so")).unwrap(), b"new native");
    }

    #[test]
    fn invalid_asset_hash_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let vm = VersionManager::new(dir.path());
        util::atomic_write(
            &vm.assets_dir.join("indexes/test.json"),
            br#"{"objects":{"a":{"hash":"x"}}}"#,
        )
        .unwrap();
        assert!(vm
            .download_assets(&json!({"assetIndex":{"id":"test","url":"invalid"}}), 1)
            .is_err());
    }
    #[test]
    fn latest_alias_uses_valid_cached_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let vm = VersionManager::new(dir.path());
        let data = json!({"id":"test","mainClass":"Main","libraries":[],"downloads":{"client":{"url":"invalid"}}});
        util::atomic_write(
            &vm.versions_dir.join("test/test.json"),
            data.to_string().as_bytes(),
        )
        .unwrap();
        let manifest = json!({"latest":{"release":"test","snapshot":"test"},"versions":[{"id":"test","url":"invalid"}]});
        util::atomic_write(
            &vm.game_dir.join("version_manifest_v2.json"),
            manifest.to_string().as_bytes(),
        )
        .unwrap();
        for alias in ["latest", "latest-snapshot"] {
            assert_eq!(vm.get_version_info(Some(alias)).unwrap().0, "test");
        }
    }
}
