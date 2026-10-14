//! NeoForge loader installation.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::http;

const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged/neoforge";
const NEOFORGE_MAVEN_META: &str =
    "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml";

#[derive(Debug, Deserialize)]
struct Metadata {
    versioning: Versioning,
}

#[derive(Debug, Deserialize)]
struct Versioning {
    versions: Versions,
}

#[derive(Debug, Deserialize)]
struct Versions {
    version: Vec<String>,
}

pub struct NeoForgeManager {
    pub game_dir: PathBuf,
    pub lib_dir: PathBuf,
}

impl NeoForgeManager {
    pub fn new(game_dir: &Path) -> Self {
        NeoForgeManager {
            game_dir: game_dir.to_path_buf(),
            lib_dir: game_dir.join("libraries"),
        }
    }

    /// Map MC version to NeoForge Maven version prefix.
    fn neoforge_prefix_for_mc(mc_version: &str) -> Option<String> {
        let parts: Vec<&str> = mc_version.split('.').collect();
        if parts.len() >= 3 && parts[0] == "1" && parts[1] == "20" {
            if parts[2] == "1" {
                // 1.20.1 used net.neoforged:forge, not neoforge
                return None;
            }
            return Some(format!("20.{}", parts[2]));
        }
        if parts.len() >= 2 && parts[0] == "1" {
            if let Ok(minor) = parts[1].parse::<u32>() {
                if minor >= 21 {
                    let patch = parts.get(2).copied().unwrap_or("0");
                    return Some(format!("{}.{}", minor, patch));
                }
            }
        }
        None
    }

    /// Get available NeoForge versions for a Minecraft version.
    pub fn get_available_versions(&self, mc_version: &str) -> AppResult<Vec<String>> {
        let (status, body) = http::http_get(NEOFORGE_MAVEN_META).map_err(|e| {
            AppError::Loader(format!("NeoForge maven metadata fetch failed: {}", e))
        })?;
        if status != 200 {
            return Err(AppError::Loader(format!(
                "NeoForge maven metadata fetch failed ({})",
                status
            )));
        }

        let prefix = NeoForgeManager::neoforge_prefix_for_mc(mc_version).ok_or_else(|| {
            AppError::Loader(format!(
                "Cannot determine NeoForge versions for Minecraft {}. Specify --loader-version.",
                mc_version
            ))
        })?;

        let text = String::from_utf8_lossy(&body);
        let metadata: Metadata = quick_xml::de::from_str(&text).map_err(|e| {
            AppError::Loader(format!("Failed to parse NeoForge maven metadata: {}", e))
        })?;

        let mut versions: Vec<String> = metadata
            .versioning
            .versions
            .version
            .into_iter()
            .filter(|v: &String| {
                if v.starts_with(&prefix) {
                    let rest = &v[prefix.len()..];
                    if prefix.ends_with('.') || prefix.ends_with('-') {
                        rest.chars()
                            .next()
                            .is_some_and(|c: char| c.is_ascii_digit())
                    } else {
                        rest.starts_with('.')
                            && rest.len() > 1
                            && rest
                                .chars()
                                .nth(1)
                                .is_some_and(|c: char| c.is_ascii_digit())
                    }
                } else {
                    false
                }
            })
            .collect();

        versions.sort_by(|a, b| {
            let nums_a: Vec<u32> = regex::Regex::new(r"(\d+)")
                .unwrap()
                .find_iter(a)
                .filter_map(|m| m.as_str().parse().ok())
                .collect();
            let nums_b: Vec<u32> = regex::Regex::new(r"(\d+)")
                .unwrap()
                .find_iter(b)
                .filter_map(|m| m.as_str().parse().ok())
                .collect();
            nums_a.cmp(&nums_b)
        });

        Ok(versions)
    }

    fn installer_url(&self, loader_version: &str) -> String {
        format!(
            "{}/{}/neoforge-{}-installer.jar",
            NEOFORGE_MAVEN, loader_version, loader_version
        )
    }

    /// Install NeoForge loader.
    pub fn install(
        &self,
        mc_version: &str,
        loader_version_id: Option<&str>,
        java: Option<&str>,
    ) -> AppResult<(String, serde_json::Value)> {
        crate::util::validate_component(mc_version)?;
        let versions = self.get_available_versions(mc_version)?;
        if versions.is_empty() {
            return Err(AppError::Loader(format!(
                "No NeoForge loader found for Minecraft {}",
                mc_version
            )));
        }

        let loader_ver = if let Some(lv) = loader_version_id {
            if !versions.contains(&lv.to_string()) {
                return Err(AppError::Loader(format!(
                    "NeoForge loader version '{}' not found.",
                    lv
                )));
            }
            lv.to_string()
        } else {
            versions.last().unwrap().clone()
        };

        crate::info!(
            "Installing NeoForge {} for Minecraft {}...",
            loader_ver,
            mc_version
        );
        crate::util::validate_component(&loader_ver)?;

        let installer_url = self.installer_url(&loader_ver);
        let installer_path = self
            .lib_dir
            .join("neoforge")
            .join(format!("neoforge-{}-installer.jar", loader_ver));
        if let Some(parent) = installer_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        http::download_file(
            &installer_url,
            &installer_path,
            &format!("NeoForge {} installer", loader_ver),
            None,
            3,
            true,
        )
        .map_err(|e| AppError::Loader(format!("NeoForge installer download failed: {}", e)))?;

        crate::installer::install_profile(
            &self.game_dir,
            &installer_path,
            "neoforge",
            mc_version,
            java,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_neoforge_prefix_for_mc_1_21_4() {
        let prefix = NeoForgeManager::neoforge_prefix_for_mc("1.21.4");
        assert_eq!(prefix, Some("21.4".to_string()));
    }

    #[test]
    fn test_neoforge_prefix_for_mc_1_21() {
        let prefix = NeoForgeManager::neoforge_prefix_for_mc("1.21");
        assert_eq!(prefix, Some("21.0".to_string()));
    }

    #[test]
    fn test_neoforge_prefix_for_mc_1_20_4() {
        let prefix = NeoForgeManager::neoforge_prefix_for_mc("1.20.4");
        assert_eq!(prefix, Some("20.4".to_string()));
    }

    #[test]
    fn test_neoforge_prefix_for_mc_1_20_1() {
        // 1.20.1 uses net.neoforged:forge, not neoforge
        let prefix = NeoForgeManager::neoforge_prefix_for_mc("1.20.1");
        assert_eq!(prefix, None);
    }

    #[test]
    fn test_neoforge_prefix_for_mc_1_19_4() {
        // Below 1.20, not supported
        let prefix = NeoForgeManager::neoforge_prefix_for_mc("1.19.4");
        assert_eq!(prefix, None);
    }
}
