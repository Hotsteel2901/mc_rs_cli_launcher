//! World (save) and resource-pack management for an instance.
//!
//! Everything here is local-only: listing, backing up/importing worlds,
//! enabling/disabling resource packs through options.txt, importing and
//! deleting packs. No network access is required.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::instance::dir_size;
use crate::util;

pub const SAVES_DIR: &str = "saves";
pub const RESOURCEPACKS_DIR: &str = "resourcepacks";
pub const SHADERPACKS_DIR: &str = "shaderpacks";
const CONTENT_META: &str = "launcher_content.json";
pub const OPTIONS_FILE: &str = "options.txt";
const BACKUP_MANIFEST: &str = "backup.json";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveEntry {
    pub folder: String,
    pub name: String,
    pub version: Option<String>,
    pub game_type: Option<u8>,
    pub hardcore: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub has_icon: bool,
}

impl SaveEntry {
    pub fn modified_unix(&self) -> Option<u64> {
        self.modified
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourcePackEntry {
    pub file: String,
    pub is_dir: bool,
    pub enabled: bool,
    pub incompatible: bool,
    pub size: u64,
    pub pack_format: Option<u32>,
    pub description: String,
    pub valid: bool,
}

/// Which shader loaders a pack supports, derived from Modrinth loaders or the
/// metadata recorded when the launcher installed it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderSupport {
    pub iris: bool,
    pub optifine: bool,
}

impl ShaderSupport {
    pub fn from_labels<'a>(labels: impl IntoIterator<Item = &'a str>) -> Self {
        let mut support = Self::default();
        for label in labels {
            match label.to_ascii_lowercase().as_str() {
                "iris" | "oculus" => support.iris = true,
                "optifine" => support.optifine = true,
                _ => {}
            }
        }
        support
    }

    pub fn from_version(version: &serde_json::Value) -> Self {
        let labels: Vec<&str> = version["loaders"]
            .as_array()
            .map(|values| values.iter().filter_map(|value| value.as_str()).collect())
            .unwrap_or_default();
        Self::from_labels(labels)
    }

    pub fn label(self) -> &'static str {
        match (self.iris, self.optifine) {
            (true, true) => "Iris + OptiFine",
            (true, false) => "Iris",
            (false, true) => "OptiFine",
            _ => "unknown",
        }
    }

    pub fn summary(self, chinese: bool) -> String {
        match (self.iris, self.optifine, chinese) {
            (true, true, true) => "Iris / OptiFine 均支持".into(),
            (true, false, true) => "仅支持 Iris".into(),
            (false, true, true) => "仅支持 OptiFine".into(),
            (true, true, false) => "Iris + OptiFine".into(),
            (true, false, false) => "Iris only".into(),
            (false, true, false) => "OptiFine only".into(),
            (_, _, true) => "未知".into(),
            _ => "unknown".into(),
        }
    }

    pub fn is_empty(self) -> bool {
        !self.iris && !self.optifine
    }
}

/// Shader loader availability in one instance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderLoaderStatus {
    pub iris: bool,
    pub optifine: bool,
}

