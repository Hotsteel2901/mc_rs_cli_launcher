//! Mod management — search, install, list, disable, enable, uninstall mods.

use std::path::{Path, PathBuf};

use crate::modrinth;

/// How missing required mod dependencies are handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DependencyMode {
    /// Prompt on an interactive terminal (CLI default).
    Ask,
    /// Install required dependencies without prompting (TUI / --yes).
    Auto,
    /// Never install dependencies (--no-deps).
    Skip,
}

pub struct ModManager {
    pub game_dir: PathBuf,
}

impl ModManager {
    pub fn new(game_dir: &Path) -> Self {
        ModManager {
            game_dir: game_dir.to_path_buf(),
        }
    }

    /// Get the mods directory for a Minecraft version.
    pub fn mods_dir(&self, mc_version: &str) -> PathBuf {
        crate::util::validate_component(mc_version).unwrap_or_else(|e| crate::die!(e));
        self.game_dir.join("versions").join(mc_version).join("mods")
    }

    /// List installed Minecraft versions.
    pub fn list_installed_versions(game_dir: &Path) -> Vec<String> {
        let versions_dir = game_dir.join("versions");
        if !versions_dir.exists() {
            return Vec::new();
        }
        let mut vers: Vec<String> = std::fs::read_dir(&versions_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                e.path().join(format!("{}.json", name)).exists()
            })
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();

