//! Instance (version) lifecycle, Modrinth modpack archives and world backups.
//!
//! An "instance" is a directory under versions/. It may be a plain Mojang
//! version, or a renamed/copied/modpack version whose id differs from the
//! Minecraft version it is based on. The base version is recorded in a small
//! instance.json marker so loader profiles (which are keyed by Minecraft
//! version) can still be found.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::error::{AppError, AppResult};
use crate::http;
use crate::util;

pub const LOADERS: [&str; 3] = ["fabric", "forge", "neoforge"];
pub const MARKER_FILE: &str = "instance.json";

/// Optional metadata stored inside an instance directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstanceMarker {
    /// The Minecraft version the instance is based on.
    pub base: Option<String>,
    /// Detected or preferred loader.
    pub loader: Option<String>,
    pub loader_version: Option<String>,
    /// Human readable name (modpack name, etc).
    pub name: Option<String>,
    /// Where the instance came from, for example "modrinth".
    pub pack: Option<String>,
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .filter(|v| !v.is_empty())
}

impl InstanceMarker {
    pub fn read(instance_dir: &Path) -> Option<Self> {
        let bytes = fs::read(instance_dir.join(MARKER_FILE)).ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        Some(Self {
            base: string_field(&value, "base"),
            loader: string_field(&value, "loader"),
            loader_version: string_field(&value, "loader_version"),
            name: string_field(&value, "name"),
            pack: string_field(&value, "pack"),
        })
    }

    pub fn write(&self, instance_dir: &Path) -> AppResult<()> {
        fs::create_dir_all(instance_dir)?;
        let value = serde_json::json!({
            "base": self.base,
            "loader": self.loader,
            "loader_version": self.loader_version,
            "name": self.name,
            "pack": self.pack,
        });
        util::atomic_write(
            &instance_dir.join(MARKER_FILE),
            &serde_json::to_vec_pretty(&value)?,
        )?;
        Ok(())
    }
}

/// Read the marker of an instance, if it has one.
pub fn marker_for(game_dir: &Path, id: &str) -> Option<InstanceMarker> {
    util::validate_component(id).ok()?;
    InstanceMarker::read(&game_dir.join("versions").join(id))
}

/// Candidate ids to consult, most specific first: the instance id itself and
/// then the Minecraft version it is based on.
pub fn base_candidates(game_dir: &Path, id: &str) -> Vec<String> {
    let mut ids = vec![id.to_string()];
    if let Some(base) = marker_for(game_dir, id)
        .and_then(|marker| marker.base)
        .filter(|base| base != id)
    {
        ids.push(base);
    }
    ids
}

/// The Minecraft version an instance is based on.
pub fn instance_base(game_dir: &Path, id: &str) -> String {
    marker_for(game_dir, id)
        .and_then(|marker| marker.base)
        .filter(|base| base != id)
        .unwrap_or_else(|| id.to_string())
}

/// Path of a loader profile for an instance, checking the instance id and its
/// base version.
pub fn loader_profile_path(game_dir: &Path, loader: &str, id: &str) -> Option<PathBuf> {
    base_candidates(game_dir, id)
        .into_iter()
        .find_map(|candidate| {
            let path = game_dir
                .join("libraries")
                .join(loader)
                .join(format!("{loader}-profile-{candidate}.json"));
            path.exists().then_some(path)
        })
}

/// Turn a pack name into a safe instance id.
pub fn sanitize_instance_id(name: &str) -> String {
    let mut out: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '+') {
                c
            } else {
                '-'
            }
        })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out = out.trim_matches(['-', '.', ' ']).to_string();
    if out.is_empty() {
        out = "modpack".into();
    }
    if out.chars().count() > 64 {
        out = out.chars().take(64).collect();
    }
    if util::validate_component(&out).is_err() {
        out = format!("pack-{out}");
        if util::validate_component(&out).is_err() {
            out = "modpack".into();
        }
    }
    out
}

