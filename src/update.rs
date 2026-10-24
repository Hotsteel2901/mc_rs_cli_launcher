//! Mod update checking and application through Modrinth.
//!
//! Installed jars are identified by SHA-1, so this works for mods that were
//! downloaded by any launcher, not only by us.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::http;
use crate::mod_manager::ModManager;
use crate::modrinth;

#[derive(Clone, Debug)]
pub struct ModUpdate {
    pub file: PathBuf,
    pub filename: String,
    pub enabled: bool,
    pub project: String,
    pub current: String,
    pub latest: String,
    pub download_url: String,
    pub download_name: String,
    pub download_sha1: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct UpdateCheck {
    pub updates: Vec<ModUpdate>,
    pub up_to_date: usize,
    pub unknown: usize,
}

fn string_list(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Check all installed jars against the newest version available for the
/// current Minecraft version and loader.
pub fn check(
    game_dir: &Path,
    mc_version: &str,
    loader: Option<&str>,
    include_disabled: bool,
) -> Result<UpdateCheck, String> {
    let manager = ModManager::new(game_dir);
    let mods_dir = manager.mods_dir(mc_version);
    let mut files: Vec<(PathBuf, bool)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&mods_dir) {
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let enabled = name.ends_with(".jar");
            let disabled = name.ends_with(".jar.disabled");
            if !enabled && !(include_disabled && disabled) {
                continue;
            }
            files.push((entry.path(), enabled));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut result = UpdateCheck::default();
    if files.is_empty() {
        return Ok(result);
    }

    let mut hashes = Vec::new();
    let mut by_hash: HashMap<String, (PathBuf, bool)> = HashMap::new();
    for (path, enabled) in &files {
        let hash = crate::util::sha1_file(path)
            .map_err(|error| format!("Cannot hash {}: {error}", path.display()))?;
        hashes.push(hash.clone());
        by_hash.insert(hash, (path.clone(), *enabled));
    }
    let known = modrinth::lookup_files_by_sha1(&hashes)?;

    let mut titles: HashMap<String, String> = HashMap::new();
    for (hash, (path, enabled)) in &by_hash {
        let Some(current) = known.get(hash) else {
            result.unknown += 1;
            continue;
        };
        let Some(project_id) = current["project_id"].as_str() else {
            result.unknown += 1;
            continue;
        };
        let title = match titles.get(project_id) {
            Some(title) => title.clone(),
            None => {
                let title = modrinth::get_project_result(project_id)
                    .ok()
                    .and_then(|project| {
                        project["title"]
                            .as_str()
                            .map(|title| title.to_string())
                            .or_else(|| project["slug"].as_str().map(|slug| slug.to_string()))
                    })
                    .unwrap_or_else(|| project_id.to_string());
                titles.insert(project_id.to_string(), title.clone());
                title
            }
        };
        let current_loaders = string_list(&current["loaders"]);
        let effective: Vec<String> = match loader {
            Some(loader) => vec![loader.to_string()],
            None => current_loaders.clone(),
        };
        let latest = modrinth::latest_compatible(
            project_id,
            (!effective.is_empty()).then_some(effective.as_slice()),
            mc_version,
        )?;
        match latest {
            Some(latest) => match plan(&latest, current, path.clone(), *enabled, title.clone()) {
                Some(update) => result.updates.push(update),
                None => result.up_to_date += 1,
            },
            None => result.up_to_date += 1,
        }
    }
    result
        .updates
        .sort_by_key(|update| update.project.to_lowercase());
    Ok(result)
}

/// Pure comparison: an update exists when the newest compatible release is
/// newer than the installed one (by publish date, then id).
pub fn plan(
    latest: &serde_json::Value,
    current: &serde_json::Value,
    file: PathBuf,
    enabled: bool,
    title: String,
) -> Option<ModUpdate> {
    let latest_id = latest["id"].as_str()?;
    let current_id = current["id"].as_str()?;
    if latest_id == current_id {
        return None;
    }
    // Never move a mod to a different loader by accident.
    let current_loaders = string_list(&current["loaders"]);
    let latest_loaders = string_list(&latest["loaders"]);
    if !current_loaders.is_empty()
        && !latest_loaders.is_empty()
        && !latest_loaders.iter().any(|candidate| {
            current_loaders
                .iter()
                .any(|current| current.eq_ignore_ascii_case(candidate))
        })
    {
        return None;
    }
    let latest_date = latest["date_published"].as_str().unwrap_or("");
    let current_date = current["date_published"].as_str().unwrap_or("");
    if !latest_date.is_empty() && !current_date.is_empty() && latest_date < current_date {
        // The API returned an older compatible release; do not downgrade.
        return None;
    }
    let primary = modrinth::primary_file(latest)?;
    let url = primary["url"].as_str()?.to_string();
    let filename = primary["filename"].as_str()?.to_string();
    Some(ModUpdate {
        file,
        filename: filename.clone(),
        enabled,
        project: title,
        current: current["version_number"]
            .as_str()
            .unwrap_or(current_id)
            .to_string(),
        latest: latest["version_number"]
            .as_str()
            .unwrap_or(latest_id)
            .to_string(),
        download_url: url,
        download_name: filename,
        download_sha1: primary["hashes"]["sha1"].as_str().map(String::from),
    })
}

/// Download and install an update. Disabled mods stay disabled.
pub fn apply(game_dir: &Path, mc_version: &str, update: &ModUpdate) -> AppResult<PathBuf> {
    let manager = ModManager::new(game_dir);
    let mods_dir = manager.mods_dir(mc_version);
    std::fs::create_dir_all(&mods_dir)?;
    let target = mods_dir.join(&update.download_name);
    if target.exists() && target != update.file {
        return Err(AppError::Generic(format!(
            "Refusing to overwrite existing {}",
            target.display()
        )));
    }
    http::download_file(
        &update.download_url,
        &target,
        &update.download_name,
        update.download_sha1.as_deref(),
        3,
        true,
    )
    .map_err(AppError::Http)?;
    if update.file != target && update.file.exists() {
        std::fs::remove_file(&update.file)?;
    }
    if !update.enabled {
        let disabled = target.with_file_name(format!(
            "{}.disabled",
            target
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default()
        ));
        std::fs::rename(&target, &disabled)?;
        return Ok(disabled);
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(id: &str, number: &str, date: &str, filename: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "project_id": "P",
            "version_number": number,
            "date_published": date,
            "files": [{
                "primary": true,
                "filename": filename,
                "url": format!("https://cdn.example/{filename}"),
                "hashes": {"sha1": "abc", "sha512": "def"}
            }]
        })
    }

    #[test]
    fn newer_releases_plan_an_update() {
        let current = version("v1", "1.0.0", "2024-01-01T00:00:00Z", "mod-1.jar");
        let latest = version("v2", "2.0.0", "2024-02-01T00:00:00Z", "mod-2.jar");
        let update = plan(
            &latest,
            &current,
            PathBuf::from("mod-1.jar"),
            true,
            "Mod".into(),
        )
        .unwrap();
        assert_eq!(update.current, "1.0.0");
        assert_eq!(update.latest, "2.0.0");
        assert_eq!(update.download_name, "mod-2.jar");
        assert_eq!(update.download_sha1.as_deref(), Some("abc"));
    }

    #[test]
    fn same_or_older_releases_do_not_plan_an_update() {
        let current = version("v1", "1.0.0", "2024-02-01T00:00:00Z", "mod-1.jar");
        assert!(plan(&current, &current, PathBuf::new(), true, "Mod".into()).is_none());
        let older = version("v0", "0.9.0", "2024-01-01T00:00:00Z", "mod-0.jar");
        assert!(plan(&older, &current, PathBuf::new(), true, "Mod".into()).is_none());
        let no_file = serde_json::json!({"id": "v2", "files": []});
        assert!(plan(&no_file, &current, PathBuf::new(), true, "Mod".into()).is_none());
    }

    #[test]
    fn updates_never_switch_loaders() {
        let mut current = version("v1", "1.0.0", "2024-01-01T00:00:00Z", "mod-fabric-1.jar");
        current["loaders"] = serde_json::json!(["fabric"]);
        let mut latest = version("v2", "2.0.0", "2024-02-01T00:00:00Z", "mod-neoforge-2.jar");
        latest["loaders"] = serde_json::json!(["neoforge"]);
        assert!(plan(&latest, &current, PathBuf::new(), true, "Mod".into()).is_none());

        latest["loaders"] = serde_json::json!(["fabric", "quilt"]);
        assert!(plan(&latest, &current, PathBuf::new(), true, "Mod".into()).is_some());
    }
}
