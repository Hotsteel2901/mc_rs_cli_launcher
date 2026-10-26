//! Maintenance commands: environment diagnostics, cache pruning and repair.
//!
//! These are deliberately read-only until the caller passes `--yes`, and they
//! never touch credentials: a broken keychain must not make `doctor` hang.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::instance::InstanceManager;
use crate::mod_manager::ModManager;
use crate::version::VersionManager;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Fail,
    Skip,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
            Self::Skip => "SKIP",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::Skip => "skip",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub detail: String,
    pub hint: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    fn push(&mut self, name: &str, status: Status, detail: impl Into<String>) {
        self.checks.push(Check {
            name: name.to_string(),
            status,
            detail: detail.into(),
            hint: None,
        });
    }

    fn push_hint(
        &mut self,
        name: &str,
        status: Status,
        detail: impl Into<String>,
        hint: impl Into<String>,
    ) {
        self.checks.push(Check {
            name: name.to_string(),
            status,
            detail: detail.into(),
            hint: Some(hint.into()),
        });
    }

    pub fn failures(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.status == Status::Fail)
            .count()
    }

    pub fn warnings(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.status == Status::Warn)
            .count()
    }
}

/// Inspect the environment without downloading, installing or logging in.
pub fn doctor(game_dir: &Path) -> Report {
    let mut report = Report::default();

    report.push(
        "Platform",
        Status::Ok,
        format!(
            "{} / {} / kernel {}",
            crate::util::os_name(),
            crate::util::os_arch(),
            crate::util::os_version()
        ),
    );

    report.push("Game directory", Status::Ok, game_dir.display().to_string());
    match writable(game_dir) {
        Ok(()) => report.push("Directory writable", Status::Ok, "yes"),
        Err(error) => report.push_hint(
            "Directory writable",
            Status::Fail,
            error,
            "Check permissions or choose another --dir",
        ),
    }

    // Account metadata only. Never touch the keychain from a diagnostic.
    match crate::account::AccountManager::public_metadata(game_dir) {
        Ok(data) => {
            let key = data["default"].as_str().unwrap_or("");
            match data["accounts"].get(key) {
                Some(account) => report.push(
                    "Account",
                    Status::Ok,
                    format!(
                        "{} ({})",
                        account["username"].as_str().unwrap_or("?"),
                        account["type"].as_str().unwrap_or("?")
                    ),
                ),
                None => report.push_hint(
                    "Account",
                    Status::Warn,
                    "no saved account",
                    "Run: hot-rs-launcher login  (or: offline <name>)",
                ),
            }
        }
        Err(error) => report.push_hint(
            "Account",
            Status::Fail,
            format!("cannot read launcher_accounts.json: {error}"),
            "Fix or move the file; it is never overwritten automatically",
        ),
    }

    let instances = InstanceManager::new(game_dir).list();
    let broken = instances
        .iter()
        .filter(|detail| {
            let jar = game_dir
                .join("versions")
                .join(&detail.id)
                .join(format!("{}.jar", detail.id));
            !jar.is_file()
        })
        .map(|detail| detail.id.clone())
        .collect::<Vec<_>>();
    if instances.is_empty() {
        report.push_hint(
            "Instances",
            Status::Warn,
            "none installed",
            "Run: hot-rs-launcher download -v 1.21.4",
        );
    } else if broken.is_empty() {
        report.push(
            "Instances",
            Status::Ok,
            format!("{} installed, client jars present", instances.len()),
        );
    } else {
        report.push_hint(
            "Instances",
            Status::Warn,
            format!(
                "{} installed; missing client jar: {}",
                instances.len(),
                broken.join(", ")
            ),
            "Run: hot-rs-launcher repair --yes",
        );
    }

    let javas = crate::java::detect_java(Some(game_dir));
    if javas.is_empty() {
        report.push_hint(
            "Java",
            Status::Warn,
            "no runtime detected",
            "Run: hot-rs-launcher install-java --java-major 21",
        );
    } else {
        let newest = javas.iter().find_map(|info| info.major);
        report.push(
            "Java",
            Status::Ok,
            format!(
                "{} runtime(s); newest Java {}",
                javas.len(),
                newest
                    .map(|major| major.to_string())
                    .unwrap_or_else(|| "?".into())
            ),
        );
        if let Some(required) = required_java(game_dir).first().copied() {
            match crate::java::best_java(&javas, Some(required)) {
                Some(info) if info.major == Some(required) => report.push(
                    "Java requirement",
                    Status::Ok,
                    format!("Java {required} available at {}", info.path),
                ),
                Some(info) => report.push_hint(
                    "Java requirement",
                    Status::Warn,
                    format!(
                        "no exact Java {required}; closest is Java {}",
                        info.major
                            .map(|major| major.to_string())
                            .unwrap_or_else(|| "?".into())
                    ),
                    "Install the exact major for modded instances",
                ),
                None => report.push_hint(
                    "Java requirement",
                    Status::Warn,
                    format!("no Java {required} found"),
                    "Run: hot-rs-launcher install-java --java-major <major>",
                ),
            }
        }
    }

    // `remove-version --keep-saves` intentionally leaves saves/ behind. They
    // are invisible to `list-installed`, so surface them here instead of
    // letting the user think the worlds vanished.
    let mut orphans = Vec::new();
    if let Ok(entries) = fs::read_dir(game_dir.join("versions")) {
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if path.is_dir()
                && !path.join(format!("{id}.json")).is_file()
                && path.join("saves").is_dir()
            {
                orphans.push(id);
            }
        }
    }
    if !orphans.is_empty() {
        report.push_hint(
            "Orphaned saves",
            Status::Warn,
            format!(
                "{} folder(s) kept without version metadata: {}",
                orphans.len(),
                orphans.join(", ")
            ),
            "Restore them by importing a backup, or move versions/<id>/saves manually",
        );
    }

    match crate::http::source_mode() {
        crate::http::SourceMode::Official => {
            report.push("Download source", Status::Ok, "official (forced)")
        }
        crate::http::SourceMode::BmclApi => {
            report.push("Download source", Status::Ok, "BMCLAPI (forced)")
        }
        crate::http::SourceMode::Auto => report.push(
            "Download source",
            Status::Ok,
            format!(
                "auto (proxy: {})",
                std::env::var("HTTPS_PROXY")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "none".into())
            ),
        ),
    }

    if std::env::var("MC_LAUNCHER_NO_NETWORK").is_ok() {
        report.push("Network", Status::Skip, "skipped by MC_LAUNCHER_NO_NETWORK");
    } else {
        match crate::http::probe_url(
            "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json",
            6,
        ) {
            Ok(status) if (200..300).contains(&status) => report.push(
                "Network",
                Status::Ok,
                format!("Mojang metadata HTTP {status}"),
            ),
            Ok(status) => report.push_hint(
                "Network",
                Status::Warn,
                format!("Mojang metadata HTTP {status}"),
                "Check the mirror/proxy settings; cached versions still launch offline",
            ),
            Err(error) => report.push_hint(
                "Network",
                Status::Warn,
                error,
                "Offline launches still work when all files are cached",
            ),
        }
    }

    let backup_dir = game_dir.join("backups");
    if backup_dir.is_dir() {
        let count = fs::read_dir(&backup_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "zip"))
                    .count()
            })
            .unwrap_or(0);
        report.push("Backups", Status::Ok, format!("{count} archive(s)"));
    }

    if let Ok(metadata) = fs::metadata(game_dir.join(".launcher_verify.json")) {
        report.push(
            "Verification cache",
            Status::Ok,
            format!("{} KiB", metadata.len() / 1024),
        );
    }

    report
}