#[derive(Clone, Debug, Default)]
pub struct InstanceDetail {
    pub id: String,
    /// Minecraft version this instance is based on.
    pub base: String,
    pub java_major: Option<u32>,
    pub jar_size: u64,
    pub total_size: u64,
    pub saves_size: u64,
    pub loaders: Vec<String>,
    pub mods_enabled: usize,
    pub mods_disabled: usize,
    pub last_played: Option<SystemTime>,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RemoveReport {
    pub removed: Vec<String>,
    pub kept: Vec<String>,
    pub freed_bytes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ModpackExport {
    pub path: PathBuf,
    pub files: usize,
    pub overrides: usize,
    pub offline: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ModpackImport {
    pub id: String,
    pub base: String,
    pub loader: Option<String>,
    pub loader_version: Option<String>,
    pub files: usize,
    pub overrides: usize,
}

pub struct InstanceManager {
    pub game_dir: PathBuf,
}

impl InstanceManager {
    pub fn new(game_dir: &Path) -> Self {
        Self {
            game_dir: game_dir.to_path_buf(),
        }
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.game_dir.join("versions")
    }

    pub fn instance_dir(&self, id: &str) -> AppResult<PathBuf> {
        util::validate_component(id)?;
        Ok(self.versions_dir().join(id))
    }

    pub fn base_of(&self, id: &str) -> String {
        instance_base(&self.game_dir, id)
    }

    /// Loader names that have a profile for this instance.
    pub fn loaders(&self, id: &str) -> Vec<String> {
        LOADERS
            .iter()
            .filter(|loader| loader_profile_path(&self.game_dir, loader, id).is_some())
            .map(|loader| loader.to_string())
            .collect()
    }

    /// Loader version recorded by the loader profile, for modpack manifests.
    pub fn loader_version(&self, id: &str, loader: &str) -> Option<String> {
        let path = loader_profile_path(&self.game_dir, loader, id)?;
        let profile: serde_json::Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        let base = self.base_of(id);
        match loader {
            "fabric" => profile["id"]
                .as_str()?
                .strip_prefix("fabric-loader-")?
                .strip_suffix(&format!("-{base}"))
                .map(|value| value.to_string()),
            _ => {
                let version_id = profile["version_id"].as_str()?;
                let trimmed = version_id
                    .strip_prefix(&format!("{base}-"))
                    .unwrap_or(version_id);
                Some(
                    trimmed
                        .strip_prefix("neoforge-")
                        .unwrap_or(trimmed)
                        .to_string(),
                )
            }
        }
    }

    pub fn detail(&self, id: &str) -> AppResult<InstanceDetail> {
        let dir = self.instance_dir(id)?;
        let json_path = dir.join(format!("{id}.json"));
        let data: serde_json::Value = serde_json::from_slice(&fs::read(&json_path)?)?;
        let mut detail = InstanceDetail {
            id: id.to_string(),
            base: self.base_of(id),
            java_major: data["javaVersion"]["majorVersion"]
                .as_u64()
                .map(|value| value as u32),
            ..Default::default()
        };
        detail.jar_size = dir
            .join(format!("{id}.jar"))
            .metadata()
            .map(|meta| meta.len())
            .unwrap_or(0);
        // One pass for both numbers: the old code walked the whole instance
        // and then walked saves/ again, which is the largest subtree.
        let (total, saves) = detail_sizes(&dir, &dir.join("saves"));
        detail.total_size = total;
        detail.saves_size = saves;
        detail.loaders = self.loaders(id);
        detail.name = marker_for(&self.game_dir, id).and_then(|marker| marker.name);
        detail.last_played = dir
            .join("logs")
            .join("latest.log")
            .metadata()
            .and_then(|meta| meta.modified())
            .ok();
        if let Ok(entries) = fs::read_dir(dir.join("mods")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".jar.disabled") {
                    detail.mods_disabled += 1;
                } else if name.ends_with(".jar") {
                    detail.mods_enabled += 1;
                }
            }
        }
        Ok(detail)
    }

    pub fn list(&self) -> Vec<InstanceDetail> {
        crate::mod_manager::ModManager::list_installed_versions(&self.game_dir)
            .iter()
            .filter_map(|id| self.detail(id).ok())
            .collect()
    }

    /// Delete an instance. Loader profiles and natives are shared elsewhere and
    /// are cleaned up too; saves and mods can be preserved in place.
    /// Whether another installed instance inherits its loader profile from
    /// `profile_id`. Deleting or renaming that profile would break them.
    fn profile_shared_by_others(&self, profile_id: &str, excluding: &[&str]) -> bool {
        let Ok(entries) = fs::read_dir(self.versions_dir()) else {
            return false;
        };
        for entry in entries.flatten() {
            let other = entry.file_name().to_string_lossy().to_string();
            if excluding.contains(&other.as_str())
                || !entry.path().join(format!("{other}.json")).is_file()
            {
                continue;
            }
            if instance_base(&self.game_dir, &other) == profile_id {
                return true;
            }
        }
        false
    }

    pub fn remove(&self, id: &str, keep_saves: bool, keep_mods: bool) -> AppResult<RemoveReport> {
        let dir = self.instance_dir(id)?;
        let absolute = util::absolute_path(&dir)?;
        if !absolute.is_dir() {
            return Err(AppError::Generic(format!(
                "Instance '{id}' is not installed"
            )));
        }
        let versions = util::absolute_path(&self.versions_dir())?;
        if absolute.parent() != Some(versions.as_path()) {
            return Err("Refusing to delete an instance outside the versions directory".into());
        }

        let mut report = RemoveReport {
            freed_bytes: dir_size(&absolute),
            ..Default::default()
        };
        if !keep_saves && !keep_mods {
            fs::remove_dir_all(&absolute)?;
            report.removed.push(format!("versions/{id}"));
        } else {
            for entry in fs::read_dir(&absolute)?.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let path = entry.path();
                let keep = (keep_saves && name == "saves") || (keep_mods && name == "mods");
                if keep {
                    report.freed_bytes = report.freed_bytes.saturating_sub(dir_size(&path));
                    report.kept.push(name);
                    continue;
                }
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    fs::remove_dir_all(&path)?;
                } else {
                    fs::remove_file(&path)?;
                }
                report.removed.push(name);
            }
        }

