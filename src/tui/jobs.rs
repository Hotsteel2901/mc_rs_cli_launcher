//! Background work for the TUI.
//!
//! Heavy operations run on a worker thread and report through one channel.
//! Self-contained CLI features that may exit or prompt (Forge-family
//! installers, Modrinth mod installs) are executed as child processes so a
//! refused prompt or a third-party crash can never take down the interface.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use crate::progress::{self, TransferEvent};
use crate::tui::config::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Ok,
    Warn,
    Error,
    Debug,
}

#[derive(Debug)]
pub enum BackendEvent {
    JobStarted {
        id: u64,
        title: String,
    },
    Phase {
        id: u64,
        index: usize,
        total: usize,
        label: String,
    },
    Log {
        id: u64,
        level: LogLevel,
        text: String,
    },
    Transfer {
        id: u64,
        transfer: TransferEvent,
    },
    Finished {
        id: u64,
        result: Result<String, String>,
    },
    Search {
        request: u64,
        query: String,
        result: Result<serde_json::Value, String>,
    },
    Manifest {
        request: u64,
        result: Result<serde_json::Value, String>,
    },
    Java {
        path: Option<String>,
        major: Option<u32>,
    },
    ModUpdates {
        request: u64,
        result: Result<Vec<crate::update::ModUpdate>, String>,
    },
}

pub type EventSender = Sender<BackendEvent>;

static ACTIVE_CHILD: AtomicU32 = AtomicU32::new(0);

struct ChildTracker;

impl ChildTracker {
    fn new(pid: u32) -> Self {
        ACTIVE_CHILD.store(pid, Ordering::SeqCst);
        Self
    }
}

impl Drop for ChildTracker {
    fn drop(&mut self) {
        ACTIVE_CHILD.store(0, Ordering::SeqCst);
    }
}

fn kill_process_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    {
        // The worker is started as its own process-group leader, so a negative
        // PID also stops the Java installer it may have spawned.
        let _ = Command::new("kill")
            .arg("-9")
            .arg(format!("-{pid}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let pid_arg = pid.to_string();
        let _ = Command::new("taskkill")
            .arg("/PID")
            .arg(pid_arg)
            .arg("/T")
            .arg("/F")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Stop a child worker and its descendants when the TUI is quitting.
pub fn kill_active_child() {
    kill_process_tree(ACTIVE_CHILD.load(Ordering::SeqCst));
}

fn send_log(tx: &EventSender, id: u64, level: LogLevel, text: impl Into<String>) {
    let _ = tx.send(BackendEvent::Log {
        id,
        level,
        text: text.into(),
    });
}

fn phase(tx: &EventSender, id: u64, index: usize, total: usize, label: impl Into<String>) {
    let _ = tx.send(BackendEvent::Phase {
        id,
        index,
        total,
        label: label.into(),
    });
}

pub fn channel() -> (EventSender, Receiver<BackendEvent>) {
    mpsc::channel()
}

/// Run a fallible in-process job, always delivering exactly one `Finished`.
fn spawn_job<F>(tx: EventSender, id: u64, title: impl Into<String>, work: F)
where
    F: FnOnce(&EventSender, u64) -> Result<String, String> + Send + 'static,
{
    let title = title.into();
    let started = title.clone();
    let _ = tx.send(BackendEvent::JobStarted { id, title: started });
    let failure_tx = tx.clone();
    let spawned = std::thread::Builder::new()
        .name("mc-tui-job".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&tx, id)))
                .unwrap_or_else(|payload| {
                    let message = payload
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("unknown internal error");
                    Err(format!("Background task panicked: {message}"))
                });
            let _ = tx.send(BackendEvent::Finished { id, result });
        });
    if let Err(error) = spawned {
        let _ = failure_tx.send(BackendEvent::Finished {
            id,
            result: Err(format!("Cannot start background task: {error}")),
        });
    }
}

/// Check all installed jars against Modrinth without blocking the UI.
pub fn start_update_check(
    tx: EventSender,
    request: u64,
    game_dir: std::path::PathBuf,
    mc_version: String,
    loader: Option<String>,
) {
    let failure_tx = tx.clone();
    let spawned = std::thread::Builder::new()
        .name("mc-tui-updates".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::update::check(&game_dir, &mc_version, loader.as_deref(), true)
                    .map(|check| check.updates)
            }))
            .unwrap_or_else(|_| Err("Update check task panicked".into()));
            let _ = tx.send(BackendEvent::ModUpdates { request, result });
        });
    if let Err(error) = spawned {
        let _ = failure_tx.send(BackendEvent::ModUpdates {
            request,
            result: Err(format!("Cannot start update check: {error}")),
        });
    }
}

