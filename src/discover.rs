//! Modrinth discovery (search and browse) plus one-click installs for
//! resource packs and shader packs.

use std::path::Path;

use crate::content::{ContentManager, ShaderSupport};
use crate::modrinth;

pub const PAGE_SIZE: u32 = 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiscoverKind {
    #[default]
    Mods,
    ResourcePacks,
    Shaders,
}

impl DiscoverKind {
    pub fn project_type(self) -> &'static str {
        match self {
            Self::Mods => "mod",
            Self::ResourcePacks => "resourcepack",
            Self::Shaders => "shader",
        }
    }

    pub fn title(self, chinese: bool) -> &'static str {
        match (self, chinese) {
            (Self::Mods, false) => "Mods",
            (Self::Mods, true) => "模组",
            (Self::ResourcePacks, false) => "Resource packs",
            (Self::ResourcePacks, true) => "资源包",
            (Self::Shaders, false) => "Shaders",
            (Self::Shaders, true) => "光影包",
        }
    }
}

/// Search or browse Modrinth. An empty query with the downloads sort is the
/// browse mode used by the TUI when a screen opens.
#[allow(clippy::too_many_arguments)]
pub fn search(
    kind: DiscoverKind,
    query: &str,
    sort: &str,
    page: u32,
    limit: u32,
    game_version: Option<&str>,
    loader: Option<&str>,
    category: Option<&str>,
) -> Result<serde_json::Value, String> {
    let mut facets = vec![serde_json::json!([format!(
        "project_type:{}",
        kind.project_type()
    )])];
    if kind == DiscoverKind::Mods {
        if let Some(loader) = loader.filter(|value| !value.is_empty() && *value != "auto") {
            facets.push(serde_json::json!([format!(
                "categories:{}",
                loader.to_ascii_lowercase()
            )]));
        }
        if let Some(category) = category.filter(|value| !value.is_empty()) {
            facets.push(serde_json::json!([format!(
                "categories:{}",
                category.to_ascii_lowercase()
            )]));
        }
    }
    if let Some(game_version) = game_version.filter(|value| !value.is_empty()) {
        facets.push(serde_json::json!([format!("versions:{game_version}")]));
    }
    let sort = if query.trim().is_empty() {
        "downloads"
    } else {
        sort
    };
    let limit = limit.clamp(1, 50);
    let offset = page.saturating_mul(limit);
    modrinth::search_projects_result(query, sort, limit, offset, Some(&serde_json::json!(facets)))
}