        // Sort by version number
        vers.sort_by(|a, b| {
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
        vers
    }

    /// Get loader support info for a project.
    pub fn loader_support(
        &self,
        project_id: &str,
    ) -> Result<std::collections::BTreeMap<String, (String, String)>, String> {
        let versions = modrinth::get_project_versions_result(project_id, None, None)?;
        Ok(Self::summarize_loader_support(&versions))
    }

    /// Summarize loader support from version data.
    /// Returns map: loader -> (highest MC version, mod version).
    pub fn summarize_loader_support(
        versions: &serde_json::Value,
    ) -> std::collections::BTreeMap<String, (String, String)> {
        let mc_re = regex::Regex::new(r"^\d+\.\d+(\.\d+)?$").unwrap();
        let num_re = regex::Regex::new(r"\d+").unwrap();
        let mut support: std::collections::BTreeMap<String, (String, String)> =
            std::collections::BTreeMap::new();

        let arr = versions.as_array();
        if arr.is_none() {
            return support;
        }

        for v in arr.unwrap() {
            let game_versions: Vec<&str> = v["game_versions"]
                .as_array()
                .map(|gv| {
                    gv.iter()
                        .filter_map(|s| s.as_str())
                        .filter(|s| mc_re.is_match(s))
                        .collect()
                })
                .unwrap_or_default();

            let top = game_versions
                .iter()
                .max_by(|a, b| {
                    let na: Vec<u32> = num_re
                        .find_iter(a)
                        .filter_map(|m| m.as_str().parse().ok())
                        .collect();
                    let nb: Vec<u32> = num_re
                        .find_iter(b)
                        .filter_map(|m| m.as_str().parse().ok())
                        .collect();
                    na.cmp(&nb)
                })
                .map(|s| s.to_string());

            if let Some(top_mc) = top {
                let mod_ver = v["version_number"]
                    .as_str()
                    .unwrap_or(v["id"].as_str().unwrap_or("?"))
                    .to_string();

                for loader in v["loaders"].as_array().into_iter().flatten() {
                    let l = loader.as_str().unwrap_or("").to_lowercase();
                    let cur = support.get(&l);
                    let insert = match cur {
                        Some((cur_mc, _)) => {
                            let na: Vec<u32> = num_re
                                .find_iter(&top_mc)
                                .filter_map(|m| m.as_str().parse().ok())
                                .collect();
                            let nb: Vec<u32> = num_re
                                .find_iter(cur_mc)
                                .filter_map(|m| m.as_str().parse().ok())
                                .collect();
                            na > nb
                        }
                        None => true,
                    };
                    if insert {
                        support.insert(l.clone(), (top_mc.clone(), mod_ver.clone()));
                    }
                }
            }
        }

        support
    }

    /// Format loader support for display.
    pub fn format_loader_support(
        support: &std::collections::BTreeMap<String, (String, String)>,
    ) -> String {
        let loaders = ["fabric", "forge", "neoforge"];
        let parts: Vec<String> = loaders
            .iter()
            .map(|l| {
                if let Some((mc, modver)) = support.get(*l) {
                    format!("{} <= MC {} ({})", l, mc, modver)
                } else {
                    format!("{} \u{2014}", l)
                }
            })
            .collect();
        parts.join("  |  ")
    }

    /// Detect installed loaders for a game version.
    fn detect_loaders(&self, mc_version: &str) -> Vec<String> {
        let marker = crate::instance::marker_for(&self.game_dir, mc_version);
        let mut found = Vec::new();
        if let Some(preferred) = marker.and_then(|marker| marker.loader) {
            if crate::instance::loader_profile_path(&self.game_dir, &preferred, mc_version)
                .is_some()
            {
                found.push(preferred);
            }
        }
        for loader in &["fabric", "neoforge", "forge"] {
            if found.iter().any(|entry| entry == loader) {
                continue;
            }
            if crate::instance::loader_profile_path(&self.game_dir, loader, mc_version).is_some() {
                found.push(loader.to_string());
            }
        }
        found
    }

    /// Pick the appropriate loader.
    fn pick_loader(&self, mc_version: &str, preferred: Option<&str>) -> Option<String> {
        if let Some(p) = preferred {
            return Some(p.to_string());
        }
        let detected = self.detect_loaders(mc_version);
        if detected.is_empty() {
            return None;
        }
        if detected.len() == 1 {
            return Some(detected[0].clone());
        }
        let order = ["fabric", "neoforge", "forge"];
        for loader in &order {
            if detected.contains(&loader.to_string()) {
                crate::info!(
                    "Multiple loaders detected; picked {}. Use --loader to override.",
                    loader
                );
                return Some(loader.to_string());
            }
        }
        Some(detected[0].clone())
    }

    /// Fallible install used by the TUI and batch optimization flow.
    pub fn install_with(
        &self,
        slug: &str,
        mc_version: &str,
        loader: Option<&str>,
        version_id: Option<&str>,
        dependencies: DependencyMode,
    ) -> Result<(Vec<PathBuf>, serde_json::Value, serde_json::Value), String> {
        self.install_inner(
            slug,
            mc_version,
            loader,
            version_id,
            dependencies,
            &mut std::collections::HashSet::new(),
        )
    }

    fn install_inner(
        &self,
        slug: &str,
        mc_version: &str,
        loader: Option<&str>,
        version_id: Option<&str>,
        dependencies: DependencyMode,
        visiting: &mut std::collections::HashSet<String>,
    ) -> Result<(Vec<PathBuf>, serde_json::Value, serde_json::Value), String> {
        crate::info!("Resolving mod: {}...", slug);
        let loader = self.pick_loader(mc_version, loader);
        if let Some(ref l) = loader {
            crate::info!("Using loader: {}", l);
        }

        let project = modrinth::get_project_result(slug)
            .map_err(|error| format!("Cannot resolve mod '{slug}': {error}"))?;
        let project_id = project["id"].as_str().unwrap_or(slug);
        if !visiting.insert(project_id.to_string()) {
            return Err(format!(
                "Cyclic mod dependency detected at {slug}; installation stopped."
            ));
        }

        let versions = modrinth::get_project_versions_result(
            project_id,
            loader.as_ref().map(|l| vec![l.clone()]).as_deref(),
            Some(&[mc_version.to_string()]),
        )
        .map_err(|error| format!("Cannot list versions for {slug}: {error}"))?;

        let proj_title = project["title"].as_str().unwrap_or(slug);

        if versions.as_array().is_none_or(|a| a.is_empty()) {
            let extra = format!(
                " for MC {}{}",
                mc_version,
                loader
                    .as_ref()
                    .map(|l| format!(" ({})", l))
                    .unwrap_or_default()
            );
            crate::error_msg!("No versions found for {}{}", proj_title, extra);

            let support = self.loader_support(project_id).unwrap_or_default();
            if !support.is_empty() {
                println!("\n  Available support for '{}':", proj_title);
                println!("    {}", Self::format_loader_support(&support));
                println!();
            }
            if loader.is_none() {
                println!("  Hint: install a loader first, or specify --loader manually");
            }
            return Err(format!("No versions found for {proj_title}{extra}"));
        }

        let mut versions_arr = versions.as_array().unwrap().clone();

        let target = if let Some(vid) = version_id {
            versions_arr
                .iter()
                .find(|v| v["id"].as_str() == Some(vid))
                .cloned()
                .ok_or_else(|| format!("Version '{vid}' not found for {proj_title}"))?
        } else {
            // Sort: loader match first, then by date
            versions_arr.sort_by(|a, b| {
                let a_match = loader.as_ref().is_none_or(|l| {
                    a["loaders"].as_array().is_some_and(|arr| {
                        arr.iter().any(|v| {
                            v.as_str()
                                .is_some_and(|s| s.to_lowercase() == l.to_lowercase())
                        })
                    })
                });
                let b_match = loader.as_ref().is_none_or(|l| {
                    b["loaders"].as_array().is_some_and(|arr| {
                        arr.iter().any(|v| {
                            v.as_str()
                                .is_some_and(|s| s.to_lowercase() == l.to_lowercase())
                        })
                    })
                });
                if a_match != b_match {
                    if a_match {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Greater
                    }
                } else {
                    b["date_published"]
                        .as_str()
                        .unwrap_or("")
                        .cmp(a["date_published"].as_str().unwrap_or(""))
                }
            });
            versions_arr[0].clone()
        };

        let ver_num = target["version_number"]
            .as_str()
            .unwrap_or(target["id"].as_str().unwrap_or("?"));
        let mc_str = target["game_versions"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "?".to_string());
        let loaders_str = target["loaders"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "?".to_string());

        if let Some(ref l) = loader {
            let has_loader = target["loaders"].as_array().is_some_and(|arr| {
                arr.iter().any(|v| {
                    v.as_str()
                        .is_some_and(|s| s.to_lowercase() == l.to_lowercase())
                })
            });
            if !has_loader {
                crate::warn_msg!(
                    "Note: selected version uses loader '{}', not '{}'",
                    loaders_str,
                    l
                );
            }
        }

        crate::info!("Installing {}...", proj_title);
        println!("    Mod version:   {}", ver_num);
        println!("    Game versions: {}", mc_str);
        println!("    Loaders:       {}", loaders_str);

        // Check dependencies
        self.check_dependencies(
            &target,
            mc_version,
            loader.as_deref(),
            dependencies,
            visiting,
        )?;

        let dest_dir = self.mods_dir(mc_version);
        let l = if loader.is_some() {
            format!("{} {}", proj_title, ver_num)
        } else {
            proj_title.to_string()
        };
        let paths = modrinth::download_version_files(&target, &dest_dir, &l)
            .map_err(|error| format!("Mod installation failed: {error}"))?;
        let total_size: u64 = paths
            .iter()
            .filter_map(|p| p.metadata().ok().map(|m| m.len()))
            .sum();
        crate::success!(
            "Installed {} file(s) ({:.1} KB) -> {}",
            paths.len(),
            total_size as f64 / 1024.0,
            dest_dir.display()
        );

        visiting.remove(project_id);
        Ok((paths, target, project))
    }

    /// Check and display mod dependencies.
    fn check_dependencies(
        &self,
        version_data: &serde_json::Value,
        mc_version: &str,
        loader: Option<&str>,
        dependencies: DependencyMode,
        visiting: &mut std::collections::HashSet<String>,
    ) -> Result<(), String> {
        let deps: Vec<&serde_json::Value> = version_data["dependencies"]
            .as_array()
            .map(|arr| arr.iter().filter(|d| d["project_id"].is_string()).collect())
            .unwrap_or_default();

        if deps.is_empty() || dependencies == DependencyMode::Skip {
            return Ok(());
        }

        let dest_dir = self.mods_dir(mc_version);
        let existing_jars: Vec<String> = std::fs::read_dir(&dest_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                name.ends_with(".jar")
                    && !name.ends_with("-sources.jar")
                    && !name.ends_with("-javadoc.jar")
                    && !name.ends_with(".disabled")
            })
            .map(|e| e.file_name().to_string_lossy().to_lowercase())
            .collect();

        let mut required_missing: Vec<(String, String, String, String)> = Vec::new();
        let mut optional_list: Vec<(String, String, String, String, bool)> = Vec::new();

        for dep in &deps {
            let dep_pid = dep["project_id"].as_str().unwrap_or("");
            let dep_type = dep["dependency_type"].as_str().unwrap_or("required");

            let dep_proj = modrinth::get_project_result(dep_pid)
                .map_err(|error| format!("Cannot resolve dependency {dep_pid}: {error}"))?;
            let dep_title = dep_proj["title"].as_str().unwrap_or(dep_pid);
            let dep_slug = dep_proj["slug"].as_str().unwrap_or(dep_pid);

            let dep_versions = modrinth::get_project_versions_result(
                dep_pid,
                loader.map(|l| vec![l.to_string()]).as_deref(),
                Some(&[mc_version.to_string()]),
            )
            .map_err(|error| format!("Cannot list versions for dependency {dep_pid}: {error}"))?;

            let (dep_ver_num, dep_mc) = if let Some(arr) = dep_versions.as_array() {
                if arr.is_empty() {
                    ("(no matching version)".to_string(), "?".to_string())
                } else {
                    let best = &arr[0];
                    (
                        best["version_number"]
                            .as_str()
                            .unwrap_or(best["id"].as_str().unwrap_or("?"))
                            .to_string(),
                        best["game_versions"]
                            .as_array()
                            .map(|gv| {
                                gv.iter()
                                    .filter_map(|v| v.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_else(|| "?".to_string()),
                    )
                }
            } else {
                ("(no matching version)".to_string(), "?".to_string())
            };

            let dep_key_lower = dep_slug.to_lowercase();
            let dep_title_lower = dep_title.to_lowercase().replace(' ', "_");
            let present = existing_jars
                .iter()
                .any(|n| n.contains(&dep_key_lower) || n.contains(&dep_title_lower));

            match dep_type {
                "required" => {
                    if present {
                        crate::info!("[dep: required] {} -- already installed", dep_title);
                    } else {
                        required_missing.push((
                            dep_title.to_string(),
                            dep_slug.to_string(),
                            dep_ver_num,
                            dep_mc,
                        ));
                    }
                }
                _ => {
                    if present {
                        crate::info!("[dep: optional] {} -- already installed", dep_title);
                    }
                    optional_list.push((
                        dep_title.to_string(),
                        dep_slug.to_string(),
                        dep_ver_num,
                        dep_mc,
                        present,
                    ));
                }
            }
        }

        if required_missing.is_empty() && optional_list.is_empty() {
            return Ok(());
        }

        println!();
        crate::log::header("Dependencies");

        if !required_missing.is_empty() {
            println!("  \x1b[1m\x1b[91m[ MUST INSTALL ] Required dependencies:\x1b[0m");
            for (title, slug, ver, mc) in &required_missing {
                println!("    \x1b[91m- {}\x1b[0m", title);
                println!("      slug:    {}", slug);
                println!("      version: {}  (MC: {})", ver, mc);
                print!(
                    "      install: hot-rs-launcher install-mod {} -v {}",
                    slug, mc_version
                );
                if let Some(l) = loader {
                    print!(" --loader {}", l);
                }
                println!();
            }
        }

        if !optional_list.is_empty() {
            println!("  \x1b[1m\x1b[93m[ RECOMMENDED ] Optional dependencies:\x1b[0m");
            for (title, slug, ver, mc, present) in &optional_list {
                let mark = if *present {
                    "\x1b[92m[installed]\x1b[0m "
                } else {
                    ""
                };
                println!("    {}{}", mark, title);
                println!("      slug:    {}", slug);
                println!("      version: {}  (MC: {})", ver, mc);
                print!(
                    "      install: hot-rs-launcher install-mod {} -v {}",
                    slug, mc_version
                );
                if let Some(l) = loader {
                    print!(" --loader {}", l);
                }
                println!();
            }
        }

        if !required_missing.is_empty() {
            crate::warn_msg!(
                "{} required dependency(ies) missing. Install them first, or the mod may not load.",
                required_missing.len()
            );

            let install_required = match dependencies {
                DependencyMode::Auto => true,
                DependencyMode::Ask => {
                    use std::io::IsTerminal;
                    if std::io::stdin().is_terminal() {
                        print!("\n  Auto-install missing dependencies? [y/N]: ");
                        use std::io::{self, Write};
                        io::stdout().flush().ok();
                        let mut answer = String::new();
                        io::stdin().read_line(&mut answer).is_ok()
                            && matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
                    } else {
                        println!(
                            "  Hint: re-run with --yes to install required dependencies automatically."
                        );
                        false
                    }
                }
                DependencyMode::Skip => false,
            };
            if install_required {
                for (title, slug, _ver, _mc) in &required_missing {
                    crate::info!("Installing dependency: {} ({})", title, slug);
                    self.install_inner(
                        slug,
                        mc_version,
                        loader,
                        None,
                        DependencyMode::Auto,
                        visiting,
                    )?;
                    crate::success!("Installed: {}", title);
                }
            }
        }
        println!();
        Ok(())
    }

    /// List installed mods for a version.
    pub fn list_mods(&self, mc_version: &str) -> Vec<(String, bool, u64)> {
        let mods_dir = self.mods_dir(mc_version);
        let mut results = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&mods_dir) {
            let mut files: Vec<_> = entries.flatten().collect();
            files.sort_by_key(|e| e.file_name());
            for f in files {
                let name = f.file_name().to_string_lossy().to_string();
                if !f.file_type().is_ok_and(|kind| kind.is_file()) {
                    continue;
                }
                let size = f.metadata().map(|m| m.len()).unwrap_or(0);
                if name.ends_with(".jar.disabled") {
                    results.push((name[..name.len() - 9].to_string(), false, size));
                } else if name.ends_with(".jar") {
                    results.push((name, true, size));
                }
            }
        }
        results
    }

    /// An exact filename wins; fuzzy queries must identify exactly one regular file.
    fn find_mod(&self, query: &str, mc_version: &str, enabled: Option<bool>) -> Option<PathBuf> {
        if crate::util::validate_component(query).is_err() {
            return None;
        }
        let candidates: Vec<_> = std::fs::read_dir(self.mods_dir(mc_version))
            .ok()?
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
            .filter(|path| {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                match enabled {
                    Some(true) => name.ends_with(".jar"),
                    Some(false) => name.ends_with(".jar.disabled"),
                    None => name.ends_with(".jar") || name.ends_with(".jar.disabled"),
                }
            })
            .collect();
        let exact: Vec<_> = candidates
            .iter()
            .filter(|path| {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                name == query
                    || name == format!("{query}.jar")
                    || name == format!("{query}.jar.disabled")
            })
            .collect();
        if exact.len() == 1 {
            return Some(exact[0].clone());
        }
        let matches: Vec<_> = candidates
            .iter()
            .filter(|path| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&query.to_lowercase())
            })
            .collect();
        if matches.len() == 1 {
            return Some(matches[0].clone());
        }
        if matches.len() > 1 {
            crate::warn_msg!(
                "Ambiguous mod query '{query}'; use an exact filename. No files changed."
            );
        }
        None
    }