fn writable(game_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(game_dir)
        .map_err(|error| format!("cannot create {}: {error}", game_dir.display()))?;
    let probe = game_dir.join(format!(".doctor-{}", std::process::id()));
    fs::write(&probe, b"ok")
        .map_err(|error| format!("cannot write to {}: {error}", game_dir.display()))?;
    let _ = fs::remove_file(&probe);
    Ok(())
}

/// Java majors required by the newest installed instances (at most three).
fn required_java(game_dir: &Path) -> Vec<u32> {
    let mut majors = Vec::new();
    for id in ModManager::list_installed_versions(game_dir)
        .into_iter()
        .take(3)
    {
        let path = game_dir
            .join("versions")
            .join(&id)
            .join(format!("{id}.json"));
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(major) = value["javaVersion"]["majorVersion"].as_u64() {
                    majors.push(major as u32);
                }
            }
        }
    }
    majors.sort_unstable();
    majors.dedup();
    majors
}

// ─── pruning ───────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct PrunePlan {
    pub libraries: Vec<PathBuf>,
    pub assets: Vec<PathBuf>,
    pub bytes: u64,
    /// Instances whose metadata could not be parsed. Pruning is refused when
    /// this is non-empty: deleting a library we cannot prove is unused could
    /// break a working instance.
    pub unreadable: Vec<String>,
}

impl PrunePlan {
    pub fn is_empty(&self) -> bool {
        self.libraries.is_empty() && self.assets.is_empty()
    }
}

