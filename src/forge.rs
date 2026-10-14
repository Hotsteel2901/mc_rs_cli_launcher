//! Forge metadata and installer-based installation.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::http;

const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const FORGE_PROMOTIONS: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const FORGE_MAVEN_META: &str =
    "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml";

#[derive(Debug, Deserialize)]
struct Metadata {
    versioning: Versioning,
}

#[derive(Debug, Deserialize)]
struct Versions {
    version: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Versioning {
    versions: Versions,
}

pub struct ForgeManager {
    pub game_dir: PathBuf,
    pub lib_dir: PathBuf,
}

impl ForgeManager {
    pub fn new(game_dir: &Path) -> Self {
        ForgeManager {
            game_dir: game_dir.to_path_buf(),
            lib_dir: game_dir.join("libraries"),
        }
    }

    /// Get available Forge versions for a Minecraft version from Maven metadata.
    /// Tries BMCLAPI mirror first, falls back to official Maven if mirror data is incomplete.
    pub fn get_available_versions(&self, mc_version: &str) -> AppResult<Vec<String>> {
        let prefix = format!("{}-", mc_version);

        // Try BMCLAPI mirror first
        let versions = Self::fetch_versions_from_url(FORGE_MAVEN_META, &prefix, true);
        if let Ok(ref v) = versions {
            if !v.is_empty() {
                return Ok(v.clone());
            }
        }

        // Fall back to official Maven URL (bypass mirror)
        crate::warn_msg!(
            "Forge: no {} versions in BMCLAPI mirror, trying official Maven...",
            mc_version
        );
        Self::fetch_versions_from_url(FORGE_MAVEN_META, &prefix, false)
    }

    fn fetch_versions_from_url(
        url: &str,
        prefix: &str,
        use_mirror: bool,
    ) -> AppResult<Vec<String>> {
        let result = if use_mirror {
            http::http_get(url)
        } else {
            http::http_get_no_mirror(url)
        };

        let (status, body) = result
            .map_err(|e| AppError::Loader(format!("Forge maven metadata fetch failed: {}", e)))?;
        if status != 200 {
            return Err(AppError::Loader(format!(
                "Forge maven metadata fetch failed ({})",
                status
            )));
        }

        let text = String::from_utf8_lossy(&body);

        let metadata: Metadata = quick_xml::de::from_str(&text)
            .map_err(|e| AppError::Xml(format!("Failed to parse Forge maven metadata: {}", e)))?;

        let mut versions: Vec<String> = metadata
            .versioning
            .versions
            .version
            .into_iter()
            .filter(|v| v.starts_with(prefix))
            .map(|v| v[prefix.len()..].to_string())
            .collect();

        // Sort by numeric components
        let num_re = regex::Regex::new(r"(\d+)")?;
        versions.sort_by(|a, b| {
            let nums_a: Vec<u32> = num_re
                .find_iter(a)
                .filter_map(|m| m.as_str().parse().ok())
                .collect();
            let nums_b: Vec<u32> = num_re
                .find_iter(b)
                .filter_map(|m| m.as_str().parse().ok())
                .collect();
            nums_a.cmp(&nums_b)
        });

        Ok(versions)
    }

    /// Get recommended Forge version from promotions.
    pub fn get_recommended_version(&self, mc_version: &str) -> Option<String> {
        let (status, body) = http::http_get(FORGE_PROMOTIONS).unwrap_or_else(|_| (0, Vec::new()));
        if status != 200 {
            return None;
        }
        let data: serde_json::Value = serde_json::from_slice(&body).ok()?;
        let promos = data["promos"].as_object()?;
        promos
            .get(&format!("{}-recommended", mc_version))
            .or_else(|| promos.get(&format!("{}-latest", mc_version)))
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    /// Build the Forge installer URL.
    fn installer_url(&self, mc_version: &str, loader_version: &str) -> String {
        format!(
            "{}/{}-{}/forge-{}-{}-installer.jar",
            FORGE_MAVEN, mc_version, loader_version, mc_version, loader_version
        )
    }

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
                "No Forge loader found for Minecraft {}",
                mc_version
            )));
        }

        let loader_ver = if let Some(lv) = loader_version_id {
            if !versions.contains(&lv.to_string()) {
                return Err(AppError::Loader(format!(
                    "Forge loader version '{}' not found for MC {}",
                    lv, mc_version
                )));
            }
            lv.to_string()
        } else {
            let rec = self.get_recommended_version(mc_version);
            match rec {
                Some(r) if versions.contains(&r) => r,
                _ => versions
                    .last()
                    .ok_or_else(|| AppError::Loader("No Forge versions available".into()))?
                    .clone(),
            }
        };

        let full_ver = format!("{}-{}", mc_version, loader_ver);
        crate::info!(
            "Installing Forge {} for Minecraft {}...",
            loader_ver,
            mc_version
        );
        crate::util::validate_component(&loader_ver)?;

        // ── Download the Forge installer JAR ──
        let installer_url = self.installer_url(mc_version, &loader_ver);
        let installer_path = self
            .lib_dir
            .join("forge")
            .join(format!("forge-{}-installer.jar", full_ver));
        if let Some(parent) = installer_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        http::download_file(
            &installer_url,
            &installer_path,
            &format!("Forge {} installer", loader_ver),
            None,
            3,
            true,
        )
        .map_err(|e| AppError::Loader(format!("Forge installer download failed: {}", e)))?;

        crate::installer::install_profile(
            &self.game_dir,
            &installer_path,
            "forge",
            mc_version,
            java,
        )
    }
}