/// Download and replace a batch of mods, reporting per-mod results.
pub fn start_apply_updates(
    tx: EventSender,
    id: u64,
    game_dir: std::path::PathBuf,
    mc_version: String,
    updates: Vec<crate::update::ModUpdate>,
) {
    let total = updates.len();
    spawn_job(tx, id, format!("Update {total} mod(s)"), move |tx, id| {
        let mut done = 0usize;
        let mut failures: Vec<String> = Vec::new();
        for (index, update) in updates.iter().enumerate() {
            if progress::is_cancelled() {
                return Err("Cancelled".into());
            }
            phase(
                tx,
                id,
                index + 1,
                total.max(1),
                format!("Updating {}", update.project),
            );
            match crate::update::apply(&game_dir, &mc_version, update) {
                Ok(path) => {
                    done += 1;
                    send_log(
                        tx,
                        id,
                        LogLevel::Info,
                        format!(
                            "{} -> {}",
                            update.project,
                            path.file_name()
                                .map(|name| name.to_string_lossy().to_string())
                                .unwrap_or_default()
                        ),
                    );
                }
                Err(error) => {
                    failures.push(format!("{}: {error}", update.project));
                    send_log(
                        tx,
                        id,
                        LogLevel::Warn,
                        format!("{} failed: {error}", update.project),
                    );
                }
            }
        }
        if !failures.is_empty() {
            return Err(format!(
                "Updated {done}/{total}; failed: {}",
                failures.join("; ")
            ));
        }
        Ok(format!("Updated {done} mod(s)"))
    });
}

/// Export an instance as a .mrpack (Modrinth lookups happen in the worker).
pub fn start_export_modpack(
    tx: EventSender,
    id: u64,
    game_dir: std::path::PathBuf,
    instance: String,
    path: std::path::PathBuf,
) {
    spawn_job(tx, id, format!("Export {instance}"), move |tx, id| {
        phase(tx, id, 1, 2, "Hash installed mods");
        let manager = crate::instance::InstanceManager::new(&game_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
        }
        let report = manager
            .export_modpack(&instance, None, &path)
            .map_err(|error| error.to_string())?;
        phase(tx, id, 2, 2, "Write .mrpack");
        Ok(format!(
            "Modpack exported to {} ({} download(s), {} override(s))",
            report.path.display(),
            report.files,
            report.overrides
        ))
    });
}

pub fn start_backup_save(
    tx: EventSender,
    id: u64,
    game_dir: std::path::PathBuf,
    instance: String,
    folder: String,
) {
    spawn_job(tx, id, format!("Backup world {folder}"), move |tx, id| {
        phase(tx, id, 1, 2, "Archive world files");
        let manager = crate::content::ContentManager::new(&game_dir);
        let path = manager
            .backup_save(&instance, &folder, None)
            .map_err(|error| error.to_string())?;
        phase(tx, id, 2, 2, "Finish archive");
        Ok(format!("World backup: {}", path.display()))
    });
}

pub fn start_import_save(
    tx: EventSender,
    id: u64,
    game_dir: std::path::PathBuf,
    instance: String,
    archive: std::path::PathBuf,
) {
    spawn_job(tx, id, "Import world".to_string(), move |tx, id| {
        phase(tx, id, 1, 2, "Read world archive");
        let manager = crate::content::ContentManager::new(&game_dir);
        let folder = manager
            .import_save(&instance, &archive, None)
            .map_err(|error| error.to_string())?;
        phase(tx, id, 2, 2, "Extract world files");
        Ok(format!("Imported world as '{folder}'"))
    });
}

pub fn start_import_resourcepack(
    tx: EventSender,
    id: u64,
    game_dir: std::path::PathBuf,
    instance: String,
    source: std::path::PathBuf,
) {
    spawn_job(tx, id, "Import resource pack".to_string(), move |tx, id| {
        phase(tx, id, 1, 2, "Validate pack metadata");
        let manager = crate::content::ContentManager::new(&game_dir);
        let name = manager
            .import_resourcepack(&instance, &source)
            .map_err(|error| error.to_string())?;
        phase(tx, id, 2, 2, "Copy pack files");
        Ok(format!("Imported resource pack '{name}'"))
    });
}

