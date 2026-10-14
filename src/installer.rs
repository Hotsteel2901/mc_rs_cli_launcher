//! Shared Forge-family installer execution. Modern loaders require processor execution,
//! not merely extraction of the installer JAR.
use crate::error::{AppError, AppResult};
use crate::{java, util, version::VersionManager};
use std::path::Path;
use std::process::Command;

pub fn install_profile(
    game_dir: &Path,
    installer: &Path,
    loader: &str,
    mc_version: &str,
    explicit_java: Option<&str>,
) -> AppResult<(String, serde_json::Value)> {
    util::validate_component(mc_version)?;
    let game_dir = util::absolute_path(game_dir)?;
    let installer = util::absolute_path(installer)?;
    let vm = VersionManager::new(&game_dir);
    let (_, base) = vm.get_version_info(Some(mc_version))?;
    vm.download_client_jar(mc_version, &base)?;
    let profiles = game_dir.join("launcher_profiles.json");
    if !profiles.exists() {
        util::atomic_write(&profiles, b"{\"profiles\":{}}")?;
    }
    let required_java = base["javaVersion"]["majorVersion"]
        .as_u64()
        .map(|n| n as u32);
    let java = if let Some(path) = explicit_java {
        path.to_string()
    } else {
        java::check_java(required_java).ok_or_else(|| {
            AppError::Loader(format!(
                "Install Java {} or specify --java to run the {loader} installer",
                required_java.unwrap_or(17)
            ))
        })?
    };
    // Read the exact target ID; never pick an unrelated older installed version.
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&installer)?)
        .map_err(|e| AppError::Loader(e.to_string()))?;
    let expected: serde_json::Value = {
        let entry = archive.by_name("version.json").map_err(|_| {
            AppError::Loader(
                "This legacy installer has no version.json and is not supported".into(),
            )
        })?;
        serde_json::from_reader(entry)?
    };
    let id = expected["id"]
        .as_str()
        .ok_or("Installer version metadata has no ID")?;
    util::validate_component(id)?;
    if expected["inheritsFrom"].as_str() != Some(mc_version) {
        return Err("Installer does not target the requested Minecraft version".into());
    }
    drop(archive);
    crate::info!("Running {loader} installer and its processors; this may take several minutes...");
    let status = Command::new(java)
        .arg("-jar")
        .arg(&installer)
        .arg("--installClient")
        .arg(&game_dir)
        .current_dir(&game_dir)
        .status()?;
    if !status.success() {
        return Err(AppError::Loader(format!(
            "{loader} installer failed: {status}"
        )));
    }
    let path = vm.versions_dir.join(id).join(format!("{id}.json"));
    let installed: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    let profile = loader_profile(&installed, loader, mc_version, id)?;
    let profile_path = vm
        .libraries_dir
        .join(loader)
        .join(format!("{loader}-profile-{mc_version}.json"));
    util::atomic_write(&profile_path, &serde_json::to_vec_pretty(&profile)?)?;
    Ok((id.into(), profile))
}

fn loader_profile(
    data: &serde_json::Value,
    loader: &str,
    mc: &str,
    id: &str,
) -> AppResult<serde_json::Value> {
    if data["id"].as_str() != Some(id)
        || data["inheritsFrom"].as_str() != Some(mc)
        || !data["mainClass"].is_string()
        || !data["libraries"].is_array()
    {
        return Err("Installed loader profile does not match the requested version".into());
    }
    // Launcher already adds vanilla arguments/libraries. Keep ONLY the child delta.
    Ok(serde_json::json!({
        "profile_format": 1, "loader":loader,"mc_version":mc,"version_id":id,
        "mainClass":data["mainClass"],"libraries":data["libraries"],"arguments":data["arguments"],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_keeps_only_loader_arguments_and_checks_parent() {
        let data = serde_json::json!({"id":"loader-1","inheritsFrom":"1.21.4","mainClass":"LoaderMain","libraries":[],"arguments":{"game":["--loaderVersion","1"]}});
        let profile = loader_profile(&data, "forge", "1.21.4", "loader-1").unwrap();
        assert_eq!(profile["arguments"]["game"], data["arguments"]["game"]);
        assert!(loader_profile(&data, "forge", "1.20.1", "loader-1").is_err());
    }
}