        for loader in LOADERS {
            let name = format!("{loader}-profile-{id}.json");
            let profile = self.game_dir.join("libraries").join(loader).join(&name);
            if !profile.exists() {
                continue;
            }
            if self.profile_shared_by_others(id, &[id]) {
                // Other instances inherit this profile through their base
                // version; deleting it would make them launch as vanilla.
                crate::warn_msg!(
                    "Keeping libraries/{loader}/{name}: other instances still use it."
                );
                report.kept.push(format!("libraries/{loader}/{name}"));
                continue;
            }
            fs::remove_file(&profile)?;
            report.removed.push(format!("libraries/{loader}/{name}"));
        }
        let natives = self.game_dir.join("natives").join(id);
        if natives.exists() {
            fs::remove_dir_all(&natives)?;
            report.removed.push(format!("natives/{id}"));
        }
        Ok(report)
    }

    pub fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        util::validate_component(to)?;
        if from == to {
            return Ok(());
        }
        let src = self.instance_dir(from)?;
        let dst = self.instance_dir(to)?;
        if !src.join(format!("{from}.json")).is_file() {
            return Err(AppError::Generic(format!(
                "Instance '{from}' is not installed"
            )));
        }
        if dst.exists() {
            return Err(AppError::Generic(format!("Instance '{to}' already exists")));
        }
        fs::rename(&src, &dst)?;
        self.rekey(&dst, from, to)?;

        let natives_src = self.game_dir.join("natives").join(from);
        let natives_dst = self.game_dir.join("natives").join(to);
        if natives_src.exists() && !natives_dst.exists() {
            fs::rename(&natives_src, &natives_dst)?;
        }
        for loader in LOADERS {
            let profile_src = self
                .game_dir
                .join("libraries")
                .join(loader)
                .join(format!("{loader}-profile-{from}.json"));
            let profile_dst = self
                .game_dir
                .join("libraries")
                .join(loader)
                .join(format!("{loader}-profile-{to}.json"));
            if profile_src.exists() && !profile_dst.exists() {
                if self.profile_shared_by_others(from, &[from, to]) {
                    // Keep the shared profile in place: the renamed instance
                    // still resolves it through its base version.
                    crate::warn_msg!(
                        "Keeping libraries/{loader}/{loader}-profile-{from}.json: other instances still use it."
                    );
                } else {
                    fs::rename(&profile_src, &profile_dst)?;
                    patch_profile(&profile_dst, from, to)?;
                }
            }
        }
        Ok(())
    }

    pub fn copy(&self, from: &str, to: &str) -> AppResult<u64> {
        util::validate_component(to)?;
        if from == to {
            return Err(AppError::Generic(
                "Source and destination are the same".into(),
            ));
        }
        let src = self.instance_dir(from)?;
        let dst = self.instance_dir(to)?;
        if !src.join(format!("{from}.json")).is_file() {
            return Err(AppError::Generic(format!(
                "Instance '{from}' is not installed"
            )));
        }
        if dst.exists() {
            return Err(AppError::Generic(format!("Instance '{to}' already exists")));
        }
        let bytes = copy_dir(&src, &dst)?;
        self.rekey(&dst, from, to)?;
        Ok(bytes)
    }

    /// Rename the version json and jar inside a newly created instance dir.
    fn rekey(&self, dir: &Path, from: &str, to: &str) -> AppResult<()> {
        let from_json = dir.join(format!("{from}.json"));
        let to_json = dir.join(format!("{to}.json"));
        if from_json.exists() {
            fs::rename(&from_json, &to_json)?;
        }
        patch_profile(&to_json, from, to)?;

        let from_jar = dir.join(format!("{from}.jar"));
        let to_jar = dir.join(format!("{to}.jar"));
        if from_jar.exists() && !to_jar.exists() {
            fs::rename(&from_jar, &to_jar)?;
        }

        // Preserve the Minecraft version a renamed/copied instance is based on,
        // so loader profiles and modpack exports keep working.
        let mut marker = InstanceMarker::read(dir).unwrap_or_default();
        if marker.base.is_none() {
            marker.base = Some(from.to_string());
            marker.write(dir)?;
        }
        Ok(())
    }

    /// Export the instance's mods as a Modrinth pack (.mrpack). Mods that
    /// Modrinth recognizes are listed as downloads; everything else is stored
    /// in overrides/mods so the pack still works offline.
    pub fn export_modpack(
        &self,
        id: &str,
        loader: Option<&str>,
        output: &Path,
    ) -> AppResult<ModpackExport> {
        self.export_modpack_with(id, loader, output, |hashes| {
            crate::modrinth::lookup_files_by_sha1(hashes)
        })
    }

    fn export_modpack_with<F>(
        &self,
        id: &str,
        loader: Option<&str>,
        output: &Path,
        lookup: F,
    ) -> AppResult<ModpackExport>
    where
        F: Fn(&[String]) -> Result<HashMap<String, serde_json::Value>, String>,
    {
        let dir = self.instance_dir(id)?;
        if !dir.join(format!("{id}.json")).is_file() {
            return Err(AppError::Generic(format!(
                "Instance '{id}' is not installed"
            )));
        }
        let marker = marker_for(&self.game_dir, id).unwrap_or_default();
        let base = self.base_of(id);
        let loader_name = loader
            .map(|value| value.to_string())
            .or_else(|| marker.loader.clone())
            .or_else(|| self.loaders(id).first().cloned());

        let mut jars: Vec<(PathBuf, bool)> = Vec::new();
        if let Ok(entries) = fs::read_dir(dir.join("mods")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".jar") {
                    jars.push((entry.path(), true));
                } else if name.ends_with(".jar.disabled") {
                    jars.push((entry.path(), false));
                }
            }
        }
        jars.sort_by(|a, b| a.0.cmp(&b.0));

        let mut hashes = Vec::new();
        let mut by_hash: HashMap<String, (PathBuf, bool)> = HashMap::new();
        for (path, enabled) in &jars {
            let hash = util::sha1_file(path)
                .map_err(|e| AppError::Generic(format!("Cannot hash {}: {e}", path.display())))?;
            hashes.push(hash.clone());
            by_hash.insert(hash, (path.clone(), *enabled));
        }
        let mut offline = false;
        let known = if hashes.is_empty() {
            HashMap::new()
        } else {
            match lookup(&hashes) {
                Ok(map) => map,
                Err(error) => {
                    crate::warn_msg!(
                        "Modrinth lookup failed ({error}); exporting files as overrides."
                    );
                    offline = true;
                    HashMap::new()
                }
            }
        };

        let mut files: Vec<serde_json::Value> = Vec::new();
        let mut overrides: Vec<(PathBuf, String)> = Vec::new();
        for (hash, (path, enabled)) in &by_hash {
            let filename = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            if !enabled {
                overrides.push((path.clone(), filename));
                continue;
            }
            let matched = known.get(hash).and_then(|version| {
                version["files"]
                    .as_array()
                    .and_then(|files| {
                        files.iter().find(|file| {
                            file["hashes"]["sha1"]
                                .as_str()
                                .is_some_and(|value| value.eq_ignore_ascii_case(hash))
                        })
                    })
                    .map(|file| (file.clone(), version.clone()))
            });
            match matched {
                Some((file, version)) => {
                    let sha512 = file["hashes"]["sha512"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    let url = file["url"].as_str().unwrap_or_default().to_string();
                    if sha512.is_empty() || url.is_empty() {
                        overrides.push((path.clone(), filename));
                        continue;
                    }
                    files.push(serde_json::json!({
                        "path": format!("mods/{filename}"),
                        "hashes": {"sha1": hash, "sha512": sha512},
                        "env": {"client": "required", "server": "required"},
                        "downloads": [url],
                        "fileSize": file["size"].as_u64().unwrap_or(0),
                        "_project": version["project_id"].clone(),
                    }));
                }
                None => overrides.push((path.clone(), filename)),
            }
        }

        // Deterministic archives: stable order keeps diffs and tests predictable.
        files.sort_by(|a, b| {
            a["path"]
                .as_str()
                .unwrap_or("")
                .cmp(b["path"].as_str().unwrap_or(""))
        });
        overrides.sort_by(|a, b| a.1.cmp(&b.1));

        let key = match loader_name.as_deref() {
            Some("fabric") => Some("fabric-loader"),
            Some("forge") => Some("forge"),
            Some("neoforge") => Some("neoforge"),
            _ => None,
        };
        let mut dependencies = serde_json::Map::new();
        dependencies.insert("minecraft".into(), serde_json::json!(base));
        if let (Some(key), Some(loader_name)) = (key, loader_name.as_deref()) {
            if let Some(version) = marker
                .loader_version
                .clone()
                .or_else(|| self.loader_version(id, loader_name))
            {
                dependencies.insert(key.into(), serde_json::json!(version));
            }
        }
        let pack_name = marker.name.clone().unwrap_or_else(|| id.to_string());
        let index = serde_json::json!({
            "formatVersion": 1,
            "game": "minecraft",
            "versionId": sanitize_instance_id(&pack_name),
            "name": pack_name,
            "summary": format!("Exported from {id} by Hot-RS-Launcher"),
            "files": files,
            "dependencies": dependencies,
        });

        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::File::create(output)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = file_options();
        zip.start_file("modrinth.index.json", options)
            .map_err(zip_error)?;
        zip.write_all(&serde_json::to_vec_pretty(&index)?)?;
        for (path, filename) in &overrides {
            if crate::progress::is_cancelled() {
                return Err("Cancelled".into());
            }
            zip.start_file(format!("overrides/mods/{filename}"), options)
                .map_err(zip_error)?;
            let bytes = fs::read(path)?;
            zip.write_all(&bytes)?;
        }
        zip.finish().map_err(zip_error)?;

        Ok(ModpackExport {
            path: output.to_path_buf(),
            files: files.len(),
            overrides: overrides.len(),
            offline,
        })
    }

    /// Import a Modrinth modpack (.mrpack) as a new instance. The base version
    /// metadata is fetched when it is not installed yet; mod files and
    /// overrides are written into the instance directory.
    pub fn import_modpack(&self, archive: &Path, name: Option<&str>) -> AppResult<ModpackImport> {
        let file = fs::File::open(archive)?;
        let mut zip = zip::ZipArchive::new(file).map_err(zip_error)?;
        let mut index_name = None;
        for i in 0..zip.len() {
            if let Ok(entry) = zip.by_index(i) {
                let entry_name = entry.name().to_string();
                if entry_name.eq_ignore_ascii_case("modrinth.index.json") {
                    index_name = Some(entry_name.clone());
                }
            }
        }
        let index_name = index_name.ok_or_else(|| {
            AppError::Generic(
                "This archive has no modrinth.index.json and is not a Modrinth modpack".into(),
            )
        })?;
        let index: serde_json::Value = {
            let mut entry = zip.by_name(&index_name).map_err(zip_error)?;
            let mut text = String::new();
            entry.read_to_string(&mut text)?;
            serde_json::from_str(&text)?
        };
        if index["formatVersion"].as_u64() != Some(1) {
            return Err(AppError::Generic(
                "Unsupported modpack format (expected formatVersion 1)".into(),
            ));
        }
        let base = index["dependencies"]["minecraft"]
            .as_str()
            .ok_or_else(|| {
                AppError::Generic("Modpack does not declare a Minecraft version".into())
            })?
            .to_string();
        util::validate_component(&base)?;

        let loader = [
            ("fabric-loader", "fabric"),
            ("forge", "forge"),
            ("neoforge", "neoforge"),
        ]
        .iter()
        .find(|(key, _)| index["dependencies"][*key].is_string())
        .map(|(_, loader)| (*loader).to_string());
        let loader_version = loader.as_deref().and_then(|loader| {
            let key = if loader == "fabric" {
                "fabric-loader"
            } else {
                loader
            };
            index["dependencies"][key]
                .as_str()
                .map(|value| value.to_string())
        });

        let pack_name = index["name"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .or_else(|| index["versionId"].as_str())
            .unwrap_or("modpack");
        let id = sanitize_instance_id(name.unwrap_or(pack_name));
        let dir = self.instance_dir(&id)?;
        if dir.exists() {
            return Err(AppError::Generic(format!(
                "Instance '{id}' already exists; choose another name with --name"
            )));
        }
        fs::create_dir_all(&dir)?;
        // Any failure below removes this half-created instance instead of
        // leaving a directory that looks installed but misses its mods.
        let guard = PartialDir::new(&dir);

        // Overrides first so launcher metadata cannot be overwritten by a pack.
        // One bounded pass handles both override roots and rejects zip bombs.
        let overrides =
            util::extract_zip_entries(&mut zip, &dir, util::ZipLimits::default(), |name| {
                ["overrides/", "client-overrides/"]
                    .iter()
                    .find_map(|prefix| name.strip_prefix(prefix).map(str::to_string))
                    .filter(|relative| !relative.is_empty())
            })?;
        drop(zip);

        let base_dir = self.versions_dir().join(&base);
        let base_dir_existed = base_dir.exists();
        let base_json = self.base_version_json(&base)?;
        if !base_dir_existed {
            // Metadata-only helper directory; the instance has its own copy and
            // the base should not appear as a separately installed version.
            let _ = fs::remove_dir_all(&base_dir);
        }
        let mut files = 0;
        for entry in index["files"].as_array().into_iter().flatten() {
            if entry["env"]["client"].as_str() == Some("unsupported") {
                continue;
            }
            let Some(relative) = entry["path"].as_str() else {
                continue;
            };
            let target = util::safe_join(&dir, relative)?;
            if target.exists() {
                // An override already provides this file.
                files += 1;
                continue;
            }
            let Some(url) = entry["downloads"]
                .as_array()
                .and_then(|urls| urls.first())
                .and_then(|url| url.as_str())
            else {
                continue;
            };
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let label = target
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            http::download_file(
                url,
                &target,
                &label,
                entry["hashes"]["sha1"].as_str(),
                3,
                true,
            )
            .map_err(AppError::Http)?;
            files += 1;
        }

        // Publish the instance only after every file is in place: a network
        // failure must not leave something that `list-installed` calls ready.
        let mut instance_json = base_json;
        instance_json["id"] = serde_json::json!(id);
        util::atomic_write(
            &dir.join(format!("{id}.json")),
            &serde_json::to_vec_pretty(&instance_json)?,
        )?;
        InstanceMarker {
            base: Some(base.clone()),
            loader: loader.clone(),
            loader_version: loader_version.clone(),
            name: Some(pack_name.to_string()),
            pack: Some("modrinth".into()),
        }
        .write(&dir)?;
        guard.keep();

        Ok(ModpackImport {
            id,
            base,
            loader,
            loader_version,
            files,
            overrides,
        })
    }

    /// Read the vanilla version json, downloading metadata when needed.
    fn base_version_json(&self, mc: &str) -> AppResult<serde_json::Value> {
        let path = self.versions_dir().join(mc).join(format!("{mc}.json"));
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if value["mainClass"].is_string() && value["libraries"].is_array() {
                    return Ok(value);
                }
            }
        }
        let manager = crate::version::VersionManager::new(&self.game_dir);
        let (_, data) = manager.get_version_info(Some(mc))?;
        Ok(data)
    }

    /// Zip saves, configs and (optionally) mods into backups/.
    pub fn backup(&self, id: &str, with_mods: bool) -> AppResult<PathBuf> {
        let dir = self.instance_dir(id)?;
        if !dir.join(format!("{id}.json")).is_file() {
            return Err(AppError::Generic(format!(
                "Instance '{id}' is not installed"
            )));
        }
        let backups = self.game_dir.join("backups");
        fs::create_dir_all(&backups)?;
        let stamp = utc_stamp(SystemTime::now());
        // Two backups in the same second must not overwrite each other, and a
        // failed write must never replace an existing archive with a fragment.
        let path = util::unique_path(&backups.join(format!("{id}-{stamp}.zip")));
        let temp = tempfile::NamedTempFile::new_in(&backups)?;
        let mut zip = zip::ZipWriter::new(temp.as_file().try_clone()?);
        let options = file_options();
        let manifest = serde_json::json!({
            "id": id,
            "base": self.base_of(id),
            "created_at": SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0),
            "with_mods": with_mods,
        });
        zip.start_file("backup.json", options).map_err(zip_error)?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest)?)?;

        let mut added = 0;
        let version_json = format!("{id}.json");
        for item in [
            "saves",
            "config",
            "options.txt",
            "servers.dat",
            "instance.json",
            version_json.as_str(),
        ] {
            let source = dir.join(item);
            if source.is_dir() {
                added += add_tree(&mut zip, &source, Path::new(item), options)?;
            } else if source.is_file() {
                zip.start_file(item, options).map_err(zip_error)?;
                zip.write_all(&fs::read(&source)?)?;
                added += 1;
            }
        }
        if with_mods {
            let mods = dir.join("mods");
            if mods.is_dir() {
                added += add_tree(&mut zip, &mods, Path::new("mods"), options)?;
            }
        }
        zip.finish().map_err(zip_error)?;
        temp.persist(&path).map_err(|error| {
            AppError::Generic(format!("Cannot write {}: {}", path.display(), error.error))
        })?;
        if added == 0 {
            crate::warn_msg!("Nothing to back up in '{}'.", id);
        }
        Ok(path)
    }

    pub fn restore(&self, archive: &Path, target: Option<&str>) -> AppResult<(String, usize)> {
        let file = fs::File::open(archive)?;
        let mut zip = zip::ZipArchive::new(file).map_err(zip_error)?;
        let mut id = target.map(|value| value.to_string());
        if id.is_none() {
            if let Ok(mut entry) = zip.by_name("backup.json") {
                let mut text = String::new();
                entry.read_to_string(&mut text)?;
                let manifest: serde_json::Value = serde_json::from_str(&text)?;
                id = manifest["id"].as_str().map(|value| value.to_string());
            }
        }
        let id = id.ok_or_else(|| {
            AppError::Generic("Backup does not name an instance; pass --version".into())
        })?;
        let dir = self.instance_dir(&id)?;
        fs::create_dir_all(&dir)?;
        let restored =
            util::extract_zip_entries(&mut zip, &dir, util::ZipLimits::default(), |name| {
                if name == "backup.json" || name.starts_with("__MACOSX/") {
                    None
                } else {
                    Some(name.to_string())
                }
            })?;
        Ok((id, restored))
    }

    pub fn list_backups(&self) -> Vec<(PathBuf, u64, Option<SystemTime>)> {
        let dir = self.game_dir.join("backups");
        let mut entries: Vec<(PathBuf, u64, Option<SystemTime>)> = Vec::new();
        if let Ok(read_dir) = fs::read_dir(&dir) {
            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.ends_with(".zip") {
                    continue;
                }
                if let Ok(meta) = entry.metadata() {
                    entries.push((entry.path(), meta.len(), meta.modified().ok()));
                }
            }
        }
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.2));
        entries
    }
}