pub fn start_install_resourcepack(
    tx: EventSender,
    id: u64,
    game_dir: std::path::PathBuf,
    instance: String,
    slug: String,
    title: String,
) {
    spawn_job(
        tx,
        id,
        format!("Install resource pack {title}"),
        move |tx, id| {
            phase(tx, id, 1, 2, "Resolve Modrinth project");
            let report = crate::discover::install_resourcepack(&game_dir, &instance, &slug, true)
                .map_err(|error| error.to_string())?;
            phase(tx, id, 2, 2, "Enable resource pack");
            Ok(format!(
                "Installed resource pack '{}' ({})",
                report.title, report.file
            ))
        },
    );
}

pub fn start_install_shader(
    tx: EventSender,
    id: u64,
    game_dir: std::path::PathBuf,
    instance: String,
    slug: String,
    title: String,
) {
    spawn_job(tx, id, format!("Install shader {title}"), move |tx, id| {
        phase(tx, id, 1, 2, "Resolve Modrinth project");
        let report = crate::discover::install_shader(&game_dir, &instance, &slug, true)
            .map_err(|error| error.to_string())?;
        phase(tx, id, 2, 2, "Enable shader pack");
        Ok(format!(
            "Installed shader '{}' ({}) · {}",
            report.title,
            report.file,
            report.support.summary(false)
        ))
    });
}

pub fn start_import_shader(
    tx: EventSender,
    id: u64,
    game_dir: std::path::PathBuf,
    instance: String,
    source: std::path::PathBuf,
) {
    spawn_job(tx, id, "Import shader pack".to_string(), move |tx, id| {
        phase(tx, id, 1, 2, "Validate shader files");
        let manager = crate::content::ContentManager::new(&game_dir);
        let name = manager
            .import_shader(&instance, &source)
            .map_err(|error| error.to_string())?;
        phase(tx, id, 2, 2, "Copy shader files");
        Ok(format!("Imported shader pack '{name}'"))
    });
}

pub fn start_optimize_mods(
    tx: EventSender,
    id: u64,
    game_dir: std::path::PathBuf,
    settings: Settings,
    instance: String,
    loader: String,
) {
    let title = format!("Optimize {instance}");
    let _ = tx.send(BackendEvent::JobStarted {
        id,
        title: title.clone(),
    });
    let failure_tx = tx.clone();
    let spawned = std::thread::Builder::new()
        .name("mc-tui-optimize".into())
        .spawn(move || {
            let mut args = Vec::new();
            cli_base_args(&game_dir, &settings, &mut args);
            args.push("optimize-mods".into());
            args.push("-v".into());
            args.push(instance);
            args.push("--loader".into());
            args.push(loader);
            args.push("--apply".into());
            args.push("--yes".into());
            args.push("--limit".into());
            args.push("8".into());
            let result = run_child(&tx, id, &title, args);
            let _ = tx.send(BackendEvent::Finished { id, result });
        });
    if let Err(error) = spawned {
        let _ = failure_tx.send(BackendEvent::Finished {
            id,
            result: Err(format!("Cannot start optimization worker: {error}")),
        });
    }
}

pub fn start_java_probe(tx: EventSender) {
    let failure_tx = tx.clone();
    let spawned = std::thread::Builder::new()
        .name("mc-tui-java".into())
        .spawn(move || {
            let mut found_path = None;
            let mut found_major = None;
            for candidate in crate::java::find_java_candidates() {
                if let Some(major) = crate::java::java_major(&candidate) {
                    found_path = Some(candidate);
                    found_major = Some(major);
                    break;
                }
            }
            let _ = tx.send(BackendEvent::Java {
                path: found_path,
                major: found_major,
            });
        });
    if spawned.is_err() {
        let _ = failure_tx.send(BackendEvent::Java {
            path: None,
            major: None,
        });
    }
}