    pub fn disable_mod(&self, slug: &str, mc_version: &str) -> bool {
        let Some(source) = self.find_mod(slug, mc_version, Some(true)) else {
            return false;
        };
        let dest = source.with_file_name(format!(
            "{}.disabled",
            source.file_name().unwrap_or_default().to_string_lossy()
        ));
        !dest.exists() && std::fs::rename(source, dest).is_ok()
    }

    pub fn enable_mod(&self, slug: &str, mc_version: &str) -> bool {
        let Some(source) = self.find_mod(slug, mc_version, Some(false)) else {
            return false;
        };
        let dest = source.with_extension("");
        !dest.exists() && std::fs::rename(source, dest).is_ok()
    }

    pub fn uninstall_mod(&self, slug: &str, mc_version: &str) -> Vec<String> {
        let Some(source) = self.find_mod(slug, mc_version, None) else {
            return Vec::new();
        };
        if std::fs::remove_file(&source).is_ok() {
            vec![source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()]
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mod_lifecycle_and_ambiguous_queries() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModManager::new(dir.path());
        let mods = manager.mods_dir("1.21.4");
        std::fs::create_dir_all(&mods).unwrap();
        for name in ["sodium.jar", "sodium-extra.jar"] {
            std::fs::write(mods.join(name), b"fixture").unwrap();
        }
        assert!(!manager.disable_mod("sod", "1.21.4"));
        assert!(manager.uninstall_mod("sod", "1.21.4").is_empty());
        assert!(manager.disable_mod("sodium.jar", "1.21.4"));
        assert!(mods.join("sodium.jar.disabled").exists());
        assert!(manager.enable_mod("sodium.jar.disabled", "1.21.4"));
        assert_eq!(
            manager.uninstall_mod("sodium.jar", "1.21.4"),
            vec!["sodium.jar"]
        );
        assert!(mods.join("sodium-extra.jar").exists());
        assert!(manager.uninstall_mod("../outside", "1.21.4").is_empty());
    }
    #[test]
    fn listing_does_not_create_directories() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModManager::new(dir.path());
        assert!(manager.list_mods("1.21.4").is_empty());
        assert!(!dir.path().join("versions").exists());
    }
}