/// Compare the on-disk caches with everything the installed instances declare.
pub fn plan_prune(game_dir: &Path) -> AppResult<PrunePlan> {
    let manager = VersionManager::new(game_dir);
    let mut keep_libraries = BTreeSet::new();
    let mut keep_assets = BTreeSet::new();
    let mut unreadable = Vec::new();
    let versions_dir = game_dir.join("versions");
    let mut any = false;

    if let Ok(entries) = fs::read_dir(&versions_dir) {
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().to_string();
            let json = entry.path().join(format!("{id}.json"));
            if !json.is_file() {
                continue;
            }
            any = true;
            let data = match fs::read(&json)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            {
                Some(data) => data,
                None => {
                    unreadable.push(id);
                    continue;
                }
            };
            match manager.planned_files(&data) {
                Ok(paths) => keep_libraries.extend(paths),
                Err(error) => {
                    unreadable.push(format!("{id}: {error}"));
                    continue;
                }
            }
            if let Some(index_id) = data["assetIndex"]["id"].as_str() {
                let index_path = game_dir
                    .join("assets")
                    .join("indexes")
                    .join(format!("{index_id}.json"));
                if let Ok(bytes) = fs::read(&index_path) {
                    if let Ok(index) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        if let Some(objects) = index["objects"].as_object() {
                            for object in objects.values() {
                                if let Some(hash) = object["hash"].as_str() {
                                    if hash.len() >= 2 {
                                        keep_assets.insert(
                                            game_dir
                                                .join("assets")
                                                .join("objects")
                                                .join(&hash[..2])
                                                .join(hash),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if !any {
        return Ok(PrunePlan::default());
    }
    if !unreadable.is_empty() {
        return Ok(PrunePlan {
            unreadable,
            ..PrunePlan::default()
        });
    }

    let mut plan = PrunePlan::default();
    collect_unused(
        &game_dir.join("libraries"),
        &keep_libraries,
        true,
        &mut plan,
    );
    collect_unused(
        &game_dir.join("assets").join("objects"),
        &keep_assets,
        false,
        &mut plan,
    );
    plan.libraries.sort();
    plan.assets.sort();
    Ok(plan)
}

/// Walk one cache root and collect regular files that no instance references.
/// `libraries` keeps JSON metadata (loader profiles) regardless of the set.
fn collect_unused(root: &Path, keep: &BTreeSet<PathBuf>, libraries: bool, plan: &mut PrunePlan) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            if libraries {
                let extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("");
                if extension.eq_ignore_ascii_case("json") {
                    continue;
                }
            }
            if keep.contains(&path) {
                continue;
            }
            let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            plan.bytes += size;
            if libraries {
                plan.libraries.push(path);
            } else {
                plan.assets.push(path);
            }
        }
    }
}

/// Delete the planned files. Returns (deleted, bytes).
pub fn apply_prune(plan: &PrunePlan) -> AppResult<(usize, u64)> {
    let mut deleted = 0;
    let mut bytes = 0;
    for path in plan.libraries.iter().chain(plan.assets.iter()) {
        let size = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
        match fs::remove_file(path) {
            Ok(()) => {
                deleted += 1;
                bytes += size;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Generic(format!(
                    "Cannot delete {}: {error}",
                    path.display()
                )))
            }
        }
    }
    Ok((deleted, bytes))
}

// ─── repair ────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct InstanceHealth {
    pub id: String,
    pub issues: Vec<String>,
}

impl InstanceHealth {
    pub fn healthy(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Check that every file an instance declares is present and non-empty.
/// This is a cheap presence check; `--deep` re-hashes instead.
pub fn inspect_instances(game_dir: &Path, deep: bool) -> Vec<InstanceHealth> {
    let manager = VersionManager::new(game_dir);
    let mut results = Vec::new();
    for id in ModManager::list_installed_versions(game_dir) {
        let mut issues = Vec::new();
        let json_path = game_dir
            .join("versions")
            .join(&id)
            .join(format!("{id}.json"));
        let data = match fs::read(&json_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        {
            Some(data) => data,
            None => {
                results.push(InstanceHealth {
                    id,
                    issues: vec!["version JSON is missing or unreadable".into()],
                });
                continue;
            }
        };
        let jar = game_dir
            .join("versions")
            .join(&id)
            .join(format!("{id}.jar"));
        if !jar.is_file() {
            issues.push("client jar missing".into());
        } else if deep {
            let expected = data["downloads"]["client"]["sha1"].as_str();
            if !crate::http::cached_file_valid(&jar, expected) {
                crate::verify::forget(&jar);
                issues.push("client jar failed checksum".into());
            }
        }
        match manager.planned_files(&data) {
            Ok(paths) => {
                for path in paths {
                    if !path.is_file() {
                        issues.push(format!("missing library {}", path.display()));
                    } else if deep && !crate::http::cached_file_valid(&path, None) {
                        crate::verify::forget(&path);
                        issues.push(format!("corrupt library {}", path.display()));
                    }
                }
            }
            Err(error) => issues.push(format!("cannot resolve libraries: {error}")),
        }
        if let Some(index_id) = data["assetIndex"]["id"].as_str() {
            let index_path = game_dir
                .join("assets")
                .join("indexes")
                .join(format!("{index_id}.json"));
            match fs::read(&index_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            {
                Some(index) => {
                    let mut missing = 0;
                    if let Some(objects) = index["objects"].as_object() {
                        for object in objects.values() {
                            let Some(hash) = object["hash"].as_str() else {
                                continue;
                            };
                            if hash.len() < 2 {
                                continue;
                            }
                            let path = game_dir
                                .join("assets")
                                .join("objects")
                                .join(&hash[..2])
                                .join(hash);
                            let ok = if deep {
                                crate::http::cached_file_valid(&path, Some(hash))
                            } else {
                                path.is_file()
                            };
                            if !ok {
                                missing += 1;
                                if missing >= 5 {
                                    break;
                                }
                            }
                        }
                    }
                    if missing > 0 {
                        issues.push(format!("{missing}+ asset object(s) missing"));
                    }
                }
                None => issues.push("asset index missing".into()),
            }
        }
        results.push(InstanceHealth { id, issues });
    }
    results
}