#[allow(clippy::too_many_arguments)]
pub fn start_search(
    tx: EventSender,
    request: u64,
    kind: crate::discover::DiscoverKind,
    query: String,
    mc_version: Option<String>,
    loader: Option<String>,
    page: u32,
    sort: String,
    category: Option<String>,
    game_dir: std::path::PathBuf,
) {
    let failure_tx = tx.clone();
    let failure_query = query.clone();
    let mark_version = mc_version.clone();
    let spawned = std::thread::Builder::new()
        .name("mc-tui-search".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> Result<serde_json::Value, String> {
                    let mut value = crate::discover::search(
                        kind,
                        &query,
                        &sort,
                        page,
                        crate::discover::PAGE_SIZE,
                        mc_version.as_deref(),
                        loader.as_deref(),
                        category.as_deref(),
                    )?;
                    if kind == crate::discover::DiscoverKind::Mods {
                        if let Some(version) = mark_version.as_deref() {
                            crate::discover::mark_installed_mods(&mut value, &game_dir, version);
                        }
                    }
                    Ok(value)
                },
            ))
            .unwrap_or_else(|_| Err("Search task panicked".into()));
            let _ = tx.send(BackendEvent::Search {
                request,
                query,
                result,
            });
        });
    if let Err(error) = spawned {
        let _ = failure_tx.send(BackendEvent::Search {
            request,
            query: failure_query,
            result: Err(format!("Cannot start search task: {error}")),
        });
    }
}

pub fn start_manifest(tx: EventSender, request: u64, game_dir: std::path::PathBuf) {
    let failure_tx = tx.clone();
    let spawned = std::thread::Builder::new()
        .name("mc-tui-manifest".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::version::VersionManager::new(&game_dir)
                    .fetch_manifest_result()
                    .map_err(|error| error.to_string())
            }))
            .unwrap_or_else(|_| Err("Version manifest task panicked".into()));
            let _ = tx.send(BackendEvent::Manifest { request, result });
        });
    if let Err(error) = spawned {
        let _ = failure_tx.send(BackendEvent::Manifest {
            request,
            result: Err(format!("Cannot start manifest task: {error}")),
        });
    }
}

pub fn start_download(
    tx: EventSender,
    id: u64,
    game_dir: std::path::PathBuf,
    mc_version: String,
    threads: usize,
    skip_assets: bool,
) {
    spawn_job(
        tx,
        id,
        format!("Download Minecraft {mc_version}"),
        move |tx, id| {
            let vm = crate::version::VersionManager::new(&game_dir);
            phase(tx, id, 1, 4, "Resolving version metadata");
            let (resolved, data) = vm
                .get_version_info(Some(&mc_version))
                .map_err(|e| format!("Version metadata failed: {e}"))?;
            if progress::is_cancelled() {
                return Err("Cancelled".into());
            }

            phase(tx, id, 2, 4, "Fetching client jar");
            send_log(
                tx,
                id,
                LogLevel::Info,
                format!("Target: Minecraft {resolved}"),
            );
            vm.download_client_jar(&resolved, &data)
                .map_err(|e| format!("Client jar failed: {e}"))?;
            if progress::is_cancelled() {
                return Err("Cancelled".into());
            }

            phase(tx, id, 3, 4, "Libraries and natives");
            let natives = game_dir.join("natives").join(&resolved);
            let _ = std::fs::remove_dir_all(&natives);
            std::fs::create_dir_all(&natives)
                .map_err(|e| format!("Cannot create {}: {e}", natives.display()))?;
            let jars = vm
                .download_libraries(&data, &natives, threads)
                .map_err(|e| format!("Libraries failed: {e}"))?;
            send_log(
                tx,
                id,
                LogLevel::Info,
                format!("{} libraries ready", jars.len()),
            );
            if progress::is_cancelled() {
                return Err("Cancelled".into());
            }

            if skip_assets {
                phase(tx, id, 4, 4, "Assets skipped");
            } else {
                phase(tx, id, 4, 4, "Downloading assets");
                vm.download_assets(&data, threads)
                    .map_err(|e| format!("Assets failed: {e}"))?;
            }
            if progress::is_cancelled() {
                return Err("Cancelled".into());
            }
            Ok(format!("Minecraft {resolved} is ready to play"))
        },
    );
}