fn zip_error(error: zip::result::ZipError) -> AppError {
    AppError::Generic(format!("Archive error: {error}"))
}

/// Deletes a directory that a failed install/modpack import left behind.
struct PartialDir {
    path: PathBuf,
    keep: bool,
}

impl PartialDir {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            keep: false,
        }
    }

    /// Mark the directory as complete so it survives the drop.
    fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for PartialDir {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn file_options() -> zip::write::FileOptions {
    zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644)
}

fn add_tree(
    zip: &mut zip::ZipWriter<fs::File>,
    source: &Path,
    relative: &Path,
    options: zip::write::FileOptions,
) -> AppResult<usize> {
    let mut added = 0;
    for entry in fs::read_dir(source)?.flatten() {
        if crate::progress::is_cancelled() {
            return Err("Cancelled".into());
        }
        let path = entry.path();
        let name = entry.file_name();
        let child = relative.join(&name);
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            added += add_tree(zip, &path, &child, options)?;
        } else if kind.is_file() {
            let entry_name = child.to_string_lossy().replace('\\', "/");
            zip.start_file(entry_name, options).map_err(zip_error)?;
            zip.write_all(&fs::read(&path)?)?;
            added += 1;
        }
    }
    Ok(added)
}

/// Recursive size of regular files; symlinks are not followed.
pub fn dir_size(path: &Path) -> u64 {
    tree_sizes(path, Path::new("")).0
}