impl ShaderLoaderStatus {
    pub fn summary(self, chinese: bool) -> String {
        match (self.iris, self.optifine, chinese) {
            (true, true, true) => "Iris 与 OptiFine 均已安装".into(),
            (true, false, true) => "已安装 Iris".into(),
            (false, true, true) => "已安装 OptiFine".into(),
            (true, true, false) => "Iris and OptiFine installed".into(),
            (true, false, false) => "Iris installed".into(),
            (false, true, false) => "OptiFine installed".into(),
            (_, _, true) => "未安装光影加载器".into(),
            _ => "no shader loader installed".into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShaderEntry {
    pub file: String,
    pub is_dir: bool,
    pub enabled: bool,
    pub size: u64,
    pub support: Option<ShaderSupport>,
    pub title: Option<String>,
    pub project: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderMeta {
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
}

impl ShaderMeta {
    pub fn support(&self) -> ShaderSupport {
        ShaderSupport::from_labels(self.loaders.iter().map(String::as_str))
    }
}

#[derive(Default, Serialize, Deserialize)]
struct ContentMeta {
    #[serde(default)]
    shaders: std::collections::BTreeMap<String, ShaderMeta>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PackMeta {
    format: Option<u32>,
    description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct LevelInfo {
    name: Option<String>,
    version: Option<String>,
    game_type: Option<u8>,
    hardcore: bool,
}

pub fn game_mode_label(code: u8, chinese: bool) -> &'static str {
    match (code, chinese) {
        (0, false) => "survival",
        (0, true) => "生存",
        (1, false) => "creative",
        (1, true) => "创造",
        (2, false) => "adventure",
        (2, true) => "冒险",
        (3, false) => "spectator",
        (3, true) => "旁观",
        (_, false) => "unknown",
        (_, true) => "未知",
    }
}

pub struct ContentManager {
    pub game_dir: PathBuf,
}

impl ContentManager {
    pub fn new(game_dir: &Path) -> Self {
        Self {
            game_dir: game_dir.to_path_buf(),
        }
    }

    fn instance_dir(&self, id: &str) -> AppResult<PathBuf> {
        util::validate_component(id)?;
        Ok(self.game_dir.join("versions").join(id))
    }

    pub fn saves_dir(&self, id: &str) -> AppResult<PathBuf> {
        Ok(self.instance_dir(id)?.join(SAVES_DIR))
    }

    pub fn resourcepacks_dir(&self, id: &str) -> AppResult<PathBuf> {
        Ok(self.instance_dir(id)?.join(RESOURCEPACKS_DIR))
    }

    fn options_path(&self, id: &str) -> AppResult<PathBuf> {
        Ok(self.instance_dir(id)?.join(OPTIONS_FILE))
    }

    /// List worlds that contain a level.dat, newest first.
    pub fn list_saves(&self, id: &str) -> Vec<SaveEntry> {
        let Ok(dir) = self.saves_dir(id) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        if let Ok(read_dir) = fs::read_dir(&dir) {
            for entry in read_dir.flatten() {
                if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                let folder = entry.file_name().to_string_lossy().to_string();
                let path = entry.path();
                let level = path.join("level.dat");
                if !level.is_file() {
                    continue;
                }
                let info = read_level_info(&level).unwrap_or_default();
                let modified = entry.metadata().and_then(|meta| meta.modified()).ok();
                entries.push(SaveEntry {
                    name: info
                        .name
                        .filter(|name| !name.trim().is_empty())
                        .unwrap_or_else(|| folder.clone()),
                    folder,
                    version: info.version,
                    game_type: info.game_type,
                    hardcore: info.hardcore,
                    size: dir_size(&path),
                    modified,
                    has_icon: path.join("icon.png").is_file(),
                });
            }
        }
        entries.sort_by(|a, b| {
            b.modified
                .cmp(&a.modified)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        entries
    }

    /// Zip one world into backups/saves (or a caller-provided path).
    pub fn backup_save(&self, id: &str, folder: &str, dest: Option<&Path>) -> AppResult<PathBuf> {
        util::validate_component(folder)?;
        let saves = self.saves_dir(id)?;
        let world = saves.join(folder);
        if !world.join("level.dat").is_file() {
            return Err(AppError::Generic(format!(
                "World '{folder}' does not exist in this instance"
            )));
        }
        let dest = dest.map(|path| path.to_path_buf()).unwrap_or_else(|| {
            self.game_dir.join("backups").join("saves").join(format!(
                "{}-{}-{}.zip",
                id,
                folder,
                crate::instance::utc_stamp(SystemTime::now())
            ))
        });
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let info = read_level_info(&world.join("level.dat")).unwrap_or_default();
        let manifest = serde_json::json!({
            "kind": "world",
            "instance": id,
            "folder": folder,
            "name": info.name.unwrap_or_else(|| folder.to_string()),
            "created_at": now_epoch(),
        });
        let file = fs::File::create(&dest)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = file_options();
        zip.start_file(BACKUP_MANIFEST, options)
            .map_err(zip_error)?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
        add_world_tree(&mut zip, &world, Path::new(""), options)?;
        zip.finish().map_err(zip_error)?;
        Ok(dest)
    }

    /// Import a world zip (a launcher backup or a vanilla world archive).
    /// Returns the folder name that was created under saves/.
    pub fn import_save(&self, id: &str, archive: &Path, name: Option<&str>) -> AppResult<String> {
        let saves = self.saves_dir(id)?;
        let file = fs::File::open(archive)?;
        let mut zip = zip::ZipArchive::new(file).map_err(zip_error)?;
        let mut names = Vec::new();
        for index in 0..zip.len() {
            if let Ok(entry) = zip.by_index(index) {
                names.push(entry.name().to_string());
            }
        }
        let manifest: Option<serde_json::Value> = {
            match zip.by_name(BACKUP_MANIFEST) {
                Ok(mut entry) => {
                    let mut text = String::new();
                    entry.read_to_string(&mut text).ok();
                    serde_json::from_str(&text).ok()
                }
                Err(_) => None,
            }
        };
        let prefix = world_prefix(&names).ok_or_else(|| {
            AppError::Generic(
                "Archive does not contain a Minecraft world (level.dat was not found)".into(),
            )
        })?;
        let base = name
            .map(|value| value.to_string())
            .or_else(|| {
                manifest
                    .as_ref()
                    .and_then(|value| value["folder"].as_str().map(String::from))
            })
            .or_else(|| {
                archive
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "world".into());
        let base = sanitize_world_folder(&base);
        if let Some(name) = name {
            util::validate_component(name)?;
        }
        fs::create_dir_all(&saves)?;
        let folder = unique_world_folder(&saves, &base);
        let target = saves.join(&folder);
        fs::create_dir_all(&target)?;

        for index in 0..zip.len() {
            let mut entry = match zip.by_index(index) {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let entry_name = entry.name().to_string();
            if entry_name == BACKUP_MANIFEST
                || entry_name.ends_with('/')
                || entry_name.starts_with("__MACOSX/")
            {
                continue;
            }
            let relative = match &prefix {
                Some(prefix) => match entry_name.strip_prefix(&format!("{prefix}/")) {
                    Some(relative) => relative.to_string(),
                    None => continue,
                },
                None => entry_name,
            };
            if relative.is_empty() {
                continue;
            }
            let dest = util::safe_join(&target, &relative)?;
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            util::atomic_write(&dest, &bytes)?;
        }
        if !target.join("level.dat").is_file() {
            let _ = fs::remove_dir_all(&target);
            return Err(AppError::Generic(
                "World archive did not produce a level.dat".into(),
            ));
        }
        Ok(folder)
    }

    pub fn delete_save(&self, id: &str, folder: &str) -> AppResult<u64> {
        util::validate_component(folder)?;
        let world = self.saves_dir(id)?.join(folder);
        if !world.is_dir() {
            return Err(AppError::Generic(format!(
                "World '{folder}' does not exist in this instance"
            )));
        }
        let size = dir_size(&world);
        fs::remove_dir_all(&world)?;
        Ok(size)
    }

    /// List zip and directory packs, with enabled state from options.txt.
    pub fn list_resourcepacks(&self, id: &str) -> Vec<ResourcePackEntry> {
        let Ok(dir) = self.resourcepacks_dir(id) else {
            return Vec::new();
        };
        let options = self.options_path(id).unwrap_or_default();
        let lines = read_lines(&options);
        let enabled = array_value(&lines, "resourcePacks");
        let incompatible = array_value(&lines, "incompatibleResourcePacks");
        let mut entries = Vec::new();
        if let Ok(read_dir) = fs::read_dir(&dir) {
            for entry in read_dir.flatten() {
                let file = entry.file_name().to_string_lossy().to_string();
                let path = entry.path();
                let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
                if is_dir {
                    if !path.join("pack.mcmeta").is_file() {
                        continue;
                    }
                } else if !file.to_ascii_lowercase().ends_with(".zip") {
                    continue;
                }
                let key = format!("file/{file}");
                let meta = read_pack_meta(&path, is_dir);
                let (pack_format, description, valid) = match meta {
                    Some(meta) => (meta.format, meta.description, true),
                    None => (None, String::new(), false),
                };
                entries.push(ResourcePackEntry {
                    file,
                    is_dir,
                    enabled: enabled.iter().any(|value| value == &key),
                    incompatible: incompatible.iter().any(|value| value == &key),
                    size: if is_dir {
                        dir_size(&path)
                    } else {
                        entry.metadata().map(|meta| meta.len()).unwrap_or(0)
                    },
                    pack_format,
                    description,
                    valid,
                });
            }
        }
        entries.sort_by(|a, b| {
            b.enabled
                .cmp(&a.enabled)
                .then_with(|| a.file.to_lowercase().cmp(&b.file.to_lowercase()))
        });
        entries
    }

    /// Enable or disable a pack by editing options.txt, the same mechanism the
    /// vanilla resource-pack screen uses.
    pub fn set_resourcepack_enabled(&self, id: &str, file: &str, enabled: bool) -> AppResult<bool> {
        util::validate_component(file)?;
        let target = self.resourcepacks_dir(id)?.join(file);
        if !target.exists() {
            return Err(AppError::Generic(format!(
                "Resource pack '{file}' does not exist in this instance"
            )));
        }
        let options = self.options_path(id)?;
        let existing = read_lines_checked(&options)?;
        if existing.is_none() && !enabled {
            return Ok(false);
        }
        let mut lines = existing.unwrap_or_default();
        let key = format!("file/{file}");
        let mut packs = array_value(&lines, "resourcePacks");
        if enabled {
            packs.retain(|value| value != &key);
            packs.insert(0, key.clone());
            let mut incompatible = array_value(&lines, "incompatibleResourcePacks");
            incompatible.retain(|value| value != &key);
            set_array(&mut lines, "incompatibleResourcePacks", &incompatible);
        } else {
            packs.retain(|value| value != &key);
        }
        set_array(&mut lines, "resourcePacks", &packs);
        write_lines(&options, &lines)?;
        Ok(true)
    }

    /// Copy a pack zip or folder into resourcepacks/ after validating mcmeta.
    pub fn import_resourcepack(&self, id: &str, source: &Path) -> AppResult<String> {
        let dir = self.resourcepacks_dir(id)?;
        let is_dir = source.is_dir();
        if !is_dir && !source.is_file() {
            return Err(AppError::Generic(format!(
                "Resource pack source does not exist: {}",
                source.display()
            )));
        }
        if !is_dir
            && !source
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".zip")
        {
            return Err(AppError::Generic(
                "Resource packs must be a .zip file or a folder".into(),
            ));
        }
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .ok_or_else(|| AppError::Generic("Cannot determine the resource pack name".into()))?;
        util::validate_component(&name)?;
        if is_dir {
            if !source.join("pack.mcmeta").is_file() {
                return Err(AppError::Generic(
                    "Folder has no pack.mcmeta and is not a resource pack".into(),
                ));
            }
        } else if !zip_has_pack_meta(source)? {
            return Err(AppError::Generic(
                "Zip has no pack.mcmeta and is not a resource pack".into(),
            ));
        }
        fs::create_dir_all(&dir)?;
        let target = dir.join(&name);
        if target.exists() {
            return Err(AppError::Generic(format!(
                "Resource pack '{name}' already exists"
            )));
        }
        if is_dir {
            copy_tree(source, &target)?;
        } else {
            fs::copy(source, &target)?;
        }
        Ok(name)
    }

    pub fn delete_resourcepack(&self, id: &str, file: &str) -> AppResult<u64> {
        util::validate_component(file)?;
        let target = self.resourcepacks_dir(id)?.join(file);
        if !target.exists() {
            return Err(AppError::Generic(format!(
                "Resource pack '{file}' does not exist in this instance"
            )));
        }
        let size = if target.is_dir() {
            dir_size(&target)
        } else {
            target.metadata().map(|meta| meta.len()).unwrap_or(0)
        };
        // Read options.txt before deleting so an unreadable file never causes a
        // partial delete followed by a silent rewrite.
        let options = self.options_path(id)?;
        let existing = read_lines_checked(&options)?;
        if target.is_dir() {
            fs::remove_dir_all(&target)?;
        } else {
            fs::remove_file(&target)?;
        }
        // Drop stale references so Minecraft does not warn about a missing pack.
        if let Some(mut lines) = existing {
            let key = format!("file/{file}");
            let mut packs = array_value(&lines, "resourcePacks");
            packs.retain(|value| value != &key);
            set_array(&mut lines, "resourcePacks", &packs);
            let mut incompatible = array_value(&lines, "incompatibleResourcePacks");
            incompatible.retain(|value| value != &key);
            set_array(&mut lines, "incompatibleResourcePacks", &incompatible);
            write_lines(&options, &lines)?;
        }
        Ok(size)
    }

    pub fn shaderpacks_dir(&self, id: &str) -> AppResult<PathBuf> {
        Ok(self.instance_dir(id)?.join(SHADERPACKS_DIR))
    }

    fn meta_path(&self, id: &str) -> AppResult<PathBuf> {
        Ok(self.instance_dir(id)?.join(CONTENT_META))
    }

    fn read_meta(&self, id: &str) -> ContentMeta {
        self.meta_path(id)
            .ok()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn write_meta(&self, id: &str, meta: &ContentMeta) -> AppResult<()> {
        let path = self.meta_path(id)?;
        util::atomic_write(&path, &serde_json::to_vec_pretty(meta)?)?;
        Ok(())
    }

    pub fn record_shader_meta(
        &self,
        id: &str,
        file: &str,
        support: ShaderSupport,
        title: Option<&str>,
        project: Option<&str>,
    ) -> AppResult<()> {
        let mut meta = self.read_meta(id);
        let mut loaders = Vec::new();
        if support.iris {
            loaders.push("iris".to_string());
        }
        if support.optifine {
            loaders.push("optifine".to_string());
        }
        meta.shaders.insert(
            file.to_string(),
            ShaderMeta {
                loaders,
                title: title.map(String::from),
                project: project.map(String::from),
            },
        );
        self.write_meta(id, &meta)
    }

    fn forget_shader_meta(&self, id: &str, file: &str) -> AppResult<()> {
        let mut meta = self.read_meta(id);
        if meta.shaders.remove(file).is_some() {
            self.write_meta(id, &meta)?;
        }
        Ok(())
    }

    fn optifine_installed(&self, id: &str) -> bool {
        let Ok(dir) = self.instance_dir(id) else {
            return false;
        };
        if dir.join("optionsof.txt").is_file() {
            return true;
        }
        if let Ok(entries) = fs::read_dir(dir.join("mods")) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("optifine")
                {
                    return true;
                }
            }
        }
        fs::read(dir.join(format!("{id}.json")))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value["id"].as_str().map(|value| value.to_ascii_lowercase()))
            .is_some_and(|value| value.contains("optifine"))
    }

    pub fn shader_loader_status(&self, id: &str) -> ShaderLoaderStatus {
        let mut status = ShaderLoaderStatus {
            optifine: self.optifine_installed(id),
            iris: false,
        };
        if let Ok(dir) = self.instance_dir(id) {
            if let Ok(entries) = fs::read_dir(dir.join("mods")) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                    if name.contains("iris") || name.contains("oculus") {
                        status.iris = true;
                    }
                    if name.contains("optifine") {
                        status.optifine = true;
                    }
                }
            }
        }
        status
    }

    fn shader_options_files(&self, id: &str) -> AppResult<Vec<PathBuf>> {
        let dir = self.instance_dir(id)?;
        let mut files = vec![dir.join(OPTIONS_FILE)];
        let optionsof = dir.join("optionsof.txt");
        if optionsof.exists() || self.optifine_installed(id) {
            files.push(optionsof);
        }
        Ok(files)
    }

    fn read_shader_pack(&self, id: &str) -> Option<String> {
        for path in self.shader_options_files(id).ok()? {
            if let Some(value) = scalar_value(&read_lines(&path), "shaderPack") {
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
        None
    }

    fn write_shader_pack(&self, id: &str, value: &str) -> AppResult<()> {
        for path in self.shader_options_files(id)? {
            let mut lines = read_lines_checked(&path)?.unwrap_or_default();
            set_scalar(&mut lines, "shaderPack", value);
            write_lines(&path, &lines)?;
        }
        Ok(())
    }

    pub fn list_shaders(&self, id: &str) -> Vec<ShaderEntry> {
        let Ok(dir) = self.shaderpacks_dir(id) else {
            return Vec::new();
        };
        let active = self.read_shader_pack(id);
        let meta = self.read_meta(id);
        let mut entries = Vec::new();
        if let Ok(read_dir) = fs::read_dir(&dir) {
            for entry in read_dir.flatten() {
                let file = entry.file_name().to_string_lossy().to_string();
                let path = entry.path();
                let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
                if is_dir {
                    if !path.join("shaders").is_dir() {
                        continue;
                    }
                } else if !file.to_ascii_lowercase().ends_with(".zip") {
                    continue;
                }
                let meta = meta.shaders.get(&file);
                entries.push(ShaderEntry {
                    enabled: active.as_deref() == Some(file.as_str()),
                    size: if is_dir {
                        dir_size(&path)
                    } else {
                        entry.metadata().map(|meta| meta.len()).unwrap_or(0)
                    },
                    support: meta.map(|meta| meta.support()),
                    title: meta.and_then(|meta| meta.title.clone()),
                    project: meta.and_then(|meta| meta.project.clone()),
                    file,
                    is_dir,
                });
            }
        }
        entries.sort_by(|a, b| {
            b.enabled
                .cmp(&a.enabled)
                .then_with(|| a.file.to_lowercase().cmp(&b.file.to_lowercase()))
        });
        entries
    }

    pub fn disable_shader(&self, id: &str) -> AppResult<()> {
        self.write_shader_pack(id, "OFF")
    }

    pub fn set_shader_enabled(&self, id: &str, file: &str, enabled: bool) -> AppResult<bool> {
        util::validate_component(file)?;
        let target = self.shaderpacks_dir(id)?.join(file);
        if !target.exists() {
            return Err(AppError::Generic(format!(
                "Shader pack '{file}' does not exist in this instance"
            )));
        }
        self.write_shader_pack(id, if enabled { file } else { "OFF" })?;
        Ok(true)
    }

    pub fn import_shader(&self, id: &str, source: &Path) -> AppResult<String> {
        let dir = self.shaderpacks_dir(id)?;
        let is_dir = source.is_dir();
        if !is_dir && !source.is_file() {
            return Err(AppError::Generic(format!(
                "Shader source does not exist: {}",
                source.display()
            )));
        }
        if !is_dir
            && !source
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".zip")
        {
            return Err(AppError::Generic(
                "Shader packs must be a .zip file or a folder".into(),
            ));
        }
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .ok_or_else(|| AppError::Generic("Cannot determine the shader pack name".into()))?;
        util::validate_component(&name)?;
        if is_dir {
            if !source.join("shaders").is_dir() {
                return Err(AppError::Generic(
                    "Folder has no shaders directory and is not a shader pack".into(),
                ));
            }
        } else if !zip_has_shaders(source)? {
            return Err(AppError::Generic(
                "Zip has no shaders directory and is not a shader pack".into(),
            ));
        }
        fs::create_dir_all(&dir)?;
        let target = dir.join(&name);
        if target.exists() {
            return Err(AppError::Generic(format!(
                "Shader pack '{name}' already exists"
            )));
        }
        if is_dir {
            copy_tree(source, &target)?;
        } else {
            fs::copy(source, &target)?;
        }
        Ok(name)
    }

    pub fn delete_shader(&self, id: &str, file: &str) -> AppResult<u64> {
        util::validate_component(file)?;
        let target = self.shaderpacks_dir(id)?.join(file);
        if !target.exists() {
            return Err(AppError::Generic(format!(
                "Shader pack '{file}' does not exist in this instance"
            )));
        }
        let size = if target.is_dir() {
            dir_size(&target)
        } else {
            target.metadata().map(|meta| meta.len()).unwrap_or(0)
        };
        if self.read_shader_pack(id).as_deref() == Some(file) {
            self.write_shader_pack(id, "OFF")?;
        }
        if target.is_dir() {
            fs::remove_dir_all(&target)?;
        } else {
            fs::remove_file(&target)?;
        }
        self.forget_shader_meta(id, file)?;
        Ok(size)
    }
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn zip_error(error: zip::result::ZipError) -> AppError {
    AppError::Generic(format!("Archive error: {error}"))
}

fn file_options() -> zip::write::FileOptions {
    zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644)
}

fn add_world_tree(
    zip: &mut zip::ZipWriter<fs::File>,
    source: &Path,
    relative: &Path,
    options: zip::write::FileOptions,
) -> AppResult<usize> {
    let mut added = 0;
    for entry in fs::read_dir(source)?.flatten() {
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let child = if relative.as_os_str().is_empty() {
            PathBuf::from(&name)
        } else {
            relative.join(&name)
        };
        if kind.is_dir() {
            added += add_world_tree(zip, &entry.path(), &child, options)?;
        } else if kind.is_file() {
            let entry_name = child.to_string_lossy().replace('\\', "/");
            if entry_name == BACKUP_MANIFEST {
                continue;
            }
            zip.start_file(entry_name, options).map_err(zip_error)?;
            zip.write_all(&fs::read(entry.path())?)?;
            added += 1;
        }
    }
    Ok(added)
}

fn copy_tree(src: &Path, dst: &Path) -> AppResult<u64> {
    fs::create_dir_all(dst)?;
    let mut total = 0;
    for entry in fs::read_dir(src)?.flatten() {
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        let target = dst.join(entry.file_name());
        if kind.is_dir() {
            total += copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            total += entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(total)
}

fn world_prefix(names: &[String]) -> Option<Option<String>> {
    let clean: Vec<&str> = names
        .iter()
        .map(|name| name.as_str())
        .filter(|name| {
            *name != BACKUP_MANIFEST && !name.starts_with("__MACOSX/") && !name.ends_with('/')
        })
        .collect();
    if clean.contains(&"level.dat") {
        return Some(None);
    }
    let mut prefixes: Vec<String> = clean
        .iter()
        .filter_map(|name| name.split_once('/'))
        .filter(|(_, rest)| *rest == "level.dat")
        .map(|(prefix, _)| prefix.to_string())
        .collect();
    prefixes.sort();
    prefixes.dedup();
    if prefixes.len() == 1 {
        return Some(Some(prefixes.remove(0)));
    }
    None
}

fn sanitize_world_folder(name: &str) -> String {
    let mut out: String = name
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.' | ' ' | '+') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    out = out.trim_matches(['-', '.', ' ']).to_string();
    if out.is_empty() {
        out = "world".into();
    }
    if out.chars().count() > 64 {
        out = out.chars().take(64).collect();
    }
    if util::validate_component(&out).is_err() {
        out = format!("world-{}", crate::instance::utc_stamp(SystemTime::now()));
    }
    out
}

fn unique_world_folder(saves: &Path, base: &str) -> String {
    if !saves.join(base).exists() {
        return base.to_string();
    }
    for index in 2..1000 {
        let candidate = format!("{base}-{index}");
        if !saves.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{base}-{}", now_epoch())
}

fn read_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .map(|text| text.lines().map(String::from).collect())
        .unwrap_or_default()
}

/// Like read_lines, but distinguishes a missing file from a read error so a
/// failed read never turns into an overwrite with defaults.
fn read_lines_checked(path: &Path) -> AppResult<Option<Vec<String>>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text.lines().map(String::from).collect())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::Io(error)),
    }
}

fn write_lines(path: &Path, lines: &[String]) -> AppResult<()> {
    let mut text = lines.join("\n");
    text.push('\n');
    util::atomic_write(path, text.as_bytes())?;
    Ok(())
}

fn array_value(lines: &[String], key: &str) -> Vec<String> {
    let prefix = format!("{key}:");
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|rest| serde_json::from_str(rest.trim()).ok())
        .unwrap_or_default()
}

fn scalar_value(lines: &[String], key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(|value| value.trim().to_string())
}

fn set_scalar(lines: &mut Vec<String>, key: &str, value: &str) {
    let line = format!("{key}:{value}");
    let prefix = format!("{key}:");
    if let Some(index) = lines.iter().position(|line| line.starts_with(&prefix)) {
        lines[index] = line;
    } else {
        lines.push(line);
    }
}

fn set_array(lines: &mut Vec<String>, key: &str, values: &[String]) {
    let line = format!(
        "{key}:{}",
        serde_json::to_string(values).unwrap_or_else(|_| "[]".into())
    );
    let prefix = format!("{key}:");
    if let Some(index) = lines.iter().position(|line| line.starts_with(&prefix)) {
        lines[index] = line;
    } else {
        lines.push(line);
    }
}

fn read_level_info(path: &Path) -> Option<LevelInfo> {
    let bytes = fs::read(path).ok()?;
    level_info_from_bytes(&bytes)
}

fn level_info_from_bytes(bytes: &[u8]) -> Option<LevelInfo> {
    let raw = gunzip(bytes).unwrap_or_else(|| bytes.to_vec());
    let root = parse_nbt_root(&raw)?;
    let data = root.get("Data")?;
    Some(LevelInfo {
        name: data
            .get("LevelName")
            .and_then(Nbt::as_str)
            .map(String::from)
            .filter(|name| !name.trim().is_empty()),
        version: data
            .get("Version")
            .and_then(|version| version.get("Name"))
            .and_then(Nbt::as_str)
            .map(String::from),
        game_type: data
            .get("GameType")
            .and_then(Nbt::as_i32)
            .map(|value| value as u8),
        hardcore: data
            .get("hardcore")
            .and_then(Nbt::as_i8)
            .is_some_and(|value| value != 0),
    })
}

fn gunzip(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 2 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return None;
    }
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut output = Vec::new();
    decoder.read_to_end(&mut output).ok()?;
    Some(output)
}

fn read_pack_meta(path: &Path, is_dir: bool) -> Option<PackMeta> {
    let text = if is_dir {
        fs::read_to_string(path.join("pack.mcmeta")).ok()?
    } else {
        let file = fs::File::open(path).ok()?;
        let mut archive = zip::ZipArchive::new(file).ok()?;
        let mut target = None;
        for index in 0..archive.len() {
            if let Ok(entry) = archive.by_index(index) {
                if entry.name().eq_ignore_ascii_case("pack.mcmeta") {
                    target = Some(entry.name().to_string());
                }
            }
        }
        let mut entry = archive.by_name(&target?).ok()?;
        let mut text = String::new();
        entry.read_to_string(&mut text).ok()?;
        text
    };
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let pack = value.get("pack")?;
    let format = pack
        .get("pack_format")
        .and_then(|value| value.as_u64())
        .map(|value| value as u32)
        .or_else(|| {
            pack.get("supported_formats")
                .and_then(|value| value.as_array())
                .and_then(|formats| formats.first())
                .and_then(|value| value.as_u64())
                .map(|value| value as u32)
        });
    let description = pack
        .get("description")
        .map(flatten_text)
        .unwrap_or_default();
    Some(PackMeta {
        format,
        description: strip_colors(&description),
    })
}

fn zip_has_pack_meta(path: &Path) -> AppResult<bool> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
    for index in 0..archive.len() {
        if let Ok(entry) = archive.by_index(index) {
            if entry.name().eq_ignore_ascii_case("pack.mcmeta") {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn zip_has_shaders(path: &Path) -> AppResult<bool> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
    for index in 0..archive.len() {
        if let Ok(entry) = archive.by_index(index) {
            if entry.name().starts_with("shaders/") {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn flatten_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(items) => items.iter().map(flatten_text).collect(),
        serde_json::Value::Object(map) => {
            if let Some(text) = map.get("text") {
                return flatten_text(text);
            }
            if let Some(key) = map.get("translate").and_then(|value| value.as_str()) {
                return key.to_string();
            }
            String::new()
        }
        _ => String::new(),
    }
}

fn strip_colors(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut skip = false;
    for ch in text.chars() {
        if skip {
            skip = false;
            continue;
        }
        if ch == '§' {
            skip = true;
            continue;
        }
        output.push(ch);
    }
    output.trim().to_string()
}

// ── minimal NBT reader (level.dat) ────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Nbt {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    List(Vec<Nbt>),
    Compound(BTreeMap<String, Nbt>),
    ByteArray(Vec<u8>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Nbt {
    fn get(&self, key: &str) -> Option<&Nbt> {
        match self {
            Nbt::Compound(map) => map.get(key),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Nbt::String(value) => Some(value),
            _ => None,
        }
    }

    fn as_i8(&self) -> Option<i8> {
        match self {
            Nbt::Byte(value) => Some(*value),
            _ => None,
        }
    }

    fn as_i32(&self) -> Option<i32> {
        match self {
            Nbt::Byte(value) => Some(*value as i32),
            Nbt::Short(value) => Some(*value as i32),
            Nbt::Int(value) => Some(*value),
            Nbt::Long(value) => i32::try_from(*value).ok(),
            _ => None,
        }
    }
}

struct NbtReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> NbtReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(len)?;
        if end > self.data.len() {
            return None;
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn i16(&mut self) -> Option<i16> {
        self.take(2)
            .and_then(|bytes| bytes.try_into().ok())
            .map(i16::from_be_bytes)
    }

    fn u16(&mut self) -> Option<u16> {
        self.take(2)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u16::from_be_bytes)
    }

    fn i32(&mut self) -> Option<i32> {
        self.take(4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(i32::from_be_bytes)
    }

    fn i64(&mut self) -> Option<i64> {
        self.take(8)
            .and_then(|bytes| bytes.try_into().ok())
            .map(i64::from_be_bytes)
    }

    fn f32(&mut self) -> Option<f32> {
        self.take(4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(f32::from_be_bytes)
    }

    fn f64(&mut self) -> Option<f64> {
        self.take(8)
            .and_then(|bytes| bytes.try_into().ok())
            .map(f64::from_be_bytes)
    }

    fn string(&mut self) -> Option<String> {
        let len = self.u16()? as usize;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes).ok().map(String::from)
    }
}

fn parse_nbt_root(data: &[u8]) -> Option<Nbt> {
    let mut reader = NbtReader::new(data);
    let tag = reader.u8()?;
    if tag != 10 {
        return None;
    }
    let _name = reader.string()?;
    parse_nbt_payload(&mut reader, tag, 0)
}

fn parse_nbt_payload(reader: &mut NbtReader<'_>, tag: u8, depth: usize) -> Option<Nbt> {
    if depth > 64 {
        return None;
    }
    match tag {
        1 => Some(Nbt::Byte(reader.u8()? as i8)),
        2 => Some(Nbt::Short(reader.i16()?)),
        3 => Some(Nbt::Int(reader.i32()?)),
        4 => Some(Nbt::Long(reader.i64()?)),
        5 => Some(Nbt::Float(reader.f32()?)),
        6 => Some(Nbt::Double(reader.f64()?)),
        7 => {
            let len = reader.i32()?;
            if len < 0 {
                return None;
            }
            Some(Nbt::ByteArray(reader.take(len as usize)?.to_vec()))
        }
        8 => Some(Nbt::String(reader.string()?)),
        9 => {
            let inner = reader.u8()?;
            let len = reader.i32()?;
            if !(0..=1_000_000).contains(&len) {
                return None;
            }
            let mut items = Vec::with_capacity((len as usize).min(1024));
            for _ in 0..len {
                items.push(parse_nbt_payload(reader, inner, depth + 1)?);
            }
            Some(Nbt::List(items))
        }
        10 => {
            let mut map = BTreeMap::new();
            loop {
                let inner = reader.u8()?;
                if inner == 0 {
                    break;
                }
                let name = reader.string()?;
                let value = parse_nbt_payload(reader, inner, depth + 1)?;
                map.insert(name, value);
            }
            Some(Nbt::Compound(map))
        }
        11 => {
            let len = reader.i32()?;
            if !(0..=1_000_000).contains(&len) {
                return None;
            }
            let mut items = Vec::with_capacity((len as usize).min(1024));
            for _ in 0..len {
                items.push(reader.i32()?);
            }
            Some(Nbt::IntArray(items))
        }
        12 => {
            let len = reader.i32()?;
            if !(0..=1_000_000).contains(&len) {
                return None;
            }
            let mut items = Vec::with_capacity((len as usize).min(1024));
            for _ in 0..len {
                items.push(reader.i64()?);
            }
            Some(Nbt::LongArray(items))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be16(value: u16) -> Vec<u8> {
        value.to_be_bytes().to_vec()
    }

    fn be32(value: i32) -> Vec<u8> {
        value.to_be_bytes().to_vec()
    }

    fn nbt_string(value: &str) -> Vec<u8> {
        let mut out = be16(value.len() as u16);
        out.extend_from_slice(value.as_bytes());
        out
    }

    fn named(tag: u8, name: &str, payload: Vec<u8>) -> Vec<u8> {
        let mut out = vec![tag];
        out.extend(nbt_string(name));
        out.extend(payload);
        out
    }

    fn compound(entries: Vec<Vec<u8>>) -> Vec<u8> {
        let mut out = Vec::new();
        for entry in entries {
            out.extend(entry);
        }
        out.push(0);
        out
    }

    fn level_dat(name: &str, version: &str, game_type: i32, hardcore: bool) -> Vec<u8> {
        let data = compound(vec![
            named(8, "LevelName", nbt_string(name)),
            named(3, "GameType", be32(game_type)),
            named(1, "hardcore", vec![u8::from(hardcore)]),
            named(
                10,
                "Version",
                compound(vec![named(8, "Name", nbt_string(version))]),
            ),
        ]);
        named(10, "", compound(vec![named(10, "Data", data)]))
    }

    fn write_pack_zip(path: &Path, format: u32, description: &str) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = file_options();
        zip.start_file("pack.mcmeta", options).unwrap();
        let meta = serde_json::json!({
            "pack": {"pack_format": format, "description": description}
        });
        zip.write_all(meta.to_string().as_bytes()).unwrap();
        zip.start_file("assets/test/lang/en_us.json", options)
            .unwrap();
        zip.write_all(b"{}").unwrap();
        zip.finish().unwrap();
    }

    fn instance(root: &Path, id: &str) -> PathBuf {
        let dir = root.join("versions").join(id);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn world(root: &Path, id: &str, folder: &str) -> PathBuf {
        let dir = instance(root, id).join(SAVES_DIR).join(folder);
        fs::create_dir_all(dir.join("region")).unwrap();
        fs::write(
            dir.join("level.dat"),
            level_dat("Test World", "1.21.4", 1, true),
        )
        .unwrap();
        fs::write(dir.join("icon.png"), b"png").unwrap();
        fs::write(dir.join("region").join("r.0.0.mca"), vec![0u8; 64]).unwrap();
        dir
    }

    #[test]
    fn level_dat_metadata_is_parsed_from_raw_and_gzip_nbt() {
        let raw = level_dat("Test World", "1.21.4", 1, true);
        let info = level_info_from_bytes(&raw).unwrap();
        assert_eq!(info.name.as_deref(), Some("Test World"));
        assert_eq!(info.version.as_deref(), Some("1.21.4"));
        assert_eq!(info.game_type, Some(1));
        assert!(info.hardcore);

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).unwrap();
        let gz = encoder.finish().unwrap();
        let info = level_info_from_bytes(&gz).unwrap();
        assert_eq!(info.name.as_deref(), Some("Test World"));
        assert!(level_info_from_bytes(b"not nbt").is_none());
        assert_eq!(game_mode_label(1, false), "creative");
        assert_eq!(game_mode_label(0, true), "生存");
    }

    #[test]
    fn saves_are_listed_with_metadata_and_can_be_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        world(root, "1.21.4", "Test World");
        fs::create_dir_all(instance(root, "1.21.4").join(SAVES_DIR).join("Empty")).unwrap();
        let manager = ContentManager::new(root);
        let saves = manager.list_saves("1.21.4");
        assert_eq!(saves.len(), 1, "directories without level.dat are ignored");
        let save = &saves[0];
        assert_eq!(save.folder, "Test World");
        assert_eq!(save.name, "Test World");
        assert_eq!(save.version.as_deref(), Some("1.21.4"));
        assert_eq!(save.game_type, Some(1));
        assert!(save.hardcore);
        assert!(save.has_icon);
        assert!(save.size >= 65);
        assert!(save.modified_unix().is_some());

        let freed = manager.delete_save("1.21.4", "Test World").unwrap();
        assert!(freed >= 65);
        assert!(!root.join("versions/1.21.4/saves/Test World").exists());
        assert!(manager.delete_save("1.21.4", "Test World").is_err());
        assert!(manager.delete_save("1.21.4", "../escape").is_err());
    }

    #[test]
    fn world_backup_and_import_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        world(root, "1.21.4", "Test World");
        let manager = ContentManager::new(root);
        let archive = root.join("out").join("world.zip");
        let path = manager
            .backup_save("1.21.4", "Test World", Some(&archive))
            .unwrap();
        assert!(path.is_file());

        manager.delete_save("1.21.4", "Test World").unwrap();
        let folder = manager.import_save("1.21.4", &archive, None).unwrap();
        assert_eq!(folder, "Test World");
        let restored = root.join("versions/1.21.4/saves/Test World");
        assert!(restored.join("level.dat").is_file());
        assert!(restored.join("region/r.0.0.mca").is_file());
        assert!(restored.join("icon.png").is_file());

        let second = manager.import_save("1.21.4", &archive, None).unwrap();
        assert_eq!(second, "Test World-2");
        assert!(root
            .join("versions/1.21.4/saves/Test World-2/level.dat")
            .is_file());

        let named = manager
            .import_save("1.21.4", &archive, Some("Imported"))
            .unwrap();
        assert_eq!(named, "Imported");

        assert!(manager.backup_save("1.21.4", "missing", None).is_err());
        assert!(manager.backup_save("1.21.4", "../escape", None).is_err());
    }

    #[test]
    fn import_accepts_a_vanilla_world_zip_and_rejects_non_worlds() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        instance(root, "1.21.4");
        let manager = ContentManager::new(root);
        let archive = root.join("vanilla.zip");
        let file = fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = file_options();
        zip.start_file("level.dat", options).unwrap();
        zip.write_all(&level_dat("Vanilla World", "1.20.1", 0, false))
            .unwrap();
        zip.start_file("region/r.0.0.mca", options).unwrap();
        zip.write_all(&[0u8; 8]).unwrap();
        zip.finish().unwrap();

        let folder = manager
            .import_save("1.21.4", &archive, Some("Vanilla"))
            .unwrap();
        assert_eq!(folder, "Vanilla");
        assert!(root
            .join("versions/1.21.4/saves/Vanilla/region/r.0.0.mca")
            .is_file());

        let bad = root.join("bad.zip");
        let file = fs::File::create(&bad).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("readme.txt", file_options()).unwrap();
        zip.write_all(b"nothing").unwrap();
        zip.finish().unwrap();
        assert!(manager.import_save("1.21.4", &bad, None).is_err());
        assert!(!root.join("versions/1.21.4/saves/bad").exists());
    }

    #[test]
    fn resource_packs_follow_options_txt_and_survive_edits() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let dir = instance(root, "1.21.4").join(RESOURCEPACKS_DIR);
        fs::create_dir_all(&dir).unwrap();
        write_pack_zip(&dir.join("Test.zip"), 15, "A §atest pack");
        fs::write(
            instance(root, "1.21.4").join(OPTIONS_FILE),
            "renderDistance:12\nresourcePacks:[\"vanilla\",\"file/Test.zip\"]\nincompatibleResourcePacks:[]\n",
        )
        .unwrap();

        let manager = ContentManager::new(root);
        let packs = manager.list_resourcepacks("1.21.4");
        assert_eq!(packs.len(), 1);
        assert!(packs[0].enabled);
        assert_eq!(packs[0].pack_format, Some(15));
        assert_eq!(packs[0].description, "A test pack");
        assert!(packs[0].valid);
        assert!(!packs[0].is_dir);

        assert!(manager
            .set_resourcepack_enabled("1.21.4", "Test.zip", false)
            .unwrap());
        let options = fs::read_to_string(instance(root, "1.21.4").join(OPTIONS_FILE)).unwrap();
        assert!(options.contains("renderDistance:12"));
        assert!(!options.contains("file/Test.zip"));
        assert!(!manager.list_resourcepacks("1.21.4")[0].enabled);

        fs::write(
            instance(root, "1.21.4").join(OPTIONS_FILE),
            "resourcePacks:[\"vanilla\"]\nincompatibleResourcePacks:[\"file/Test.zip\"]\n",
        )
        .unwrap();
        assert!(manager
            .set_resourcepack_enabled("1.21.4", "Test.zip", true)
            .unwrap());
        let options = fs::read_to_string(instance(root, "1.21.4").join(OPTIONS_FILE)).unwrap();
        assert!(options.contains("resourcePacks:[\"file/Test.zip\",\"vanilla\"]"));
        assert!(!options.contains("incompatibleResourcePacks:[\"file/Test.zip\"]"));
        assert!(manager.list_resourcepacks("1.21.4")[0].enabled);

        let freed = manager.delete_resourcepack("1.21.4", "Test.zip").unwrap();
        assert!(freed > 0);
        assert!(!dir.join("Test.zip").exists());
        let options = fs::read_to_string(instance(root, "1.21.4").join(OPTIONS_FILE)).unwrap();
        assert!(!options.contains("file/Test.zip"));
        assert!(manager.delete_resourcepack("1.21.4", "Test.zip").is_err());
        assert!(manager
            .set_resourcepack_enabled("1.21.4", "../escape", true)
            .is_err());
    }

    #[test]
    fn resource_pack_import_validates_archives_and_folders() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        instance(root, "1.21.4");
        let manager = ContentManager::new(root);
        let valid = root.join("valid.zip");
        write_pack_zip(&valid, 22, "Valid");
        assert_eq!(
            manager.import_resourcepack("1.21.4", &valid).unwrap(),
            "valid.zip"
        );
        assert!(root
            .join("versions/1.21.4/resourcepacks/valid.zip")
            .is_file());
        assert!(manager.import_resourcepack("1.21.4", &valid).is_err());

        let invalid = root.join("invalid.zip");
        let file = fs::File::create(&invalid).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("readme.txt", file_options()).unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        assert!(manager.import_resourcepack("1.21.4", &invalid).is_err());

        let folder = root.join("Folder Pack");
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("pack.mcmeta"),
            r#"{"pack":{"pack_format":34,"description":{"text":"Folder"}}}"#,
        )
        .unwrap();
        assert_eq!(
            manager.import_resourcepack("1.21.4", &folder).unwrap(),
            "Folder Pack"
        );
        let packs = manager.list_resourcepacks("1.21.4");
        assert!(packs
            .iter()
            .any(|pack| pack.file == "Folder Pack" && pack.is_dir));
        assert!(packs
            .iter()
            .find(|pack| pack.file == "Folder Pack")
            .is_some_and(|pack| pack.pack_format == Some(34)));
    }

    #[test]
    fn options_helpers_round_trip_arrays_without_losing_keys() {
        let mut lines = vec![
            "renderDistance:12".to_string(),
            "resourcePacks:[\"vanilla\"]".to_string(),
        ];
        assert_eq!(
            array_value(&lines, "resourcePacks"),
            vec!["vanilla".to_string()]
        );
        set_array(
            &mut lines,
            "resourcePacks",
            &["file/a.zip".to_string(), "vanilla".to_string()],
        );
        assert!(lines.contains(&"renderDistance:12".to_string()));
        assert_eq!(
            array_value(&lines, "resourcePacks"),
            vec!["file/a.zip".to_string(), "vanilla".to_string()]
        );
        assert!(array_value(&lines, "missing").is_empty());
        assert_eq!(sanitize_world_folder("  My:World  "), "My-World");
        assert_eq!(sanitize_world_folder(""), "world");
    }

    fn write_shader_zip(path: &Path) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = file_options();
        zip.start_file("shaders/shaders.properties", options)
            .unwrap();
        zip.write_all(b"profile.MEDIUM=\n").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn shader_packs_track_options_txt_and_recorded_loaders() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let dir = instance(root, "1.21.4").join(SHADERPACKS_DIR);
        fs::create_dir_all(&dir).unwrap();
        write_shader_zip(&dir.join("TestShader.zip"));
        fs::write(
            instance(root, "1.21.4").join(OPTIONS_FILE),
            "renderDistance:12\nshaderPack:TestShader.zip\n",
        )
        .unwrap();

        let manager = ContentManager::new(root);
        manager
            .record_shader_meta(
                "1.21.4",
                "TestShader.zip",
                ShaderSupport {
                    iris: true,
                    optifine: true,
                },
                Some("Test Shader"),
                Some("test-shader"),
            )
            .unwrap();
        let shaders = manager.list_shaders("1.21.4");
        assert_eq!(shaders.len(), 1);
        assert!(shaders[0].enabled);
        assert_eq!(shaders[0].title.as_deref(), Some("Test Shader"));
        assert_eq!(
            shaders[0].support,
            Some(ShaderSupport {
                iris: true,
                optifine: true
            })
        );
        assert!(shaders[0].size > 0);

        assert!(manager
            .set_shader_enabled("1.21.4", "TestShader.zip", false)
            .unwrap());
        let options = fs::read_to_string(instance(root, "1.21.4").join(OPTIONS_FILE)).unwrap();
        assert!(options.contains("shaderPack:OFF"));
        assert!(options.contains("renderDistance:12"));
        assert!(!manager.list_shaders("1.21.4")[0].enabled);

        manager.disable_shader("1.21.4").unwrap();
        assert!(manager
            .set_shader_enabled("1.21.4", "TestShader.zip", true)
            .unwrap());
        assert!(manager.list_shaders("1.21.4")[0].enabled);

        let freed = manager.delete_shader("1.21.4", "TestShader.zip").unwrap();
        assert!(freed > 0);
        assert!(!dir.join("TestShader.zip").exists());
        assert!(manager.list_shaders("1.21.4").is_empty());
        let options = fs::read_to_string(instance(root, "1.21.4").join(OPTIONS_FILE)).unwrap();
        assert!(options.contains("shaderPack:OFF"));
        assert!(manager.delete_shader("1.21.4", "TestShader.zip").is_err());
        assert!(manager
            .set_shader_enabled("1.21.4", "../escape", true)
            .is_err());
    }

