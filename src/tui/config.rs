//! Persisted TUI preferences (`<game_dir>/launcher_tui.json`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::http::SourceMode;

pub const CONFIG_FILE: &str = "launcher_tui.json";
pub const DEFAULT_RAM_MB: u32 = 4096;
pub const DEFAULT_THREADS: usize = 16;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub ram_mb: u32,
    /// 0 means "let Minecraft choose".
    pub width: u32,
    pub height: u32,
    pub threads: usize,
    /// Empty means auto-detect.
    pub java: String,
    /// "en" (default) or "zh"; TUI-only.
    pub language: String,
    /// Explicit launch loader: "fabric", "forge", "neoforge" or None for auto.
    pub active_loader: Option<String>,
    /// "auto", "official" or "bmcl".
    pub source: String,
    pub reduced_motion: bool,
    pub splash: bool,
    /// Extra JVM arguments, split like a shell command line at launch.
    pub jvm_args: String,
    /// "auto", "g1", "zgc", "parallel", "serial" or "none".
    pub gc: String,
    pub active_version: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ram_mb: DEFAULT_RAM_MB,
            width: 0,
            height: 0,
            threads: DEFAULT_THREADS,
            java: String::new(),
            language: "en".into(),
            active_loader: None,
            source: "auto".into(),
            reduced_motion: false,
            splash: true,
            jvm_args: String::new(),
            gc: "auto".into(),
            active_version: None,
        }
    }
}

impl Settings {
    pub fn path(game_dir: &Path) -> PathBuf {
        game_dir.join(CONFIG_FILE)
    }

    /// Loads settings. A missing file is not an error; a malformed file yields
    /// a readable warning and defaults (the file is left untouched until save).
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Settings>(&bytes) {
                Ok(mut settings) => {
                    settings.normalize();
                    (settings, None)
                }
                Err(error) => (
                    Settings::default(),
                    Some(format!(
                        "Ignoring invalid settings in {}: {error}",
                        path.display()
                    )),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (Settings::default(), None)
            }
            Err(error) => (
                Settings::default(),
                Some(format!("Cannot read {}: {error}", path.display())),
            ),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        crate::util::atomic_write(path, &bytes).map_err(|e| format!("Cannot save settings: {e}"))
    }

    pub fn normalize(&mut self) {
        self.ram_mb = self
            .ram_mb
            .clamp(crate::util::MIN_RAM_MB, crate::util::MAX_RAM_MB);
        self.threads = self.threads.clamp(1, 64);
        self.width = match self.width {
            0 => 0,
            value => value.clamp(320, 7680),
        };
        self.height = match self.height {
            0 => 0,
            value => value.clamp(240, 4320),
        };
        self.java = self
            .java
            .chars()
            .filter(|ch| !ch.is_control())
            .collect::<String>()
            .trim()
            .to_string();
        self.language = match self.language.trim().to_ascii_lowercase().as_str() {
            "zh" | "zh-cn" | "zh_cn" | "cn" | "chinese" | "中文" => "zh".into(),
            _ => "en".into(),
        };
        self.active_loader = self
            .active_loader
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| matches!(value.as_str(), "fabric" | "forge" | "neoforge"));
        self.source = match self.source.to_ascii_lowercase().as_str() {
            "official" | "mojang" => "official".into(),
            "bmcl" | "bmclapi" | "mirror" => "bmcl".into(),
            _ => "auto".into(),
        };
        self.jvm_args = self
            .jvm_args
            .chars()
            .filter(|ch| !ch.is_control())
            .collect::<String>()
            .trim()
            .chars()
            .take(1024)
            .collect();
        self.gc = match self.gc.trim().to_ascii_lowercase().as_str() {
            "g1" | "aikar" => "g1",
            "zgc" => "zgc",
            "parallel" => "parallel",
            "serial" | "serialgc" => "serial",
            "none" | "off" => "none",
            _ => "auto",
        }
        .into();
        if self
            .active_version
            .as_deref()
            .is_some_and(|v| crate::util::validate_component(v).is_err())
        {
            self.active_version = None;
        }
    }

    pub fn gc_preset(&self) -> crate::launcher::GcPreset {
        crate::launcher::GcPreset::parse(&self.gc).unwrap_or_default()
    }

    pub fn jvm_arguments(&self) -> Vec<String> {
        crate::split_args(&self.jvm_args).unwrap_or_default()
    }

    pub fn source_mode(&self) -> SourceMode {
        match self.source.as_str() {
            "official" => SourceMode::Official,
            "bmcl" => SourceMode::BmclApi,
            _ => SourceMode::Auto,
        }
    }

    pub fn set_source_mode(&mut self, mode: SourceMode) {
        self.source = match mode {
            SourceMode::Official => "official",
            SourceMode::BmclApi => "bmcl",
            SourceMode::Auto => "auto",
        }
        .into();
    }

    pub fn window(&self) -> (Option<u32>, Option<u32>) {
        (
            (self.width > 0).then_some(self.width),
            (self.height > 0).then_some(self.height),
        )
    }

    pub fn ram_label(&self) -> String {
        let gb = self.ram_mb as f64 / 1024.0;
        if (gb - gb.round()).abs() < f64::EPSILON {
            format!("{} GB", gb.round() as u32)
        } else {
            format!("{gb:.1} GB")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_clamps_hostile_values() {
        assert_eq!(Settings::default().language, "en");
        let mut settings = Settings {
            ram_mb: 1,
            threads: 999,
            width: 1,
            height: 999_999,
            source: "WEIRD".into(),
            language: "中文".into(),
            active_loader: Some("../escape".into()),
            active_version: Some("../escape".into()),
            ..Settings::default()
        };
        settings.normalize();
        assert_eq!(settings.ram_mb, crate::util::MIN_RAM_MB);
        assert_eq!(settings.threads, 64);
        assert_eq!(settings.width, 320);
        assert_eq!(settings.height, 4320);
        assert_eq!(settings.source, "auto");
        assert_eq!(settings.language, "zh");
        assert_eq!(settings.active_loader, None);
        assert_eq!(settings.active_version, None);
    }

    #[test]
    fn config_round_trips_and_ignores_missing_optional_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = Settings::path(dir.path());
        let mut settings = Settings {
            ram_mb: 6144,
            threads: 8,
            active_loader: Some("FORGE".into()),
            source: "bmcl".into(),
            language: "zh-CN".into(),
            width: 1280,
            height: 720,
            ..Settings::default()
        };
        settings.normalize();
        settings.save(&path).unwrap();
        let (loaded, warning) = Settings::load(&path);
        assert!(warning.is_none());
        assert_eq!(loaded, settings);
        assert_eq!(loaded.source_mode(), SourceMode::BmclApi);
        assert_eq!(loaded.language, "zh");
        assert_eq!(loaded.active_loader.as_deref(), Some("forge"));

        std::fs::write(&path, br#"{"ram_mb":3072}"#).unwrap();
        let (loaded, warning) = Settings::load(&path);
        assert!(warning.is_none());
        assert_eq!(loaded.ram_mb, 3072);
        assert_eq!(loaded.threads, DEFAULT_THREADS);
    }

    #[test]
    fn malformed_config_is_reported_but_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = Settings::path(dir.path());
        std::fs::write(&path, b"not json").unwrap();
        let (settings, warning) = Settings::load(&path);
        assert!(warning.is_some());
        assert_eq!(settings, Settings::default());
        assert_eq!(std::fs::read(&path).unwrap(), b"not json");
    }
}