/// Annotate mod search hits with installed state and version.
pub fn mark_installed_mods(value: &mut serde_json::Value, game_dir: &Path, mc_version: &str) {
    let Ok(installed) = crate::update::installed_projects(game_dir, mc_version) else {
        return;
    };
    if let Some(hits) = value["hits"].as_array_mut() {
        for hit in hits {
            if let Some(project_id) = hit["project_id"].as_str() {
                let version = installed.get(project_id);
                hit["_installed"] = serde_json::json!(version.is_some());
                if let Some(version) = version {
                    hit["_installed_version"] = serde_json::json!(version);
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ContentInstall {
    pub title: String,
    pub file: String,
    pub enabled: bool,
    pub support: ShaderSupport,
}

fn compatible_versions(slug: &str, mc_version: &str) -> Result<Vec<serde_json::Value>, String> {
    let versions =
        modrinth::get_project_versions_result(slug, None, Some(&[mc_version.to_string()]))?;
    Ok(versions.as_array().cloned().unwrap_or_default())
}

fn project_title(project: &serde_json::Value, slug: &str) -> String {
    project["title"].as_str().unwrap_or(slug).to_string()
}

fn expect_project_type(
    project: &serde_json::Value,
    expected: &str,
    slug: &str,
) -> Result<(), String> {
    let actual = project["project_type"].as_str().unwrap_or("unknown");
    if actual != expected {
        return Err(format!(
            "'{slug}' is a {actual} project, not a {expected} project"
        ));
    }
    Ok(())
}

fn download_first_file(
    version: &serde_json::Value,
    dir: &Path,
    title: &str,
) -> Result<String, String> {
    let paths = modrinth::download_version_files(version, dir, title)
        .map_err(|error| format!("Download failed: {error}"))?;
    paths
        .first()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| "No downloadable file found".to_string())
}

/// Download a Modrinth resource pack into the instance and optionally enable it.
pub fn install_resourcepack(
    game_dir: &Path,
    instance: &str,
    slug: &str,
    enable: bool,
) -> Result<ContentInstall, String> {
    let project = modrinth::get_project_result(slug)?;
    expect_project_type(&project, "resourcepack", slug)?;
    let title = project_title(&project, slug);
    let versions = compatible_versions(slug, instance)?;
    let version = versions
        .first()
        .ok_or_else(|| format!("{title} has no version compatible with Minecraft {instance}"))?;
    let manager = ContentManager::new(game_dir);
    let dir = manager
        .resourcepacks_dir(instance)
        .map_err(|error| error.to_string())?;
    let file = download_first_file(version, &dir, &title)?;
    let enabled = if enable {
        manager
            .set_resourcepack_enabled(instance, &file, true)
            .map_err(|error| error.to_string())?;
        true
    } else {
        false
    };
    Ok(ContentInstall {
        title,
        file,
        enabled,
        support: ShaderSupport::default(),
    })
}

fn shader_score(version: &serde_json::Value) -> u8 {
    let support = ShaderSupport::from_version(version);
    u8::from(support.iris) * 2 + u8::from(support.optifine)
}

/// Download a Modrinth shader pack, preferring versions that support both Iris
/// and OptiFine, and record the detected loader support for the management UI.
pub fn install_shader(
    game_dir: &Path,
    instance: &str,
    slug: &str,
    enable: bool,
) -> Result<ContentInstall, String> {
    let project = modrinth::get_project_result(slug)?;
    expect_project_type(&project, "shader", slug)?;
    let title = project_title(&project, slug);
    let versions = compatible_versions(slug, instance)?;
    let version = versions
        .iter()
        .max_by(|a, b| {
            shader_score(a).cmp(&shader_score(b)).then_with(|| {
                a["date_published"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["date_published"].as_str().unwrap_or(""))
            })
        })
        .ok_or_else(|| format!("{title} has no version compatible with Minecraft {instance}"))?;
    let support = ShaderSupport::from_version(version);
    let manager = ContentManager::new(game_dir);
    let dir = manager
        .shaderpacks_dir(instance)
        .map_err(|error| error.to_string())?;
    let file = download_first_file(version, &dir, &title)?;
    manager
        .record_shader_meta(instance, &file, support, Some(&title), Some(slug))
        .map_err(|error| error.to_string())?;
    let enabled = if enable {
        manager
            .set_shader_enabled(instance, &file, true)
            .map_err(|error| error.to_string())?;
        true
    } else {
        false
    };
    Ok(ContentInstall {
        title,
        file,
        enabled,
        support,
    })
}

/// The mod that provides Iris on this loader (Oculus on Forge).
pub fn shader_loader_mod(loader: &str) -> Option<(&'static str, &'static str)> {
    match loader {
        "fabric" | "neoforge" => Some(("iris", "Iris")),
        "forge" => Some(("oculus", "Oculus")),
        _ => None,
    }
}

/// Top optimization mods for this instance, newest/most downloaded first.
pub fn recommended_optimizations(
    game_dir: &Path,
    instance: &str,
    loader: Option<&str>,
    limit: u32,
) -> Result<Vec<serde_json::Value>, String> {
    let limit = limit.clamp(1, 30);
    // Ask for a few extra results because Iris/Oculus are filtered out below.
    let mut value = search(
        DiscoverKind::Mods,
        "",
        "downloads",
        0,
        (limit + 4).min(50),
        Some(instance),
        loader,
        Some("optimization"),
    )?;
    mark_installed_mods(&mut value, game_dir, instance);
    let mut hits = value["hits"].as_array().cloned().unwrap_or_default();
    // Iris/Oculus are shader loaders, not performance mods; they have their
    // own prompt in the shader flow.
    hits.retain(|hit| !matches!(hit["slug"].as_str(), Some("iris") | Some("oculus")));
    hits.truncate(limit as usize);
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_support_labels_cover_iris_and_optifine() {
        let both = ShaderSupport::from_labels(["iris", "optifine", "bloom"]);
        assert!(both.iris && both.optifine);
        assert_eq!(both.label(), "Iris + OptiFine");
        assert!(both.summary(false).contains("Iris"));

        let iris = ShaderSupport::from_labels(["Iris"]);
        assert!(iris.iris && !iris.optifine);
        assert_eq!(iris.label(), "Iris");

        let optifine = ShaderSupport::from_labels(["optifine"]);
        assert!(!optifine.iris && optifine.optifine);
        assert_eq!(optifine.label(), "OptiFine");

        let none = ShaderSupport::from_labels(["bloom"]);
        assert!(none.is_empty());
        assert_eq!(none.label(), "unknown");
    }

    #[test]
    fn shader_versions_prefer_both_loaders_then_iris() {
        let both = serde_json::json!({"loaders":["iris","optifine"],"date_published":"2024-01-01"});
        let iris = serde_json::json!({"loaders":["iris"],"date_published":"2024-06-01"});
        let optifine = serde_json::json!({"loaders":["optifine"],"date_published":"2024-06-01"});
        assert!(shader_score(&both) > shader_score(&iris));
        assert!(shader_score(&iris) > shader_score(&optifine));
        assert_eq!(shader_score(&both), 3);
    }

    #[test]
    fn shader_loader_mods_match_the_loader() {
        assert_eq!(shader_loader_mod("fabric"), Some(("iris", "Iris")));
        assert_eq!(shader_loader_mod("neoforge"), Some(("iris", "Iris")));
        assert_eq!(shader_loader_mod("forge"), Some(("oculus", "Oculus")));
        assert_eq!(shader_loader_mod("vanilla"), None);
    }
}