pub fn start_fabric(
    tx: EventSender,
    id: u64,
    game_dir: std::path::PathBuf,
    mc_version: String,
    loader_version: Option<String>,
) {
    spawn_job(
        tx,
        id,
        format!("Install Fabric for {mc_version}"),
        move |tx, id| {
            phase(tx, id, 1, 2, "Fetching Fabric profile");
            let manager = crate::fabric::FabricManager::new(&game_dir);
            let (jars, profile) = manager
                .install(&mc_version, loader_version.as_deref())
                .map_err(|e| format!("Fabric install failed: {e}"))?;
            if progress::is_cancelled() {
                return Err("Cancelled".into());
            }
            phase(tx, id, 2, 2, "Saving loader profile");
            let name = profile["id"].as_str().unwrap_or("Fabric");
            send_log(
                tx,
                id,
                LogLevel::Info,
                format!("{name}: {} libraries", jars.len()),
            );
            Ok(format!("Fabric installed for Minecraft {mc_version}"))
        },
    );
}

fn cli_base_args(game_dir: &std::path::Path, settings: &Settings, prefix: &mut Vec<String>) {
    prefix.push("--dir".into());
    prefix.push(game_dir.to_string_lossy().to_string());
    prefix.push("--threads".into());
    prefix.push(settings.threads.to_string());
    match settings.source_mode() {
        crate::http::SourceMode::Official => prefix.push("--official".into()),
        crate::http::SourceMode::BmclApi => prefix.push("--bmcl".into()),
        crate::http::SourceMode::Auto => {}
    }
    if !settings.java.trim().is_empty() {
        prefix.push("--java".into());
        prefix.push(settings.java.trim().to_string());
    }
}

pub fn start_loader_install(
    tx: EventSender,
    id: u64,
    game_dir: std::path::PathBuf,
    settings: Settings,
    loader: String,
    mc_version: String,
) {
    let title = format!(
        "Install {} for {mc_version}",
        match loader.as_str() {
            "forge" => "Forge",
            "neoforge" => "NeoForge",
            other => other,
        }
    );
    let _ = tx.send(BackendEvent::JobStarted {
        id,
        title: title.clone(),
    });
    let failure_tx = tx.clone();
    let spawned = std::thread::Builder::new()
        .name("mc-tui-loader".into())
        .spawn(move || {
            let mut args = Vec::new();
            cli_base_args(&game_dir, &settings, &mut args);
            args.push(format!("install-{loader}"));
            args.push("-v".into());
            args.push(mc_version);
            let result = run_child(&tx, id, &title, args);
            let _ = tx.send(BackendEvent::Finished { id, result });
        });
    if let Err(error) = spawned {
        let _ = failure_tx.send(BackendEvent::Finished {
            id,
            result: Err(format!("Cannot start loader worker: {error}")),
        });
    }
}

pub struct ModInstallJob {
    pub game_dir: std::path::PathBuf,
    pub settings: Settings,
    pub slug: String,
    pub mc_version: String,
    pub loader: Option<String>,
    pub mod_version: Option<String>,
}

pub fn start_mod_install(tx: EventSender, id: u64, job: ModInstallJob) {
    let title = format!("Install mod {}", job.slug);
    let _ = tx.send(BackendEvent::JobStarted {
        id,
        title: title.clone(),
    });
    let failure_tx = tx.clone();
    let spawned = std::thread::Builder::new()
        .name("mc-tui-mod".into())
        .spawn(move || {
            let mut args = Vec::new();
            cli_base_args(&job.game_dir, &job.settings, &mut args);
            if let Some(loader) = job.loader {
                args.push("--loader".into());
                args.push(loader);
            }
            args.push("install-mod".into());
            args.push(job.slug);
            args.push("-v".into());
            args.push(job.mc_version);
            args.push("--yes".into());
            if let Some(version) = job.mod_version {
                args.push("--mod-version".into());
                args.push(version);
            }
            let result = run_child(&tx, id, &title, args);
            let _ = tx.send(BackendEvent::Finished { id, result });
        });
    if let Err(error) = spawned {
        let _ = failure_tx.send(BackendEvent::Finished {
            id,
            result: Err(format!("Cannot start mod worker: {error}")),
        });
    }
}