/// Iterative walk that also reports how many bytes live under `watch`.
/// Iterative (rather than recursive) so a pathological archive cannot blow
/// the stack, and single-pass so `saves/` is not traversed twice.
fn tree_sizes(root: &Path, watch: &Path) -> (u64, u64) {
    let mut total = 0;
    let mut watched = 0;
    let mut stack = vec![(root.to_path_buf(), false)];
    while let Some((path, inside)) = stack.pop() {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        let inside = inside || (!watch.as_os_str().is_empty() && path == watch);
        if metadata.is_file() {
            total += metadata.len();
            if inside {
                watched += metadata.len();
            }
        } else if metadata.is_dir() {
            if let Ok(entries) = fs::read_dir(&path) {
                for entry in entries.flatten() {
                    stack.push((entry.path(), inside));
                }
            }
        }
    }
    (total, watched)
}

/// Instance sizes are shown in the list and recomputed after every job. A few
/// seconds of caching avoids re-walking gigabytes when the user just installed
/// a mod; the CLI always gets a fresh number.
fn detail_sizes(root: &Path, watch: &Path) -> (u64, u64) {
    if cfg!(test) {
        return tree_sizes(root, watch);
    }
    type SizeCache = std::sync::Mutex<std::collections::HashMap<PathBuf, (Instant, u64, u64)>>;
    static CACHE: std::sync::OnceLock<SizeCache> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some((at, total, saves)) = guard.get(root) {
            if at.elapsed() < Duration::from_secs(5) {
                return (*total, *saves);
            }
        }
    }
    let sizes = tree_sizes(root, watch);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(root.to_path_buf(), (Instant::now(), sizes.0, sizes.1));
    }
    sizes
}