    #[test]
    fn shader_loader_status_detects_iris_and_optifine() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let dir = instance(root, "1.21.4");
        let manager = ContentManager::new(root);
        assert!(!manager.shader_loader_status("1.21.4").iris);
        assert!(!manager.shader_loader_status("1.21.4").optifine);

        fs::create_dir_all(dir.join("mods")).unwrap();
        fs::write(dir.join("mods").join("iris-fabric-1.8.8.jar"), b"jar").unwrap();
        let status = manager.shader_loader_status("1.21.4");
        assert!(status.iris);
        assert!(!status.optifine);

        fs::write(dir.join("optionsof.txt"), b"shaderPack:OFF\n").unwrap();
        assert!(manager.shader_loader_status("1.21.4").optifine);
        fs::write(dir.join("mods").join("OptiFine_1.21.4.jar"), b"jar").unwrap();
        assert!(manager.shader_loader_status("1.21.4").optifine);
    }

    #[test]
    fn shader_import_requires_a_shaders_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        instance(root, "1.21.4");
        let manager = ContentManager::new(root);

        let valid = root.join("valid-shader.zip");
        write_shader_zip(&valid);
        assert_eq!(
            manager.import_shader("1.21.4", &valid).unwrap(),
            "valid-shader.zip"
        );
        assert!(root
            .join("versions/1.21.4/shaderpacks/valid-shader.zip")
            .is_file());
        assert!(manager.import_shader("1.21.4", &valid).is_err());

        let invalid = root.join("invalid.zip");
        let file = fs::File::create(&invalid).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("readme.txt", file_options()).unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        assert!(manager.import_shader("1.21.4", &invalid).is_err());

        let folder = root.join("Folder Shader");
        fs::create_dir_all(folder.join("shaders")).unwrap();
        fs::write(folder.join("shaders/shaders.properties"), b"profile.LOW=\n").unwrap();
        assert_eq!(
            manager.import_shader("1.21.4", &folder).unwrap(),
            "Folder Shader"
        );
        assert!(manager
            .list_shaders("1.21.4")
            .iter()
            .any(|shader| shader.file == "Folder Shader" && shader.is_dir));
    }
}