fn run_child(tx: &EventSender, id: u64, title: &str, args: Vec<String>) -> Result<String, String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("Cannot locate launcher executable: {e}"))?;
    phase(tx, id, 1, 1, "Running launcher worker");
    let mut command = Command::new(&exe);
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("MC_LAUNCHER_ALLOW_FILE_STORE", "0")
        .env("MC_LAUNCHER_TUI_PROGRESS", "1")
        .env("NO_COLOR", "1");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot start worker: {e}"))?;
    let _tracker = ChildTracker::new(child.id());

    let (line_tx, line_rx) = mpsc::channel::<String>();
    if let Some(stdout) = child.stdout.take() {
        spawn_reader(stdout, line_tx.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_reader(stderr, line_tx);
    }

    let mut cancelled = false;
    loop {
        while let Ok(line) = line_rx.try_recv() {
            consume_line(tx, id, &line);
        }
        if progress::is_cancelled() && !cancelled {
            cancelled = true;
            kill_process_tree(child.id());
            let _ = child.kill();
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                for line in line_rx.try_iter() {
                    consume_line(tx, id, &line);
                }
                if cancelled {
                    return Err("Cancelled".into());
                }
                if status.success() {
                    send_log(tx, id, LogLevel::Ok, format!("{title} completed"));
                    return Ok(format!("{title} completed"));
                }
                return Err(format!(
                    "Worker exited with {}",
                    status
                        .code()
                        .map(|code| format!("code {code}"))
                        .unwrap_or_else(|| "a signal".into())
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(60)),
            Err(error) => return Err(format!("Cannot wait for worker: {error}")),
        }
    }
}

fn spawn_reader(stream: impl std::io::Read + Send + 'static, tx: Sender<String>) {
    let _ = std::thread::Builder::new()
        .name("mc-tui-child-reader".into())
        .spawn(move || {
            use std::io::{BufRead, BufReader};
            let mut reader = BufReader::new(stream);
            let mut buffer = Vec::with_capacity(512);
            loop {
                buffer.clear();
                match reader.read_until(b'\n', &mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {
                        // A single undecodable byte must not deadlock a child
                        // that is still writing to a full pipe.
                        let text = String::from_utf8_lossy(&buffer);
                        let line = text.trim_end_matches(['\r', '\n']).to_string();
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
}

fn consume_line(tx: &EventSender, id: u64, line: &str) {
    if let Some(payload) = line.strip_prefix("@@MC_TUI@@") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
            if value["kind"] == "progress" {
                let transfer = TransferEvent {
                    id: value["id"].as_str().unwrap_or_default().to_string(),
                    label: value["label"].as_str().unwrap_or_default().to_string(),
                    bytes: value["bytes"].as_u64().unwrap_or(0),
                    total: value["total"].as_u64(),
                    done: value["done"].as_bool().unwrap_or(false),
                };
                let _ = tx.send(BackendEvent::Transfer { id, transfer });
            } else if value["kind"] == "notice" {
                let level = if value["level"] == "error" {
                    LogLevel::Error
                } else {
                    LogLevel::Warn
                };
                let text = value["text"].as_str().unwrap_or_default().to_string();
                send_log(tx, id, level, text);
            }
        }
        return;
    }
    let text = crate::tui::theme::strip_ansi(line).trim().to_string();
    if text.is_empty() {
        return;
    }
    let lowered = text.to_ascii_lowercase();
    let level = if lowered.contains("error") || lowered.contains("failed") {
        LogLevel::Error
    } else if lowered.contains("[warn]") || lowered.contains("warning") {
        LogLevel::Warn
    } else if lowered.contains("[ok]") || lowered.contains("success") {
        LogLevel::Ok
    } else {
        LogLevel::Info
    };
    let text = if text.chars().count() > 320 {
        text.chars().take(320).collect::<String>() + "…"
    } else {
        text
    };
    send_log(tx, id, level, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_are_parsed_without_becoming_log_noise() {
        let (tx, rx) = channel();
        consume_line(
            &tx,
            7,
            "@@MC_TUI@@{\"kind\":\"progress\",\"id\":\"a\",\"label\":\"Asset\",\"bytes\":12,\"total\":20,\"done\":false}",
        );
        match rx.try_recv().unwrap() {
            BackendEvent::Transfer { id, transfer } => {
                assert_eq!(id, 7);
                assert_eq!(transfer.bytes, 12);
                assert_eq!(transfer.total, Some(20));
            }
            other => panic!("unexpected event: {other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn ansi_logs_are_cleaned_and_classified() {
        let (tx, rx) = channel();
        consume_line(&tx, 1, "\u{1b}[31m[ERROR] boom\u{1b}[0m");
        match rx.try_recv().unwrap() {
            BackendEvent::Log { level, text, .. } => {
                assert_eq!(level, LogLevel::Error);
                assert_eq!(text, "[ERROR] boom");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