fn copy_dir(src: &Path, dst: &Path) -> AppResult<u64> {
    fs::create_dir_all(dst)?;
    let mut total = 0;
    for entry in fs::read_dir(src)?.flatten() {
        if crate::progress::is_cancelled() {
            return Err("Cancelled".into());
        }
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        let target = dst.join(entry.file_name());
        if kind.is_dir() {
            total += copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            total += entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(total)
}

/// Patch id/inheritsFrom after a rename or copy.
fn patch_profile(path: &Path, from: &str, to: &str) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    let mut changed = false;
    for key in ["id", "inheritsFrom"] {
        if value[key].as_str() == Some(from) {
            value[key] = serde_json::json!(to);
            changed = true;
        }
    }
    if changed {
        util::atomic_write(path, &serde_json::to_vec_pretty(&value)?)?;
    }
    Ok(())
}

/// "20241024-193000" in UTC, without pulling in a date-time crate.
pub(crate) fn utc_stamp(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        seconds_of_day / 3600,
        (seconds_of_day % 3600) / 60,
        seconds_of_day % 60
    )
}

/// Howard Hinnant's days-from-civil inverse.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_instance(root: &Path, id: &str) {
        let dir = root.join("versions").join(id);
        fs::create_dir_all(dir.join("saves").join("world")).unwrap();
        fs::write(dir.join("saves").join("world").join("level.dat"), b"save").unwrap();
        fs::create_dir_all(dir.join("mods")).unwrap();
        fs::write(dir.join("mods").join("a.jar"), b"jar").unwrap();
        fs::write(dir.join("logs"), b"log").ok();
        let json = serde_json::json!({
            "id": id,
            "mainClass": "net.minecraft.client.main.Main",
            "libraries": [],
            "downloads": {"client": {"url": "https://example.invalid/client.jar"}},
            "javaVersion": {"majorVersion": 21}
        });
        fs::write(
            dir.join(format!("{id}.json")),
            serde_json::to_vec_pretty(&json).unwrap(),
        )
        .unwrap();
        fs::write(dir.join(format!("{id}.jar")), b"jarbytes").unwrap();
    }

    fn read_zip(path: &Path) -> Vec<(String, Vec<u8>)> {
        let file = fs::File::open(path).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut entries = Vec::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).unwrap();
            let name = entry.name().to_string();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            entries.push((name, bytes));
        }
        entries
    }

    #[test]
    fn sanitize_instance_ids_are_safe() {
        assert_eq!(
            sanitize_instance_id("Fabulously Optimized"),
            "Fabulously-Optimized"
        );
        assert_eq!(sanitize_instance_id("  a//b  "), "a-b");
        assert_eq!(sanitize_instance_id(""), "modpack");
        assert_eq!(sanitize_instance_id(".."), "modpack");
        assert!(util::validate_component(&sanitize_instance_id("x:y*z")).is_ok());
    }

    #[test]
    fn removal_keeps_saves_when_requested_and_cleans_shared_state() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fake_instance(root, "1.21.4");
        let profile = root
            .join("libraries")
            .join("fabric")
            .join("fabric-profile-1.21.4.json");
        fs::create_dir_all(profile.parent().unwrap()).unwrap();
        fs::write(&profile, b"{\"mainClass\":\"X\"}").unwrap();
        fs::create_dir_all(root.join("natives").join("1.21.4")).unwrap();
        let manager = InstanceManager::new(root);

        let report = manager.remove("1.21.4", true, false).unwrap();
        let dir = root.join("versions").join("1.21.4");
        assert!(dir.join("saves").join("world").join("level.dat").is_file());
        assert!(!dir.join("mods").exists());
        assert!(!dir.join("1.21.4.json").exists());
        assert!(!profile.exists());
        assert!(!root.join("natives").join("1.21.4").exists());
        assert_eq!(report.kept, vec!["saves".to_string()]);
        assert!(report.freed_bytes > 0);

        let report = manager.remove("1.21.4", false, false).unwrap();
        assert!(!dir.exists());
        assert!(report
            .removed
            .iter()
            .any(|entry| entry == "versions/1.21.4"));
        assert!(manager.remove("missing", false, false).is_err());
        assert!(manager.remove("../escape", false, false).is_err());
    }

    #[test]
    fn rename_rekeys_json_jar_and_loader_profile() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fake_instance(root, "1.21.4");
        let profile = root
            .join("libraries")
            .join("fabric")
            .join("fabric-profile-1.21.4.json");
        fs::create_dir_all(profile.parent().unwrap()).unwrap();
        fs::write(
            &profile,
            serde_json::to_vec(&serde_json::json!({"id": "1.21.4", "inheritsFrom": "1.21.4"}))
                .unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("versions").join("1.21.4").join("instance.json"),
            serde_json::to_vec(&serde_json::json!({"base": "1.21.4", "loader": "fabric"})).unwrap(),
        )
        .unwrap();
        let manager = InstanceManager::new(root);

        manager.rename("1.21.4", "my-pack").unwrap();
        let dir = root.join("versions").join("my-pack");
        assert!(dir.join("my-pack.json").is_file());
        assert!(dir.join("my-pack.jar").is_file());
        assert!(!dir.join("1.21.4.json").exists());
        assert!(dir.join("saves").join("world").join("level.dat").is_file());
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("my-pack.json")).unwrap()).unwrap();
        assert_eq!(json["id"], "my-pack");
        let renamed_profile = root
            .join("libraries")
            .join("fabric")
            .join("fabric-profile-my-pack.json");
        let profile_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&renamed_profile).unwrap()).unwrap();
        assert_eq!(profile_json["id"], "my-pack");
        assert_eq!(profile_json["inheritsFrom"], "my-pack");
        assert!(!profile.exists());

        manager.copy("my-pack", "copy-pack").unwrap();
        let copied = root.join("versions").join("copy-pack");
        assert!(copied.join("copy-pack.json").is_file());
        let copied_json: serde_json::Value =
            serde_json::from_slice(&fs::read(copied.join("copy-pack.json")).unwrap()).unwrap();
        assert_eq!(copied_json["id"], "copy-pack");
        assert!(copied
            .join("saves")
            .join("world")
            .join("level.dat")
            .is_file());
        assert!(manager.copy("my-pack", "my-pack").is_err());
        assert!(manager.rename("missing", "x").is_err());
    }

    #[test]
    fn instance_details_report_base_loader_and_sizes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fake_instance(root, "1.21.4");
        let dir = root.join("versions").join("1.21.4");
        fs::write(
            dir.join("instance.json"),
            serde_json::to_vec(
                &serde_json::json!({"base": "1.21.4", "loader": "fabric", "name": "Test Pack"}),
            )
            .unwrap(),
        )
        .unwrap();
        fs::write(dir.join("mods").join("b.jar.disabled"), b"disabled").unwrap();
        let profile = root
            .join("libraries")
            .join("fabric")
            .join("fabric-profile-1.21.4.json");
        fs::create_dir_all(profile.parent().unwrap()).unwrap();
        fs::write(&profile, b"{}").unwrap();

        let manager = InstanceManager::new(root);
        let detail = manager.detail("1.21.4").unwrap();
        assert_eq!(detail.base, "1.21.4");
        assert_eq!(detail.java_major, Some(21));
        assert_eq!(detail.loaders, vec!["fabric".to_string()]);
        assert_eq!(detail.mods_enabled, 1);
        assert_eq!(detail.mods_disabled, 1);
        assert!(detail.total_size > detail.jar_size);
        assert!(detail.saves_size > 0);
        assert_eq!(detail.name.as_deref(), Some("Test Pack"));
        assert_eq!(manager.list().len(), 1);

        // A custom instance id resolves loader profiles through its marker.
        fs::create_dir_all(root.join("versions").join("custom")).unwrap();
        let mut json = serde_json::json!({"id": "custom", "mainClass": "X", "libraries": []});
        json["id"] = serde_json::json!("custom");
        fs::write(
            root.join("versions").join("custom").join("custom.json"),
            serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("versions").join("custom").join("instance.json"),
            serde_json::to_vec(&serde_json::json!({"base": "1.21.4", "loader": "fabric"})).unwrap(),
        )
        .unwrap();
        let detail = manager.detail("custom").unwrap();
        assert_eq!(detail.base, "1.21.4");
        assert_eq!(detail.loaders, vec!["fabric".to_string()]);
        assert!(loader_profile_path(root, "fabric", "custom").is_some());
    }

    #[test]
    fn modpack_export_and_import_roundtrip_without_network() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fake_instance(root, "my-instance");
        let dir = root.join("versions").join("my-instance");
        fs::write(
            dir.join("instance.json"),
            serde_json::to_vec(&serde_json::json!({
                "base": "1.21.4", "loader": "fabric", "loader_version": "0.16.9", "name": "My Pack"
            }))
            .unwrap(),
        )
        .unwrap();
        fs::create_dir_all(dir.join("mods")).unwrap();
        fs::write(dir.join("mods").join("local.jar"), b"local mod").unwrap();
        fs::write(dir.join("mods").join("off.jar.disabled"), b"disabled mod").unwrap();
        // Base version metadata so import does not touch the network.
        let base_dir = root.join("versions").join("1.21.4");
        fs::create_dir_all(&base_dir).unwrap();
        fs::write(
            base_dir.join("1.21.4.json"),
            serde_json::to_vec(&serde_json::json!({
                "id": "1.21.4", "mainClass": "net.minecraft.client.main.Main", "libraries": [],
                "downloads": {"client": {"url": "https://example.invalid/client.jar"}}
            }))
            .unwrap(),
        )
        .unwrap();

        let manager = InstanceManager::new(root);
        let pack = root.join("out").join("pack.mrpack");
        let export = manager
            .export_modpack_with("my-instance", None, &pack, |_| Err("offline".into()))
            .unwrap();
        assert!(export.offline);
        assert_eq!(export.files, 0);
        assert_eq!(export.overrides, 3);
        let entries = read_zip(&pack);
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"modrinth.index.json"));
        assert!(names.contains(&"overrides/mods/local.jar"));
        assert!(names.contains(&"overrides/mods/off.jar.disabled"));
        let index: serde_json::Value = serde_json::from_slice(
            &entries
                .iter()
                .find(|(name, _)| name == "modrinth.index.json")
                .unwrap()
                .1,
        )
        .unwrap();
        assert_eq!(index["dependencies"]["minecraft"], "1.21.4");
        assert_eq!(index["dependencies"]["fabric-loader"], "0.16.9");
        assert_eq!(index["formatVersion"], 1);

        let imported = manager
            .import_modpack(&pack, Some("imported-pack"))
            .unwrap();
        assert_eq!(imported.id, "imported-pack");
        assert_eq!(imported.base, "1.21.4");
        assert_eq!(imported.loader.as_deref(), Some("fabric"));
        assert_eq!(imported.overrides, 3);
        let target = root.join("versions").join("imported-pack");
        assert!(target.join("imported-pack.json").is_file());
        assert!(target.join("mods").join("local.jar").is_file());
        assert!(target.join("mods").join("off.jar.disabled").is_file());
        let marker = InstanceMarker::read(&target).unwrap();
        assert_eq!(marker.base.as_deref(), Some("1.21.4"));
        assert_eq!(marker.loader.as_deref(), Some("fabric"));
        assert_eq!(marker.name.as_deref(), Some("My Pack"));
        assert!(manager
            .import_modpack(&pack, Some("imported-pack"))
            .is_err());
    }

    #[test]
    fn backup_and_restore_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fake_instance(root, "1.21.4");
        let dir = root.join("versions").join("1.21.4");
        fs::create_dir_all(dir.join("config")).unwrap();
        fs::write(dir.join("config").join("mod.toml"), b"cfg").unwrap();
        fs::write(dir.join("options.txt"), b"fov:70").unwrap();
        let manager = InstanceManager::new(root);

        let archive = manager.backup("1.21.4", false).unwrap();
        assert!(archive.is_file());
        assert_eq!(manager.list_backups().len(), 1);

        fs::remove_dir_all(dir.join("saves")).unwrap();
        fs::remove_file(dir.join("options.txt")).unwrap();
        let (id, restored) = manager.restore(&archive, None).unwrap();
        assert_eq!(id, "1.21.4");
        assert!(restored >= 3);
        assert!(dir.join("saves").join("world").join("level.dat").is_file());
        assert_eq!(fs::read(dir.join("options.txt")).unwrap(), b"fov:70");

        let other = manager.restore(&archive, Some("target")).unwrap();
        assert_eq!(other.0, "target");
        assert!(root
            .join("versions")
            .join("target")
            .join("options.txt")
            .is_file());
        assert!(manager.backup("missing", false).is_err());
    }

    #[test]
    fn utc_timestamps_match_known_dates() {
        assert_eq!(utc_stamp(UNIX_EPOCH), "19700101-000000");
        assert_eq!(
            utc_stamp(UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000)),
            "20231114-221320"
        );
    }
}
