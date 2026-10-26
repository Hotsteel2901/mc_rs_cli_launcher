//! TUI application state, input handling and backend event routing.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::style::Color;
use ratatui::widgets::ListState;

use crate::account::AccountManager;
use crate::auth::MicrosoftAuth;
use crate::content::{ShaderEntry, ShaderSupport};
use crate::discover::DiscoverKind;
use crate::http::{self, SourceMode};
use crate::instance::InstanceManager;
use crate::launcher::MinecraftLauncher;
use crate::mod_manager::ModManager;
use crate::progress::{self, TransferEvent};
use crate::tui::config::Settings;
use crate::tui::i18n::{self, Lang};
use crate::tui::input::TextInput;
use crate::tui::jobs::{self, BackendEvent, EventSender, LogLevel};
use crate::tui::theme;

pub const SPLASH_MS: u64 = 1100;
/// Number of rows rendered by the Settings screen.
pub const SETTINGS_ROWS: usize = 11;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Versions,
    Mods,
    Content,
    Accounts,
    Settings,
    Logs,
    About,
}

impl Screen {
    pub const ALL: [Screen; 8] = [
        Screen::Dashboard,
        Screen::Versions,
        Screen::Mods,
        Screen::Content,
        Screen::Accounts,
        Screen::Settings,
        Screen::Logs,
        Screen::About,
    ];

    pub fn title(self, lang: Lang) -> &'static str {
        match self {
            Screen::Dashboard => lang.pick("Dashboard", "概览"),
            Screen::Versions => lang.pick("Versions", "版本"),
            Screen::Mods => lang.pick("Mods", "模组"),
            Screen::Content => lang.pick("Content", "内容"),
            Screen::Accounts => lang.pick("Accounts", "账号"),
            Screen::Settings => lang.pick("Settings", "设置"),
            Screen::Logs => lang.pick("Activity", "活动"),
            Screen::About => lang.pick("About", "关于"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Screen::Dashboard => "◈",
            Screen::Versions => "▤",
            Screen::Mods => "❖",
            Screen::Content => "▣",
            Screen::Accounts => "◉",
            Screen::Settings => "✧",
            Screen::Logs => "≡",
            Screen::About => "✦",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    fn previous(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Which local content list the Content screen is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentKind {
    Saves,
    ResourcePacks,
    Shaders,
}

impl ContentKind {
    pub const ALL: [ContentKind; 3] = [Self::Saves, Self::ResourcePacks, Self::Shaders];

    pub fn title(self, lang: Lang) -> &'static str {
        match self {
            Self::Saves => lang.pick("Saves", "存档"),
            Self::ResourcePacks => lang.pick("Resource packs", "资源包"),
            Self::Shaders => lang.pick("Shaders", "光影包"),
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|kind| *kind == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub at: Duration,
    pub level: LogLevel,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct VersionRow {
    pub id: String,
    pub kind: String,
    pub jar_size: u64,
    pub total_size: u64,
    /// Minecraft version this instance is based on (differs for modpacks).
    pub base: String,
    pub java_major: Option<u32>,
    pub loaders: Vec<String>,
    pub mods_enabled: usize,
    pub mods_disabled: usize,
    pub age: Option<Duration>,
}

#[derive(Clone, Debug)]
pub struct ModRow {
    /// Name shown in the list (without the `.disabled` suffix).
    pub display: String,
    /// Exact on-disk filename, including `.disabled` when applicable.
    pub file: String,
    pub enabled: bool,
    pub size: u64,
    /// Newest Modrinth version, when an update was detected.
    pub update: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AccountRow {
    pub key: String,
    pub kind: String,
    pub username: String,
    pub uuid: String,
    pub expires_at: Option<f64>,
    pub default: bool,
}

#[derive(Clone, Debug)]
pub struct RemoteVersion {
    pub id: String,
    pub kind: String,
    pub released: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PickerItem {
    pub id: String,
    pub label: String,
    pub meta: String,
    pub tag: String,
    pub installed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    RemoteVersions,
    Loaders,
    InstalledVersions,
    JavaRuntimes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputAction {
    OfflineName,
    JavaPath,
    JvmArgs,
    RenameVersion,
    CopyVersion,
    ExportModpack,
    ImportSave,
    ImportResourcePack,
    ImportShader,
}

#[derive(Clone, Debug)]
pub enum ConfirmAction {
    Quit,
    UninstallMod(String),
    Logout,
    ClearLogs,
    DownloadVersion(String),
    InstallLoader(String),
    InstallMod {
        slug: String,
        title: String,
    },
    ResetSettings,
    RemoveVersion(String),
    ApplyUpdates,
    DeleteSave(String),
    DeleteResourcePack(String),
    DeleteShader(String),
    InstallResourcePack {
        slug: String,
        title: String,
    },
    InstallShader {
        slug: String,
        title: String,
        support: ShaderSupport,
    },
    InstallOptimizations,
    Dismiss,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginMode {
    Browser,
    DeviceCode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuspendTask {
    Launch,
    Login(LoginMode),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppCommand {
    None,
    Suspend(SuspendTask),
}

/// Clickable regions are registered by the renderer and consumed by the event
/// loop after the next frame. Coordinates are absolute terminal cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Noop,
    Nav(Screen),
    DashboardPrimary,
    DashboardActivity,
    StatVersions,
    StatMods,
    StatSettings,
    SettingsRow(usize),
    SettingsDecrease(usize),
    SettingsIncrease(usize),
    VersionsRow(usize),
    ModsRow(usize),
    ContentTab(usize),
    ContentRow(usize),
    AccountsRow(usize),
    SearchInput,
    SearchRow(usize),
    PickerRow(usize),
    LoginRow(usize),
    ConfirmYes,
    ConfirmNo,
    CloseModal,
    JobCancel,
}

pub struct PickerState {
    pub title: String,
    pub hint: String,
    pub kind: PickerKind,
    pub input: TextInput,
    pub items: Vec<PickerItem>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub list_state: ListState,
    pub loading: bool,
    pub error: Option<String>,
}

impl PickerState {
    pub fn new(
        title: impl Into<String>,
        hint: impl Into<String>,
        kind: PickerKind,
        items: Vec<PickerItem>,
    ) -> Self {
        let mut picker = Self {
            title: title.into(),
            hint: hint.into(),
            kind,
            input: TextInput::new(""),
            items,
            filtered: Vec::new(),
            selected: 0,
            list_state: ListState::default(),
            loading: false,
            error: None,
        };
        picker.refilter();
        picker
    }

    pub fn set_items(&mut self, items: Vec<PickerItem>) {
        self.items = items;
        self.refilter();
    }

    pub fn refilter(&mut self) {
        let query = self.input.value().to_ascii_lowercase();
        self.filtered = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                query.is_empty()
                    || item.id.to_ascii_lowercase().contains(&query)
                    || item.label.to_ascii_lowercase().contains(&query)
                    || item.meta.to_ascii_lowercase().contains(&query)
                    || item.tag.to_ascii_lowercase().contains(&query)
                    || (item.installed && "installed".contains(&query))
            })
            .map(|(index, _)| index)
            .collect();
        self.selected = 0;
        self.list_state = ListState::default();
        if !self.filtered.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let max = self.filtered.len() - 1;
        let next = (self.selected as isize + delta).clamp(0, max as isize) as usize;
        self.selected = next;
        self.list_state.select(Some(next));
    }

    pub fn page(&mut self, delta: isize) {
        self.move_selection(delta * 8);
    }

    pub fn selected_item(&self) -> Option<&PickerItem> {
        self.filtered
            .get(self.selected)
            .and_then(|index| self.items.get(*index))
    }
}

pub enum Modal {
    Help {
        scroll: usize,
    },
    Confirm {
        title: String,
        lines: Vec<String>,
        action: ConfirmAction,
        danger: bool,
    },
    Input {
        title: String,
        hint: String,
        input: TextInput,
        action: InputAction,
        error: Option<String>,
    },
    Picker(PickerState),
    Login {
        selected: usize,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchFocus {
    #[default]
    Input,
    Results,
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub title: String,
    pub slug: String,
    pub description: String,
    pub author: String,
    pub downloads: u64,
    pub categories: String,
    pub loaders: Vec<String>,
    pub installed: bool,
    pub installed_version: Option<String>,
}

pub struct SearchState {
    pub open: bool,
    pub kind: DiscoverKind,
    pub input: TextInput,
    pub focus: SearchFocus,
    pub results: Vec<SearchHit>,
    pub selected: usize,
    pub list_state: ListState,
    pub loading: bool,
    pub error: Option<String>,
    pub request: u64,
    pub page: u32,
    pub sort: String,
    pub category: Option<String>,
    pub browse: bool,
    pub total_hits: u64,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            open: false,
            kind: DiscoverKind::Mods,
            input: TextInput::new(""),
            focus: SearchFocus::Input,
            results: Vec::new(),
            selected: 0,
            list_state: ListState::default(),
            loading: false,
            error: None,
            request: 0,
            page: 0,
            sort: "relevance".into(),
            category: None,
            browse: true,
            total_hits: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub level: LogLevel,
    pub born: Instant,
    pub ttl: Duration,
}

#[derive(Clone, Debug)]
pub struct Particle {
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub life: f64,
    pub max_life: f64,
    pub glyph: char,
    pub color: Color,
}

#[derive(Clone, Debug)]
pub struct TransferInfo {
    pub label: String,
    pub bytes: u64,
    pub total: Option<u64>,
    pub done: bool,
    pub at: Instant,
}

pub struct JobView {
    pub id: u64,
    pub title: String,
    pub phase_index: usize,
    pub phase_total: usize,
    pub phase_label: String,
    pub transfers: HashMap<String, TransferInfo>,
    pub done_ids: HashSet<String>,
    pub total_bytes: u64,
    pub done_files: usize,
    pub active_files: usize,
    pub latest: Option<TransferInfo>,
    pub started: Instant,
    pub last_rate_at: Instant,
    pub last_rate_bytes: u64,
    pub rate: f64,
    pub rate_history: VecDeque<u64>,
    pub display_ratio: f64,
    pub log_tail: VecDeque<(LogLevel, String)>,
    pub finished: Option<Result<String, String>>,
    pub finished_at: Option<Instant>,
    pub cancel_requested: bool,
}

impl JobView {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        let now = Instant::now();
        Self {
            id,
            title: title.into(),
            phase_index: 1,
            phase_total: 1,
            phase_label: "Preparing".into(),
            transfers: HashMap::new(),
            done_ids: HashSet::new(),
            total_bytes: 0,
            done_files: 0,
            active_files: 0,
            latest: None,
            started: now,
            last_rate_at: now,
            last_rate_bytes: 0,
            rate: 0.0,
            rate_history: VecDeque::new(),
            display_ratio: 0.0,
            log_tail: VecDeque::new(),
            finished: None,
            finished_at: None,
            cancel_requested: false,
        }
    }

    pub fn on_phase(&mut self, index: usize, total: usize, label: String) {
        self.phase_index = index.max(1);
        self.phase_total = total.max(1);
        self.phase_label = label;
        self.latest = None;
    }

    pub fn on_transfer(&mut self, transfer: TransferEvent) {
        let now = Instant::now();
        let previous = self
            .transfers
            .get(&transfer.id)
            .map(|info| info.bytes)
            .unwrap_or(0);
        if transfer.bytes > previous {
            self.total_bytes += transfer.bytes - previous;
        }
        if transfer.done && self.done_ids.insert(transfer.id.clone()) {
            self.done_files += 1;
        }
        let info = TransferInfo {
            label: transfer.label.clone(),
            bytes: transfer.bytes,
            total: transfer.total,
            done: transfer.done,
            at: now,
        };
        self.latest = Some(info.clone());
        self.transfers.insert(transfer.id, info);
        self.active_files = self.transfers.values().filter(|info| !info.done).count();
        if self.transfers.len() > 1024 {
            self.transfers
                .retain(|_, info| !info.done || info.at.elapsed() < Duration::from_secs(2));
        }
    }

    pub fn target_ratio(&self) -> Option<f64> {
        if let Some(result) = &self.finished {
            return match result {
                Ok(_) => Some(1.0),
                Err(_) => None,
            };
        }
        if self.phase_total > 1 {
            return Some(
                (self.phase_index.saturating_sub(1) as f64 / self.phase_total as f64)
                    .clamp(0.0, 1.0),
            );
        }
        let info = self.current_transfer()?;
        let total = info.total.filter(|total| *total > 0)?;
        Some((info.bytes as f64 / total as f64).clamp(0.0, 1.0))
    }

    /// The transfer most worth showing: a live one if any, otherwise the last.
    pub fn current_transfer(&self) -> Option<&TransferInfo> {
        self.transfers
            .values()
            .filter(|info| !info.done)
            .max_by_key(|info| info.at)
            .or(self.latest.as_ref())
    }

    pub fn tick(&mut self, dt: f64) {
        let target = self.target_ratio().unwrap_or(self.display_ratio);
        self.display_ratio += (target - self.display_ratio) * (dt * 9.0).min(1.0);
        if matches!(self.finished, Some(Ok(_))) {
            self.display_ratio += (1.0 - self.display_ratio) * (dt * 6.0).min(1.0);
        }

        let now = Instant::now();
        let elapsed = now.duration_since(self.last_rate_at);
        if elapsed >= Duration::from_millis(500) {
            let delta = self.total_bytes.saturating_sub(self.last_rate_bytes);
            self.rate = delta as f64 / elapsed.as_secs_f64().max(0.001);
            self.last_rate_at = now;
            self.last_rate_bytes = self.total_bytes;
            self.rate_history
                .push_back((self.rate / 1024.0).min(1_000_000.0) as u64);
            while self.rate_history.len() > 40 {
                self.rate_history.pop_front();
            }
        }
    }

    pub fn add_log(&mut self, level: LogLevel, text: String) {
        self.log_tail.push_back((level, text));
        while self.log_tail.len() > 3 {
            self.log_tail.pop_front();
        }
    }

    pub fn is_running(&self) -> bool {
        self.finished.is_none()
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

#[derive(Clone, Default)]
pub struct Bootstrap {
    pub ram: Option<u32>,
    pub threads: Option<usize>,
    pub java: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub version: Option<String>,
    pub source: Option<SourceMode>,
    pub no_splash: bool,
}

pub struct App {
    pub launcher: MinecraftLauncher,
    pub game_dir: PathBuf,
    pub settings: Settings,
    pub screen: Screen,
    pub frame: u64,
    pub session_start: Instant,
    pub last_tick: Instant,
    /// Last key/mouse/paste/backend event; drives the idle animation freeze.
    pub last_activity: Instant,
    /// Ambient animation clock is pinned here while the UI is idle to avoid
    /// streaming a full-screen diff over slow terminals for no reason.
    pub anim_frozen_at: Option<f64>,
    pub quit: bool,
    pub modal: Option<Modal>,
    pub search: SearchState,
    pub splash_until: Option<Instant>,
    pub show_job_panel: bool,
    pub job: Option<JobView>,
    pub versions: Vec<VersionRow>,
    pub version_sel: usize,
    pub version_state: ListState,
    pub mods: Vec<ModRow>,
    pub mod_sel: usize,
    pub mod_state: ListState,
    pub content_kind: ContentKind,
    pub saves: Vec<crate::content::SaveEntry>,
    pub save_sel: usize,
    pub save_state: ListState,
    pub resourcepacks: Vec<crate::content::ResourcePackEntry>,
    pub pack_sel: usize,
    pub pack_state: ListState,
    pub shaders: Vec<ShaderEntry>,
    pub shader_sel: usize,
    pub shader_state: ListState,
    pending_shader_loader: Option<(String, ShaderSupport)>,
    pub accounts: Vec<AccountRow>,
    pub account_sel: usize,
    pub account_state: ListState,
    pub settings_cursor: usize,
    pub settings_saved_at: Option<Instant>,
    /// A malformed config is never replaced until the user explicitly edits or
    /// saves preferences.
    settings_autosave_blocked: bool,
    pub logs: Vec<LogLine>,
    pub log_offset: usize,
    pub log_follow: bool,
    pub toasts: VecDeque<Toast>,
    pub particles: Vec<Particle>,
    pub remote_versions: Option<Vec<PickerItem>>,
    pub java_path: Option<String>,
    pub java_major: Option<u32>,
    pub java_probing: bool,
    /// Mod updates found by the last check, keyed by on-disk filename.
    pub mod_updates: Vec<crate::update::ModUpdate>,
    pub updates_request: u64,
    pub updates_loading: bool,
    pending_update_apply: bool,
    /// Context for the rename/copy/export input dialogs.
    rename_source: Option<String>,
    copy_source: Option<String>,
    export_source: Option<String>,
    /// Crash analysis produced by the last failed launch, replayed in Activity.
    pending_diagnosis: Option<crate::diagnose::Diagnosis>,
    pub next_job_id: u64,
    pub next_request_id: u64,
    pub manifest_request: u64,
    pub tx: EventSender,
    pub pending_activate: Option<String>,
    pub pending_loader: Option<String>,
    last_click: Option<(MouseAction, Instant)>,
}

impl App {
    pub fn lang(&self) -> Lang {
        Lang::from_code(&self.settings.language)
    }

    pub fn new(launcher: MinecraftLauncher, tx: EventSender, bootstrap: Bootstrap) -> Self {
        let game_dir = launcher.game_dir.clone();
        let (mut settings, warning) = Settings::load(&Settings::path(&game_dir));
        if let Some(ram) = bootstrap.ram {
            settings.ram_mb = ram;
        }
        if let Some(threads) = bootstrap.threads {
            settings.threads = threads;
        }
        if let Some(java) = bootstrap.java {
            settings.java = java;
        }
        if let Some(width) = bootstrap.width {
            settings.width = width;
        }
        if let Some(height) = bootstrap.height {
            settings.height = height;
        }
        if let Some(source) = bootstrap.source {
            settings.set_source_mode(source);
        }
        settings.normalize();
        http::set_source_mode(settings.source_mode());

        let now = Instant::now();
        let splash = settings.splash && !settings.reduced_motion && !bootstrap.no_splash;
        let mut app = Self {
            launcher,
            game_dir,
            settings,
            screen: Screen::Dashboard,
            frame: 0,
            session_start: now,
            last_tick: now,
            last_activity: now,
            anim_frozen_at: None,
            quit: false,
            modal: None,
            search: SearchState::default(),
            splash_until: splash.then_some(now + Duration::from_millis(SPLASH_MS)),
            show_job_panel: true,
            job: None,
            versions: Vec::new(),
            version_sel: 0,
            version_state: ListState::default(),
            mods: Vec::new(),
            mod_sel: 0,
            mod_state: ListState::default(),
            content_kind: ContentKind::Saves,
            saves: Vec::new(),
            save_sel: 0,
            save_state: ListState::default(),
            resourcepacks: Vec::new(),
            pack_sel: 0,
            pack_state: ListState::default(),
            shaders: Vec::new(),
            shader_sel: 0,
            shader_state: ListState::default(),
            pending_shader_loader: None,
            accounts: Vec::new(),
            account_sel: 0,
            account_state: ListState::default(),
            settings_cursor: 0,
            settings_saved_at: None,
            settings_autosave_blocked: warning.is_some(),
            logs: Vec::new(),
            log_offset: 0,
            log_follow: true,
            toasts: VecDeque::new(),
            particles: Vec::new(),
            remote_versions: None,
            java_path: None,
            java_major: None,
            java_probing: true,
            mod_updates: Vec::new(),
            updates_request: 0,
            updates_loading: false,
            pending_update_apply: false,
            rename_source: None,
            copy_source: None,
            export_source: None,
            pending_diagnosis: None,
            next_job_id: 1,
            next_request_id: 1,
            manifest_request: 0,
            tx: tx.clone(),
            pending_activate: None,
            pending_loader: None,
            last_click: None,
        };
        let lang = app.lang();
        app.add_log(
            LogLevel::Info,
            lang.pick("Hot-RS-Launcher TUI ready", "Hot-RS-Launcher TUI 已就绪"),
        );
        app.add_log(
            LogLevel::Debug,
            if lang == Lang::Chinese {
                format!("游戏目录：{}", app.game_dir.display())
            } else {
                format!("Game directory: {}", app.game_dir.display())
            },
        );
        if let Some(warning) = warning {
            app.add_log(LogLevel::Warn, warning.clone());
            app.toast(
                LogLevel::Warn,
                if lang == Lang::Chinese {
                    format!("配置警告：{warning}")
                } else {
                    warning
                },
            );
        }
        app.refresh_local();
        if let Some(version) = bootstrap.version {
            if app.versions.iter().any(|row| row.id == version) {
                app.settings.active_version = Some(version.clone());
                app.save_settings_silent();
                app.refresh_local();
                if let Some(index) = app.versions.iter().position(|row| row.id == version) {
                    app.version_sel = index;
                    app.version_state.select(Some(index));
                }
            } else {
                app.add_log(
                    LogLevel::Warn,
                    if app.lang() == Lang::Chinese {
                        format!("请求的版本 {version} 尚未安装")
                    } else {
                        format!("Requested version {version} is not installed")
                    },
                );
                app.toast(
                    LogLevel::Warn,
                    if app.lang() == Lang::Chinese {
                        format!("Minecraft {version} 尚未安装")
                    } else {
                        format!("Minecraft {version} is not installed")
                    },
                );
            }
        }
        app.add_log(
            LogLevel::Info,
            if app.lang() == Lang::Chinese {
                let source = match app.settings.source.as_str() {
                    "official" => "官方",
                    "bmcl" => "BMCLAPI",
                    _ => "自动",
                };
                format!("已安装 {} 个版本 · 下载源 {source}", app.versions.len())
            } else {
                format!(
                    "{} version(s) installed · source {}",
                    app.versions.len(),
                    app.settings.source
                )
            },
        );
        jobs::start_java_probe(tx);
        app
    }

    // ── local data ────────────────────────────────────────────────

    pub fn refresh_local(&mut self) {
        let previous_sel = self
            .versions
            .get(self.version_sel)
            .map(|row| row.id.clone());
        let installed = ModManager::list_installed_versions(&self.game_dir);
        let mut rows = Vec::with_capacity(installed.len());
        let mut skipped_invalid = 0usize;
        for id in installed.into_iter().rev() {
            if crate::util::validate_component(&id).is_err() {
                skipped_invalid += 1;
                continue;
            }
            let version_dir = self.game_dir.join("versions").join(&id);
            let jar = version_dir.join(format!("{id}.jar"));
            let jar_size = jar.metadata().map(|m| m.len()).unwrap_or(0);
            let mut loaders = Vec::new();
            for (loader, label) in [
                ("fabric", "Fabric"),
                ("neoforge", "NeoForge"),
                ("forge", "Forge"),
            ] {
                if crate::instance::loader_profile_path(&self.game_dir, loader, &id).is_some() {
                    loaders.push(label.to_string());
                }
            }
            let mods = ModManager::new(&self.game_dir).list_mods(&id);
            let mods_enabled = mods.iter().filter(|(_, enabled, _)| *enabled).count();
            let mods_disabled = mods.len() - mods_enabled;
            let metadata = std::fs::read(version_dir.join(format!("{id}.json")))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            let kind = metadata
                .as_ref()
                .and_then(|value| value["type"].as_str().map(String::from))
                .map(|value| theme::one_line(&value, 24))
                .unwrap_or_else(|| "release".into());
            let java_major = metadata
                .as_ref()
                .and_then(|value| value["javaVersion"]["majorVersion"].as_u64())
                .map(|value| value as u32);
            let age = std::fs::metadata(&version_dir)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok());
            rows.push(VersionRow {
                id: id.clone(),
                kind,
                jar_size,
                total_size: crate::instance::dir_size(&version_dir),
                base: crate::instance::instance_base(&self.game_dir, &id),
                java_major,
                loaders,
                mods_enabled,
                mods_disabled,
                age,
            });
        }
        self.versions = rows;
        if skipped_invalid > 0 {
            self.add_log(
                LogLevel::Warn,
                if self.lang() == Lang::Chinese {
                    format!("已跳过 {skipped_invalid} 个无效版本目录名")
                } else {
                    format!("Skipped {skipped_invalid} invalid version directory name(s)")
                },
            );
        }

        if self.versions.is_empty() {
            self.version_sel = 0;
            self.version_state = ListState::default();
            self.settings.active_version = None;
        } else {
            if self
                .settings
                .active_version
                .as_deref()
                .is_none_or(|active| !self.versions.iter().any(|row| row.id == active))
            {
                self.settings.active_version = Some(self.versions[0].id.clone());
                self.save_settings_silent();
            }
            self.version_sel = previous_sel
                .and_then(|id| self.versions.iter().position(|row| row.id == id))
                .or_else(|| {
                    self.settings
                        .active_version
                        .as_deref()
                        .and_then(|active| self.versions.iter().position(|row| row.id == active))
                })
                .unwrap_or(0)
                .min(self.versions.len().saturating_sub(1));
            self.version_state = ListState::default();
            self.version_state.select(Some(self.version_sel));
        }

        if let Some(preference) = self.settings.active_loader.as_deref() {
            let label = loader_label(preference);
            let still_installed = self
                .settings
                .active_version
                .as_deref()
                .and_then(|active| self.versions.iter().find(|row| row.id == active))
                .is_some_and(|row| {
                    row.loaders
                        .iter()
                        .any(|value| value.eq_ignore_ascii_case(label))
                });
            if !still_installed {
                self.settings.active_loader = None;
                self.save_settings_silent();
            }
        }

        self.refresh_mods();
        self.refresh_content();
        self.refresh_accounts();
        if let Some(remote) = self.remote_versions.as_mut() {
            let installed: HashSet<&str> = self.versions.iter().map(|v| v.id.as_str()).collect();
            for item in remote.iter_mut() {
                item.installed = installed.contains(item.id.as_str());
            }
        }
    }

    pub fn refresh_mods(&mut self) {
        let previous = self.mods.get(self.mod_sel).map(|row| row.file.clone());
        self.mods.clear();
        if let Some(version) = self.settings.active_version.as_deref() {
            for (name, enabled, size) in ModManager::new(&self.game_dir).list_mods(version) {
                let file = if enabled {
                    name.clone()
                } else {
                    format!("{name}.disabled")
                };
                self.mods.push(ModRow {
                    display: theme::one_line(&name, 200),
                    file,
                    enabled,
                    size,
                    update: None,
                });
            }
        }
        for row in self.mods.iter_mut() {
            row.update = self
                .mod_updates
                .iter()
                .find(|update| {
                    update
                        .file
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .as_deref()
                        == Some(row.file.as_str())
                })
                .map(|update| update.latest.clone());
        }
        self.mods.sort_by(|a, b| {
            b.enabled
                .cmp(&a.enabled)
                .then_with(|| a.display.to_lowercase().cmp(&b.display.to_lowercase()))
        });
        self.mod_sel = previous
            .and_then(|file| self.mods.iter().position(|row| row.file == file))
            .unwrap_or(0)
            .min(self.mods.len().saturating_sub(1));
        self.mod_state = ListState::default();
        if !self.mods.is_empty() {
            self.mod_state.select(Some(self.mod_sel));
        }
    }

    /// Reload worlds and resource packs for the active instance.
    pub fn refresh_content(&mut self) {
        let manager = crate::content::ContentManager::new(&self.game_dir);
        let version = self.settings.active_version.clone();
        self.saves = version
            .as_deref()
            .map(|id| manager.list_saves(id))
            .unwrap_or_default();
        self.resourcepacks = version
            .as_deref()
            .map(|id| manager.list_resourcepacks(id))
            .unwrap_or_default();
        self.shaders = version
            .as_deref()
            .map(|id| manager.list_shaders(id))
            .unwrap_or_default();

        self.save_sel = self.save_sel.min(self.saves.len().saturating_sub(1));
        self.save_state = ListState::default();
        if !self.saves.is_empty() {
            self.save_state.select(Some(self.save_sel));
        }
        self.pack_sel = self
            .pack_sel
            .min(self.resourcepacks.len().saturating_sub(1));
        self.pack_state = ListState::default();
        if !self.resourcepacks.is_empty() {
            self.pack_state.select(Some(self.pack_sel));
        }
        self.shader_sel = self.shader_sel.min(self.shaders.len().saturating_sub(1));
        self.shader_state = ListState::default();
        if !self.shaders.is_empty() {
            self.shader_state.select(Some(self.shader_sel));
        }
    }

    pub fn selected_save(&self) -> Option<&crate::content::SaveEntry> {
        self.saves.get(self.save_sel)
    }

    pub fn selected_resourcepack(&self) -> Option<&crate::content::ResourcePackEntry> {
        self.resourcepacks.get(self.pack_sel)
    }

    pub fn selected_shader(&self) -> Option<&ShaderEntry> {
        self.shaders.get(self.shader_sel)
    }

    pub fn refresh_accounts(&mut self) {
        let previous = self
            .accounts
            .get(self.account_sel)
            .map(|row| row.key.clone());
        self.accounts.clear();
        match AccountManager::public_metadata(&self.game_dir) {
            Ok(data) => {
                let default_key = data["default"].as_str().unwrap_or("").to_string();
                if let Some(map) = data["accounts"].as_object() {
                    for (key, value) in map {
                        if !value.is_object() {
                            continue;
                        }
                        self.accounts.push(AccountRow {
                            key: key.clone(),
                            kind: value["type"]
                                .as_str()
                                .unwrap_or(if key == "msa" { "msa" } else { "offline" })
                                .to_string(),
                            username: theme::one_line(
                                value["username"].as_str().unwrap_or("?"),
                                32,
                            ),
                            uuid: theme::one_line(value["uuid"].as_str().unwrap_or(""), 64),
                            expires_at: value["expires_at"].as_f64(),
                            default: *key == default_key
                                || (default_key.is_empty()
                                    && (key == "msa"
                                        || (key == "offline" && !map.contains_key("msa")))),
                        });
                    }
                }
                self.accounts.sort_by(|a, b| {
                    b.default
                        .cmp(&a.default)
                        .then_with(|| a.kind.cmp(&b.kind))
                        .then_with(|| a.username.cmp(&b.username))
                });
                self.account_sel = previous
                    .and_then(|key| self.accounts.iter().position(|row| row.key == key))
                    .or_else(|| self.accounts.iter().position(|row| row.default))
                    .unwrap_or(0)
                    .min(self.accounts.len().saturating_sub(1));
                self.account_state = ListState::default();
                if !self.accounts.is_empty() {
                    self.account_state.select(Some(self.account_sel));
                }
            }
            Err(error) => {
                let prefix = self.lang().pick("Account metadata", "账号元数据");
                self.add_log(LogLevel::Warn, format!("{prefix}: {error}"));
            }
        }
    }

    pub fn active_loader(&self) -> Option<String> {
        let active = self.settings.active_version.as_deref()?;
        let row = self.versions.iter().find(|row| row.id == active)?;
        if let Some(preference) = self.settings.active_loader.as_deref() {
            let label = loader_label(preference);
            if row
                .loaders
                .iter()
                .any(|value| value.eq_ignore_ascii_case(label))
            {
                return Some(label.to_string());
            }
        }
        row.loaders.first().cloned()
    }

    /// The lowercase loader id expected by the CLI and launcher core.
    pub fn active_loader_id(&self) -> Option<String> {
        self.active_loader()
            .map(|label| loader_id(&label).to_string())
    }

    fn loader_installed(&self, loader: &str) -> bool {
        let Some(active) = self.settings.active_version.as_deref() else {
            return false;
        };
        let label = loader_label(loader);
        self.versions
            .iter()
            .find(|row| row.id == active)
            .is_some_and(|row| {
                row.loaders
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(label))
            })
    }

    pub fn current_version(&self) -> Option<&VersionRow> {
        let active = self.settings.active_version.as_deref()?;
        self.versions.iter().find(|row| row.id == active)
    }

    pub fn selected_version(&self) -> Option<&VersionRow> {
        self.versions.get(self.version_sel)
    }

    pub fn selected_mod(&self) -> Option<&ModRow> {
        self.mods.get(self.mod_sel)
    }

    pub fn selected_account(&self) -> Option<&AccountRow> {
        self.accounts.get(self.account_sel)
    }

    pub fn current_account(&self) -> Option<&AccountRow> {
        self.accounts
            .iter()
            .find(|row| row.default)
            .or_else(|| self.accounts.first())
    }

    // ── settings ──────────────────────────────────────────────────

    pub fn save_settings(&mut self) {
        self.settings_autosave_blocked = false;
        match self.settings.save(&Settings::path(&self.game_dir)) {
            Ok(()) => {
                self.settings_saved_at = Some(Instant::now());
                self.toast(
                    LogLevel::Ok,
                    self.lang().pick("Settings saved", "设置已保存"),
                );
            }
            Err(error) => self.toast(LogLevel::Error, error),
        }
    }

    fn save_settings_silent(&mut self) {
        if self.settings_autosave_blocked {
            return;
        }
        if let Err(error) = self.settings.save(&Settings::path(&self.game_dir)) {
            self.add_log(LogLevel::Warn, error);
        } else {
            self.settings_saved_at = Some(Instant::now());
        }
    }

    pub fn settings_adjust(&mut self, direction: i32) {
        let direction = direction.signum();
        match self.settings_cursor {
            0 => {
                self.settings.language = self.lang().next().code().into();
            }
            1 => {
                let next = (self.settings.ram_mb as i64 + direction as i64 * 512).clamp(
                    crate::util::MIN_RAM_MB as i64,
                    crate::util::MAX_RAM_MB as i64,
                ) as u32;
                self.settings.ram_mb = next;
            }
            2 => {
                const PRESETS: [u32; 6] = [0, 854, 1280, 1600, 1920, 2560];
                self.settings.width = cycle_preset(&PRESETS, self.settings.width, direction);
            }
            3 => {
                const PRESETS: [u32; 6] = [0, 480, 720, 900, 1080, 1440];
                self.settings.height = cycle_preset(&PRESETS, self.settings.height, direction);
            }
            4 => {
                self.settings.threads =
                    (self.settings.threads as i64 + direction as i64).clamp(1, 64) as usize;
            }
            5 => {
                let next = match (self.settings.source.as_str(), direction) {
                    ("auto", 1) => "official",
                    ("official", 1) => "bmcl",
                    ("bmcl", 1) => "auto",
                    ("auto", -1) => "bmcl",
                    ("official", -1) => "auto",
                    ("bmcl", -1) => "official",
                    _ => "auto",
                };
                self.settings.source = next.into();
                http::set_source_mode(self.settings.source_mode());
            }
            6 => {
                self.open_java_picker();
                return;
            }
            7 => {
                self.open_jvm_input();
                return;
            }
            8 => {
                const PRESETS: [&str; 6] = ["auto", "g1", "zgc", "parallel", "serial", "none"];
                let index = PRESETS
                    .iter()
                    .position(|preset| *preset == self.settings.gc)
                    .unwrap_or(0);
                let next = (index as i32 + direction).rem_euclid(PRESETS.len() as i32) as usize;
                self.settings.gc = PRESETS[next].into();
            }
            9 => self.settings.reduced_motion = !self.settings.reduced_motion,
            10 => self.settings.splash = !self.settings.splash,
            _ => {}
        }
        self.settings.normalize();
        self.settings_autosave_blocked = false;
        self.save_settings_silent();
    }

    pub fn settings_activate(&mut self) {
        match self.settings_cursor {
            0 => {
                self.settings_adjust(1);
                let lang = self.lang();
                let message = lang.pick("Language: English", "语言：中文");
                self.toast(LogLevel::Info, message);
            }
            6 => self.open_java_picker(),
            7 => self.open_jvm_input(),
            8 => self.settings_adjust(1),
            9 => {
                self.settings.reduced_motion = !self.settings.reduced_motion;
                if self.settings.reduced_motion {
                    self.particles.clear();
                }
                self.settings_autosave_blocked = false;
                self.save_settings_silent();
            }
            10 => {
                self.settings.splash = !self.settings.splash;
                self.settings_autosave_blocked = false;
                self.save_settings_silent();
            }
            _ => self.settings_adjust(1),
        }
    }

    pub fn reset_settings(&mut self) {
        let active = self.settings.active_version.clone();
        self.settings = Settings::default();
        self.settings.active_version = active;
        http::set_source_mode(self.settings.source_mode());
        self.settings_autosave_blocked = false;
        self.save_settings_silent();
        self.toast(
            LogLevel::Ok,
            self.lang()
                .pick("Settings restored to defaults", "设置已恢复默认值"),
        );
    }

    pub fn settings_labels(lang: Lang) -> [&'static str; 11] {
        [
            lang.pick("Language", "界面语言"),
            lang.pick("Memory", "内存"),
            lang.pick("Window width", "窗口宽度"),
            lang.pick("Window height", "窗口高度"),
            lang.pick("Download threads", "下载线程"),
            lang.pick("Download source", "下载源"),
            lang.pick("Java runtime", "Java 运行时"),
            lang.pick("JVM arguments", "JVM 参数"),
            lang.pick("Garbage collector", "垃圾回收器"),
            lang.pick("Reduced motion", "减少动效"),
            lang.pick("Intro splash", "开场动画"),
        ]
    }

    // ── logging, toasts and effects ───────────────────────────────

    pub fn add_log(&mut self, level: LogLevel, text: impl Into<String>) {
        let text = theme::one_line(&text.into(), 4000);
        self.logs.push(LogLine {
            at: self.session_start.elapsed(),
            level,
            text,
        });
        if self.logs.len() > 800 {
            let overflow = self.logs.len() - 800;
            self.logs.drain(0..overflow);
            self.log_offset = self.log_offset.saturating_sub(overflow);
        }
        if self.log_follow {
            self.log_offset = 0;
        }
    }

    pub fn toast(&mut self, level: LogLevel, text: impl Into<String>) {
        let text = theme::one_line(&text.into(), 320);
        let ttl = match level {
            LogLevel::Error => Duration::from_secs(8),
            LogLevel::Warn => Duration::from_secs(6),
            _ => Duration::from_secs(4),
        };
        self.toasts.push_back(Toast {
            text,
            level,
            born: Instant::now(),
            ttl,
        });
        while self.toasts.len() > 4 {
            self.toasts.pop_front();
        }
    }

    fn log_and_toast(&mut self, level: LogLevel, text: impl Into<String>) {
        let text = text.into();
        self.add_log(level, text.clone());
        self.toast(level, text);
    }

    fn spawn_particles(&mut self, color: Color, amount: usize) {
        if self.settings.reduced_motion {
            return;
        }
        let glyphs = ['✦', '✧', '·', '◆', '●', '▪'];
        let mut seed = self.frame.wrapping_mul(6364136223846793005).wrapping_add(1);
        for _ in 0..amount {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let a = ((seed >> 33) % 1000) as f64 / 1000.0;
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let b = ((seed >> 33) % 1000) as f64 / 1000.0;
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let c = ((seed >> 33) % 1000) as f64 / 1000.0;
            let color = match (seed % 4) as u8 {
                0 => color,
                1 => theme::GOLD,
                2 => theme::CYAN,
                _ => theme::LIME,
            };
            let life = 0.9 + a * 1.1;
            self.particles.push(Particle {
                x: 0.5 + (b - 0.5) * 0.25,
                y: 0.55 + (c - 0.5) * 0.15,
                vx: (b - 0.5) * 0.9,
                vy: -0.35 - a * 0.75,
                life,
                max_life: life,
                glyph: glyphs[(seed >> 8) as usize % glyphs.len()],
                color,
            });
        }
        while self.particles.len() > 140 {
            self.particles.remove(0);
        }
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        if self.wants_fast_frames() {
            self.anim_frozen_at = None;
        } else if self.anim_frozen_at.is_none()
            && self.last_activity.elapsed() > Duration::from_secs(2)
        {
            self.anim_frozen_at = Some(self.session_start.elapsed().as_secs_f64());
        }
        let dt = now
            .duration_since(self.last_tick)
            .as_secs_f64()
            .clamp(0.0, 0.25);
        self.last_tick = now;
        self.frame = self.frame.wrapping_add(1);

        if let Some(job) = self.job.as_mut() {
            job.tick(dt);
        }
        if let Some(job) = self.job.as_ref() {
            // Successful jobs fade out quickly; failures stay long enough to
            // read. Either way the entry must eventually disappear, otherwise
            // `wants_fast_frames` would keep the UI at 30 FPS forever.
            let limit = if job.finished.as_ref().is_some_and(|result| result.is_ok()) {
                Duration::from_millis(2400)
            } else {
                Duration::from_millis(12_000)
            };
            if job.finished.is_some() && job.finished_at.is_some_and(|at| at.elapsed() > limit) {
                self.job = None;
            }
        }

        self.toasts
            .retain(|toast| toast.born.elapsed() < toast.ttl + Duration::from_millis(260));

        self.particles.retain_mut(|particle| {
            particle.life -= dt;
            if particle.life <= 0.0 {
                return false;
            }
            particle.x += particle.vx * dt;
            particle.y += particle.vy * dt;
            particle.vy += 0.85 * dt;
            particle.vx *= 1.0 - 0.35 * dt;
            (0.0..=1.0).contains(&particle.x) && particle.y < 1.2
        });

        if self.splash_until.is_some_and(|until| now >= until) {
            self.splash_until = None;
        }
    }

    pub fn splash_active(&self) -> bool {
        self.splash_until.is_some()
    }

    /// Record user or backend activity; ambient animation resumes immediately.
    pub fn note_activity(&mut self) {
        self.last_activity = Instant::now();
        self.anim_frozen_at = None;
    }

    /// Clock used by gradients, glows and particles. It stops while the UI is
    /// idle so unchanged screens stop producing terminal output.
    pub fn anim_time(&self) -> f64 {
        self.anim_frozen_at
            .unwrap_or_else(|| self.session_start.elapsed().as_secs_f64())
    }

    /// True while something on screen (splash, toasts, a running job) is
    /// animating. Idle screens otherwise drop to a slower redraw cadence so a
    /// continuously animated gradient does not stream a full-screen diff at
    /// 30 FPS over SSH for no reason.
    pub fn wants_fast_frames(&self) -> bool {
        self.splash_until.is_some()
            || self.job.as_ref().is_some_and(|job| {
                job.finished.is_none()
                    || job
                        .finished_at
                        .is_none_or(|at| at.elapsed() < Duration::from_millis(2400))
            })
            || self
                .toasts
                .iter()
                .any(|toast| toast.born.elapsed() < toast.ttl + Duration::from_millis(260))
    }

    pub fn skip_splash(&mut self) {
        self.splash_until = None;
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn job_status_label(&self) -> String {
        let lang = self.lang();
        match &self.job {
            Some(job) if job.is_running() => theme::truncate_chars(
                &format!(
                    "{} · {} · {}",
                    lang.pick("WORKING", "工作中"),
                    job.title,
                    i18n::localize_job_phase(lang, &job.phase_label)
                ),
                64,
            ),
            Some(job) => match &job.finished {
                Some(Ok(_)) => lang
                    .pick("COMPLETE · esc dismiss", "完成 · esc 关闭")
                    .into(),
                Some(Err(_)) => lang
                    .pick(
                        "FAILED · esc dismiss · l for details",
                        "失败 · esc 关闭 · l 查看详情",
                    )
                    .into(),
                None => lang.pick("WORKING", "工作中").into(),
            },
            None if self.search.open => lang.pick("SEARCH · Modrinth", "搜索 · Modrinth").into(),
            None if self.modal.is_some() => lang.pick("DIALOG", "对话框").into(),
            None => lang.pick("READY", "就绪").into(),
        }
    }

    pub fn job_status_color(&self) -> Color {
        match &self.job {
            Some(job) if job.is_running() => theme::GOLD,
            Some(job) => match &job.finished {
                Some(Ok(_)) => theme::GREEN,
                Some(Err(_)) => theme::RED,
                None => theme::GOLD,
            },
            None if self.search.open => theme::CYAN,
            None if self.modal.is_some() => theme::GOLD,
            None => theme::GREEN,
        }
    }

    // ── input handling ────────────────────────────────────────────

    pub fn handle_paste(&mut self, text: &str) {
        self.note_activity();
        if let Some(modal) = self.modal.as_mut() {
            match modal {
                Modal::Input { input, .. } => input.insert_str(text),
                Modal::Picker(picker) => {
                    picker.input.insert_str(text);
                    picker.refilter();
                }
                _ => {}
            }
            return;
        }
        if self.search.open && self.search.focus == SearchFocus::Input {
            self.search.input.insert_str(text);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppCommand {
        self.note_activity();
        if key.kind == KeyEventKind::Release {
            return AppCommand::None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.request_quit();
            return AppCommand::None;
        }
        if self.splash_active() {
            self.skip_splash();
            return AppCommand::None;
        }
        if self.modal.is_some() {
            return self.handle_modal_key(key);
        }
        if self.search.open {
            return self.handle_search_key(key);
        }
        if key.code == KeyCode::Esc && self.job.as_ref().is_some_and(|job| job.finished.is_some()) {
            self.job = None;
            return AppCommand::None;
        }
        if key.code == KeyCode::Char('c') && self.job.as_ref().is_some_and(JobView::is_running) {
            self.cancel_job();
            return AppCommand::None;
        }
        match key.code {
            KeyCode::Char('q') => {
                self.request_quit();
                AppCommand::None
            }
            KeyCode::Char('?') => {
                self.modal = Some(Modal::Help { scroll: 0 });
                AppCommand::None
            }
            KeyCode::Char('j') => {
                self.show_job_panel = !self.show_job_panel;
                AppCommand::None
            }
            KeyCode::Char('l') => {
                self.screen = Screen::Logs;
                AppCommand::None
            }
            KeyCode::Tab => {
                self.screen = self.screen.next();
                AppCommand::None
            }
            KeyCode::BackTab => {
                self.screen = self.screen.previous();
                AppCommand::None
            }
            KeyCode::Char('1') => {
                self.screen = Screen::Dashboard;
                AppCommand::None
            }
            KeyCode::Char('2') => {
                self.screen = Screen::Versions;
                AppCommand::None
            }
            KeyCode::Char('3') => {
                self.screen = Screen::Mods;
                AppCommand::None
            }
            KeyCode::Char('4') => {
                self.screen = Screen::Content;
                AppCommand::None
            }
            KeyCode::Char('5') => {
                self.screen = Screen::Accounts;
                AppCommand::None
            }
            KeyCode::Char('6') => {
                self.screen = Screen::Settings;
                AppCommand::None
            }
            KeyCode::Char('7') => {
                self.screen = Screen::Logs;
                AppCommand::None
            }
            KeyCode::Char('8') => {
                self.screen = Screen::About;
                AppCommand::None
            }
            KeyCode::Esc => {
                if self.screen != Screen::Dashboard {
                    self.screen = Screen::Dashboard;
                    AppCommand::None
                } else {
                    AppCommand::None
                }
            }
            _ => match self.screen {
                Screen::Dashboard => self.handle_dashboard_key(key),
                Screen::Versions => self.handle_versions_key(key),
                Screen::Mods => self.handle_mods_key(key),
                Screen::Content => self.handle_content_key(key),
                Screen::Accounts => self.handle_accounts_key(key),
                Screen::Settings => self.handle_settings_key(key),
                Screen::Logs => self.handle_logs_key(key),
                Screen::About => self.handle_about_key(key),
            },
        }
    }

    pub fn handle_mouse(&mut self, event: MouseEvent, hit: Option<MouseAction>) -> AppCommand {
        self.note_activity();
        if self.splash_active() {
            if matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.skip_splash();
            }
            return AppCommand::None;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(action) = hit else {
                    self.last_click = None;
                    return AppCommand::None;
                };
                let now = Instant::now();
                let double_click = self.last_click.is_some_and(|(last, at)| {
                    last == action && now.duration_since(at) < Duration::from_millis(450)
                });
                self.last_click = Some((action, now));
                self.apply_mouse_action(action, double_click)
            }
            MouseEventKind::ScrollUp => {
                self.last_click = None;
                self.scroll_mouse(true);
                AppCommand::None
            }
            MouseEventKind::ScrollDown => {
                self.last_click = None;
                self.scroll_mouse(false);
                AppCommand::None
            }
            _ => AppCommand::None,
        }
    }

    fn apply_mouse_action(&mut self, action: MouseAction, double_click: bool) -> AppCommand {
        match action {
            MouseAction::Noop => {}
            MouseAction::Nav(screen) => {
                self.screen = screen;
                self.search.open = false;
            }
            MouseAction::DashboardPrimary => return self.play_active(),
            MouseAction::DashboardActivity => self.screen = Screen::Logs,
            MouseAction::StatVersions => self.screen = Screen::Versions,
            MouseAction::StatMods => self.screen = Screen::Mods,
            MouseAction::StatSettings => self.screen = Screen::Settings,
            MouseAction::SettingsRow(index) => {
                self.settings_cursor = index.min(SETTINGS_ROWS - 1);
            }
            MouseAction::SettingsDecrease(index) => {
                self.settings_cursor = index.min(SETTINGS_ROWS - 1);
                if matches!(index, 0 | 6 | 7 | 9 | 10) {
                    self.settings_activate();
                } else {
                    self.settings_adjust(-1);
                }
            }
            MouseAction::SettingsIncrease(index) => {
                self.settings_cursor = index.min(SETTINGS_ROWS - 1);
                if matches!(index, 0 | 6 | 7 | 9 | 10) {
                    self.settings_activate();
                } else {
                    self.settings_adjust(1);
                }
            }
            MouseAction::VersionsRow(index) => {
                if index >= self.versions.len() {
                    return AppCommand::None;
                }
                if double_click && self.version_sel == index {
                    let version = self.versions[index].id.clone();
                    return self.launch_version(version);
                }
                self.version_sel = index;
                self.version_state.select(Some(index));
            }
            MouseAction::ModsRow(index) => {
                if index >= self.mods.len() {
                    return AppCommand::None;
                }
                if double_click && self.mod_sel == index {
                    self.toggle_selected_mod();
                } else {
                    self.mod_sel = index;
                    self.mod_state.select(Some(index));
                }
            }
            MouseAction::ContentTab(index) => {
                if let Some(kind) = ContentKind::ALL.get(index) {
                    self.content_kind = *kind;
                }
            }
            MouseAction::ContentRow(index) => match self.content_kind {
                ContentKind::Saves => {
                    if index >= self.saves.len() {
                        return AppCommand::None;
                    }
                    if double_click && self.save_sel == index {
                        let folder = self.saves[index].folder.clone();
                        self.begin_backup_save(folder);
                    } else {
                        self.save_sel = index;
                        self.save_state.select(Some(index));
                    }
                }
                ContentKind::ResourcePacks => {
                    if index >= self.resourcepacks.len() {
                        return AppCommand::None;
                    }
                    if double_click && self.pack_sel == index {
                        self.toggle_selected_resourcepack();
                    } else {
                        self.pack_sel = index;
                        self.pack_state.select(Some(index));
                    }
                }
                ContentKind::Shaders => {
                    if index >= self.shaders.len() {
                        return AppCommand::None;
                    }
                    if double_click && self.shader_sel == index {
                        self.toggle_selected_shader();
                    } else {
                        self.shader_sel = index;
                        self.shader_state.select(Some(index));
                    }
                }
            },
            MouseAction::AccountsRow(index) => {
                if index >= self.accounts.len() {
                    return AppCommand::None;
                }
                if double_click && self.account_sel == index {
                    self.set_selected_default();
                } else {
                    self.account_sel = index;
                    self.account_state.select(Some(index));
                }
            }
            MouseAction::SearchInput => {
                self.search.focus = SearchFocus::Input;
            }
            MouseAction::SearchRow(index) => {
                if index >= self.search.results.len() {
                    return AppCommand::None;
                }
                self.search.selected = index;
                self.search.list_state.select(Some(index));
                self.search.focus = SearchFocus::Results;
                if double_click {
                    self.confirm_search_install();
                }
            }
            MouseAction::PickerRow(index) => {
                let mut chosen = None;
                if let Some(Modal::Picker(picker)) = self.modal.as_mut() {
                    if index >= picker.filtered.len() {
                        return AppCommand::None;
                    }
                    if double_click && picker.selected == index {
                        if let Some(item) = picker.selected_item() {
                            chosen = Some((picker.kind, item.id.clone()));
                        }
                    } else {
                        picker.selected = index;
                        picker.list_state.select(Some(index));
                    }
                }
                if let Some((kind, id)) = chosen {
                    self.modal = None;
                    return self.apply_pick(kind, &id);
                }
            }
            MouseAction::LoginRow(index) => {
                let mut chosen = None;
                if let Some(Modal::Login { selected }) = self.modal.as_mut() {
                    if double_click && *selected == index {
                        chosen = Some(if index == 0 {
                            LoginMode::Browser
                        } else {
                            LoginMode::DeviceCode
                        });
                    } else {
                        *selected = index.min(1);
                    }
                }
                if let Some(mode) = chosen {
                    self.modal = None;
                    return AppCommand::Suspend(SuspendTask::Login(mode));
                }
            }
            MouseAction::ConfirmYes => {
                if let Some(Modal::Confirm { action, .. }) = self.modal.take() {
                    return self.apply_confirm(action);
                }
            }
            MouseAction::ConfirmNo | MouseAction::CloseModal => {
                self.modal = None;
            }
            MouseAction::JobCancel => self.cancel_job(),
        }
        AppCommand::None
    }

    fn scroll_mouse(&mut self, up: bool) {
        if let Some(modal) = self.modal.as_mut() {
            match modal {
                Modal::Help { scroll } => {
                    if up {
                        *scroll = scroll.saturating_sub(3);
                    } else {
                        *scroll = scroll.saturating_add(3);
                    }
                }
                Modal::Picker(picker) => picker.move_selection(if up { -1 } else { 1 }),
                Modal::Login { selected } => {
                    if up {
                        *selected = selected.saturating_sub(1);
                    } else {
                        *selected = (*selected + 1).min(1);
                    }
                }
                Modal::Confirm { .. } | Modal::Input { .. } => {}
            }
            return;
        }
        if self.search.open {
            if !self.search.results.is_empty() {
                self.search.focus = SearchFocus::Results;
                if up {
                    self.search.selected = self.search.selected.saturating_sub(1);
                } else {
                    self.search.selected =
                        (self.search.selected + 1).min(self.search.results.len() - 1);
                }
                self.search.list_state.select(Some(self.search.selected));
            }
            return;
        }
        match self.screen {
            Screen::Dashboard => {}
            Screen::Versions => {
                if !self.versions.is_empty() {
                    self.version_sel = if up {
                        self.version_sel.saturating_sub(1)
                    } else {
                        (self.version_sel + 1).min(self.versions.len() - 1)
                    };
                    self.version_state.select(Some(self.version_sel));
                }
            }
            Screen::Mods => {
                if !self.mods.is_empty() {
                    self.mod_sel = if up {
                        self.mod_sel.saturating_sub(1)
                    } else {
                        (self.mod_sel + 1).min(self.mods.len() - 1)
                    };
                    self.mod_state.select(Some(self.mod_sel));
                }
            }
            Screen::Content => self.move_content_selection(if up { -1 } else { 1 }),
            Screen::Accounts => {
                if !self.accounts.is_empty() {
                    self.account_sel = if up {
                        self.account_sel.saturating_sub(1)
                    } else {
                        (self.account_sel + 1).min(self.accounts.len() - 1)
                    };
                    self.account_state.select(Some(self.account_sel));
                }
            }
            Screen::Settings => {
                self.settings_cursor = if up {
                    self.settings_cursor.saturating_sub(1)
                } else {
                    (self.settings_cursor + 1).min(SETTINGS_ROWS - 1)
                };
            }
            Screen::Logs => {
                if up {
                    self.log_follow = false;
                    self.log_offset = (self.log_offset + 1).min(self.logs.len().saturating_sub(1));
                } else {
                    self.log_offset = self.log_offset.saturating_sub(1);
                    if self.log_offset == 0 {
                        self.log_follow = true;
                    }
                }
            }
            Screen::About => {}
        }
    }

    fn handle_dashboard_key(&mut self, key: KeyEvent) -> AppCommand {
        match key.code {
            KeyCode::Enter | KeyCode::Char('p') => self.play_active(),
            KeyCode::Char('n') => {
                self.open_remote_picker();
                AppCommand::None
            }
            KeyCode::Char('i') => {
                self.open_loader_picker();
                AppCommand::None
            }
            KeyCode::Char('s') => {
                self.open_search();
                AppCommand::None
            }
            KeyCode::Char('a') => {
                self.screen = Screen::Accounts;
                AppCommand::None
            }
            KeyCode::Char('r') => {
                self.refresh_local();
                self.log_and_toast(
                    LogLevel::Info,
                    self.lang().pick("Workspace refreshed", "工作区已刷新"),
                );
                AppCommand::None
            }
            _ => AppCommand::None,
        }
    }

    fn handle_versions_key(&mut self, key: KeyEvent) -> AppCommand {
        match key.code {
            KeyCode::Up => {
                if !self.versions.is_empty() {
                    self.version_sel = self.version_sel.saturating_sub(1);
                    self.version_state.select(Some(self.version_sel));
                }
                AppCommand::None
            }
            KeyCode::Down => {
                if !self.versions.is_empty() {
                    self.version_sel = (self.version_sel + 1).min(self.versions.len() - 1);
                    self.version_state.select(Some(self.version_sel));
                }
                AppCommand::None
            }
            KeyCode::PageUp => {
                self.version_sel = self.version_sel.saturating_sub(8);
                self.version_state.select(Some(self.version_sel));
                AppCommand::None
            }
            KeyCode::PageDown => {
                if !self.versions.is_empty() {
                    self.version_sel = (self.version_sel + 8).min(self.versions.len() - 1);
                    self.version_state.select(Some(self.version_sel));
                }
                AppCommand::None
            }
            KeyCode::Enter | KeyCode::Char('p') => {
                if let Some(version) = self.selected_version().map(|row| row.id.clone()) {
                    self.launch_version(version)
                } else {
                    self.toast(
                        LogLevel::Warn,
                        self.lang().pick("No version selected", "未选择版本"),
                    );
                    AppCommand::None
                }
            }
            KeyCode::Char('v') => {
                if let Some(version) = self.selected_version().map(|row| row.id.clone()) {
                    self.set_active_version(version);
                }
                AppCommand::None
            }
            KeyCode::Char('n') => {
                self.open_remote_picker();
                AppCommand::None
            }
            KeyCode::Char('i') => {
                if let Some(version) = self.selected_version().map(|row| row.id.clone()) {
                    if self.settings.active_version.as_deref() != Some(version.as_str()) {
                        self.settings.active_version = Some(version);
                        self.save_settings_silent();
                        self.refresh_mods();
                    }
                }
                self.open_loader_picker();
                AppCommand::None
            }
            KeyCode::Char('r') => {
                self.refresh_local();
                self.log_and_toast(
                    LogLevel::Info,
                    self.lang().pick("Version list refreshed", "版本列表已刷新"),
                );
                AppCommand::None
            }
            KeyCode::Char('s') => {
                self.open_search();
                AppCommand::None
            }
            KeyCode::Char('d') => {
                if let Some(version) = self.selected_version().map(|row| row.id.clone()) {
                    let lang = self.lang();
                    self.modal = Some(Modal::Confirm {
                        title: if lang == Lang::Chinese {
                            format!("删除实例 {version}")
                        } else {
                            format!("Delete instance {version}")
                        },
                        lines: vec![
                            lang.pick(
                                "The instance directory, saves and mods are deleted.",
                                "实例目录、存档和模组都会被删除。",
                            )
                            .into(),
                            lang.pick(
                                "Keep saves with the CLI: remove-version --keep-saves",
                                "如需保留存档：remove-version --keep-saves",
                            )
                            .into(),
                        ],
                        action: ConfirmAction::RemoveVersion(version),
                        danger: true,
                    });
                }
                AppCommand::None
            }
            KeyCode::Char('R') => {
                self.open_rename_input();
                AppCommand::None
            }
            KeyCode::Char('c') => {
                self.open_copy_input();
                AppCommand::None
            }
            KeyCode::Char('e') => {
                self.open_export_input();
                AppCommand::None
            }
            _ => AppCommand::None,
        }
    }

    fn handle_mods_key(&mut self, key: KeyEvent) -> AppCommand {
        match key.code {
            KeyCode::Up => {
                if !self.mods.is_empty() {
                    self.mod_sel = self.mod_sel.saturating_sub(1);
                    self.mod_state.select(Some(self.mod_sel));
                }
                AppCommand::None
            }
            KeyCode::Down => {
                if !self.mods.is_empty() {
                    self.mod_sel = (self.mod_sel + 1).min(self.mods.len() - 1);
                    self.mod_state.select(Some(self.mod_sel));
                }
                AppCommand::None
            }
            KeyCode::Char(' ') => {
                self.toggle_selected_mod();
                AppCommand::None
            }
            KeyCode::Char('x') => {
                if let Some(mod_row) = self.selected_mod() {
                    let file = mod_row.file.clone();
                    let display = mod_row.display.clone();
                    let lang = self.lang();
                    self.modal = Some(Modal::Confirm {
                        title: lang.pick("Uninstall mod", "卸载模组").into(),
                        lines: vec![
                            if lang == Lang::Chinese {
                                format!("从当前版本移除 {display}？")
                            } else {
                                format!("Remove {display} from the active version?")
                            },
                            lang.pick(
                                "The jar file is deleted from disk.",
                                "JAR 文件将从磁盘删除。",
                            )
                            .into(),
                        ],
                        action: ConfirmAction::UninstallMod(file),
                        danger: true,
                    });
                }
                AppCommand::None
            }
            KeyCode::Char('s') | KeyCode::Char('/') => {
                self.open_search();
                AppCommand::None
            }
            KeyCode::Char('v') => {
                self.open_installed_version_picker();
                AppCommand::None
            }
            KeyCode::Char('r') => {
                self.refresh_mods();
                self.log_and_toast(
                    LogLevel::Info,
                    self.lang().pick("Mod list refreshed", "模组列表已刷新"),
                );
                AppCommand::None
            }
            KeyCode::Char('u') => {
                self.begin_update_check();
                AppCommand::None
            }
            KeyCode::Char('U') => {
                self.confirm_apply_updates();
                AppCommand::None
            }
            KeyCode::Char('O') => {
                self.confirm_optimize_mods();
                AppCommand::None
            }
            _ => AppCommand::None,
        }
    }

    fn handle_content_key(&mut self, key: KeyEvent) -> AppCommand {
        match key.code {
            KeyCode::Up => self.move_content_selection(-1),
            KeyCode::Down => self.move_content_selection(1),
            KeyCode::PageUp => self.move_content_selection(-8),
            KeyCode::PageDown => self.move_content_selection(8),
            KeyCode::Char('t') | KeyCode::Char(']') => {
                self.content_kind = self.content_kind.next();
            }
            KeyCode::Char('[') => {
                self.content_kind = self.content_kind.previous();
            }
            KeyCode::Char(' ') | KeyCode::Enter => match self.content_kind {
                ContentKind::Saves => self.backup_selected_save(),
                ContentKind::ResourcePacks => self.toggle_selected_resourcepack(),
                ContentKind::Shaders => self.toggle_selected_shader(),
            },
            KeyCode::Char('d') => match self.content_kind {
                ContentKind::Saves => {}
                ContentKind::ResourcePacks => {
                    self.open_discover(DiscoverKind::ResourcePacks);
                }
                ContentKind::Shaders => {
                    self.open_discover(DiscoverKind::Shaders);
                }
            },
            KeyCode::Char('b') => {
                if self.content_kind == ContentKind::Saves {
                    self.backup_selected_save();
                }
            }
            KeyCode::Char('i') => self.open_content_import(),
            KeyCode::Char('x') => self.confirm_delete_content(),
            KeyCode::Char('o') => self.open_content_dir(),
            KeyCode::Char('v') => self.open_installed_version_picker(),
            KeyCode::Char('r') => {
                self.refresh_content();
                self.log_and_toast(
                    LogLevel::Info,
                    self.lang().pick("Content list refreshed", "内容列表已刷新"),
                );
            }
            _ => {}
        }
        AppCommand::None
    }

    fn move_content_selection(&mut self, delta: isize) {
        match self.content_kind {
            ContentKind::Saves => {
                if !self.saves.is_empty() {
                    self.save_sel = (self.save_sel as isize + delta)
                        .clamp(0, self.saves.len() as isize - 1)
                        as usize;
                    self.save_state.select(Some(self.save_sel));
                }
            }
            ContentKind::ResourcePacks => {
                if !self.resourcepacks.is_empty() {
                    self.pack_sel = (self.pack_sel as isize + delta)
                        .clamp(0, self.resourcepacks.len() as isize - 1)
                        as usize;
                    self.pack_state.select(Some(self.pack_sel));
                }
            }
            ContentKind::Shaders => {
                if !self.shaders.is_empty() {
                    self.shader_sel = (self.shader_sel as isize + delta)
                        .clamp(0, self.shaders.len() as isize - 1)
                        as usize;
                    self.shader_state.select(Some(self.shader_sel));
                }
            }
        }
    }

    fn backup_selected_save(&mut self) {
        let Some(save) = self.selected_save() else {
            self.toast(
                LogLevel::Warn,
                self.lang().pick("No world selected", "未选择存档"),
            );
            return;
        };
        let folder = save.folder.clone();
        self.begin_backup_save(folder);
    }

    fn toggle_selected_resourcepack(&mut self) {
        let lang = self.lang();
        let Some(pack) = self.selected_resourcepack() else {
            self.toast(
                LogLevel::Warn,
                lang.pick("No resource pack selected", "未选择资源包"),
            );
            return;
        };
        let file = pack.file.clone();
        let enabled = !pack.enabled;
        let Some(version) = self.settings.active_version.clone() else {
            self.toast(
                LogLevel::Warn,
                lang.pick(
                    "Select an installed version first",
                    "请先选择一个已安装版本",
                ),
            );
            return;
        };
        let manager = crate::content::ContentManager::new(&self.game_dir);
        match manager.set_resourcepack_enabled(&version, &file, enabled) {
            Ok(_) => {
                self.refresh_content();
                if let Some(index) = self.resourcepacks.iter().position(|pack| pack.file == file) {
                    self.pack_sel = index;
                    self.pack_state.select(Some(index));
                }
                let message = if lang == Lang::Chinese {
                    format!("{}资源包 {file}", if enabled { "已启用" } else { "已禁用" })
                } else {
                    format!(
                        "{} resource pack {file}",
                        if enabled { "Enabled" } else { "Disabled" }
                    )
                };
                self.log_and_toast(LogLevel::Ok, message);
            }
            Err(error) => self.log_and_toast(LogLevel::Error, error.to_string()),
        }
    }

    fn toggle_selected_shader(&mut self) {
        let lang = self.lang();
        let Some(shader) = self.selected_shader() else {
            self.toast(
                LogLevel::Warn,
                lang.pick("No shader pack selected", "未选择光影包"),
            );
            return;
        };
        let file = shader.file.clone();
        let enabled = !shader.enabled;
        let Some(version) = self.settings.active_version.clone() else {
            return;
        };
        let manager = crate::content::ContentManager::new(&self.game_dir);
        let result = if enabled {
            manager
                .set_shader_enabled(&version, &file, true)
                .map(|_| ())
        } else {
            manager.disable_shader(&version)
        };
        match result {
            Ok(()) => {
                self.refresh_content();
                if let Some(index) = self.shaders.iter().position(|shader| shader.file == file) {
                    self.shader_sel = index;
                    self.shader_state.select(Some(index));
                }
                let message = if lang == Lang::Chinese {
                    format!("{}光影包 {file}", if enabled { "已启用" } else { "已禁用" })
                } else {
                    format!(
                        "{} shader pack {file}",
                        if enabled { "Enabled" } else { "Disabled" }
                    )
                };
                self.log_and_toast(LogLevel::Ok, message);
            }
            Err(error) => self.log_and_toast(LogLevel::Error, error.to_string()),
        }
    }

    fn open_content_import(&mut self) {
        let lang = self.lang();
        let (title, hint, action) = match self.content_kind {
            ContentKind::Saves => (
                lang.pick("Import world", "导入存档").to_string(),
                lang.pick(
                    "Path to a world .zip (launcher backup or vanilla world)",
                    "存档 .zip 的路径（启动器备份或原版世界压缩包）",
                )
                .to_string(),
                InputAction::ImportSave,
            ),
            ContentKind::ResourcePacks => (
                lang.pick("Import resource pack", "导入资源包").to_string(),
                lang.pick(
                    "Path to a pack .zip or folder",
                    "资源包 .zip 或文件夹的路径",
                )
                .to_string(),
                InputAction::ImportResourcePack,
            ),
            ContentKind::Shaders => (
                lang.pick("Import shader pack", "导入光影包").to_string(),
                lang.pick(
                    "Path to a shader .zip or folder with a shaders directory",
                    "光影包 .zip 或包含 shaders 目录的文件夹路径",
                )
                .to_string(),
                InputAction::ImportShader,
            ),
        };
        self.modal = Some(Modal::Input {
            title,
            hint,
            input: TextInput::new("").with_max_chars(1024),
            action,
            error: None,
        });
    }

    fn confirm_delete_content(&mut self) {
        let lang = self.lang();
        match self.content_kind {
            ContentKind::Saves => {
                let Some(save) = self.selected_save() else {
                    self.toast(LogLevel::Warn, lang.pick("No world selected", "未选择存档"));
                    return;
                };
                let folder = save.folder.clone();
                let name = save.name.clone();
                self.modal = Some(Modal::Confirm {
                    title: if lang == Lang::Chinese {
                        format!("删除存档 {name}")
                    } else {
                        format!("Delete world {name}")
                    },
                    lines: vec![
                        if lang == Lang::Chinese {
                            format!("永久删除文件夹 {folder} 及其中的区域文件？")
                        } else {
                            format!("Permanently delete folder {folder} and its region files?")
                        },
                        lang.pick(
                            "Back it up first with b; this cannot be undone.",
                            "建议先按 b 备份；此操作无法撤销。",
                        )
                        .into(),
                    ],
                    action: ConfirmAction::DeleteSave(folder),
                    danger: true,
                });
            }
            ContentKind::ResourcePacks => {
                let Some(pack) = self.selected_resourcepack() else {
                    self.toast(
                        LogLevel::Warn,
                        lang.pick("No resource pack selected", "未选择资源包"),
                    );
                    return;
                };
                let file = pack.file.clone();
                self.modal = Some(Modal::Confirm {
                    title: lang.pick("Delete resource pack", "删除资源包").into(),
                    lines: vec![
                        if lang == Lang::Chinese {
                            format!("从实例中删除 {file}？")
                        } else {
                            format!("Remove {file} from this instance?")
                        },
                        lang.pick(
                            "It is also removed from options.txt.",
                            "它也会从 options.txt 中移除。",
                        )
                        .into(),
                    ],
                    action: ConfirmAction::DeleteResourcePack(file),
                    danger: true,
                });
            }
            ContentKind::Shaders => {
                let Some(shader) = self.selected_shader() else {
                    self.toast(
                        LogLevel::Warn,
                        lang.pick("No shader pack selected", "未选择光影包"),
                    );
                    return;
                };
                let file = shader.file.clone();
                self.modal = Some(Modal::Confirm {
                    title: lang.pick("Delete shader pack", "删除光影包").into(),
                    lines: vec![
                        if lang == Lang::Chinese {
                            format!("从实例中删除 {file}？")
                        } else {
                            format!("Remove {file} from this instance?")
                        },
                        lang.pick(
                            "It is disabled in options.txt as well.",
                            "同时会从 options.txt 中禁用。",
                        )
                        .into(),
                    ],
                    action: ConfirmAction::DeleteShader(file),
                    danger: true,
                });
            }
        }
    }

    fn open_content_dir(&mut self) {
        let lang = self.lang();
        let Some(version) = self.settings.active_version.clone() else {
            self.toast(
                LogLevel::Warn,
                lang.pick(
                    "Select an installed version first",
                    "请先选择一个已安装版本",
                ),
            );
            return;
        };
        let manager = crate::content::ContentManager::new(&self.game_dir);
        let dir = match self.content_kind {
            ContentKind::Saves => manager.saves_dir(&version),
            ContentKind::ResourcePacks => manager.resourcepacks_dir(&version),
            ContentKind::Shaders => manager.shaderpacks_dir(&version),
        };
        match dir {
            Ok(path) => {
                let _ = std::fs::create_dir_all(&path);
                open_path(&path);
                self.toast(LogLevel::Info, path.display().to_string());
            }
            Err(error) => self.log_and_toast(LogLevel::Error, error.to_string()),
        }
    }

    fn begin_backup_save(&mut self, folder: String) {
        let lang = self.lang();
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                lang.pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return;
        }
        let Some(version) = self.settings.active_version.clone() else {
            self.toast(
                LogLevel::Warn,
                lang.pick(
                    "Select an installed version first",
                    "请先选择一个已安装版本",
                ),
            );
            return;
        };
        let id = self.next_job_id;
        self.next_job_id += 1;
        self.pending_activate = None;
        self.pending_loader = None;
        self.pending_update_apply = false;
        self.show_job_panel = true;
        self.job = Some(JobView::new(
            id,
            if lang == Lang::Chinese {
                format!("备份存档 {folder}")
            } else {
                format!("Backup world {folder}")
            },
        ));
        progress::reset_cancel();
        progress::set_job(id);
        jobs::start_backup_save(self.tx.clone(), id, self.game_dir.clone(), version, folder);
    }

    fn begin_import_save(&mut self, archive: std::path::PathBuf) {
        let lang = self.lang();
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                lang.pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return;
        }
        let Some(version) = self.settings.active_version.clone() else {
            self.toast(
                LogLevel::Warn,
                lang.pick(
                    "Select an installed version first",
                    "请先选择一个已安装版本",
                ),
            );
            return;
        };
        let label = archive
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "world.zip".into());
        let id = self.next_job_id;
        self.next_job_id += 1;
        self.pending_activate = None;
        self.pending_loader = None;
        self.pending_update_apply = false;
        self.show_job_panel = true;
        self.job = Some(JobView::new(
            id,
            if lang == Lang::Chinese {
                format!("导入存档 {label}")
            } else {
                format!("Import world {label}")
            },
        ));
        progress::reset_cancel();
        progress::set_job(id);
        jobs::start_import_save(self.tx.clone(), id, self.game_dir.clone(), version, archive);
    }

    fn begin_import_resourcepack(&mut self, source: std::path::PathBuf) {
        let lang = self.lang();
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                lang.pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return;
        }
        let Some(version) = self.settings.active_version.clone() else {
            self.toast(
                LogLevel::Warn,
                lang.pick(
                    "Select an installed version first",
                    "请先选择一个已安装版本",
                ),
            );
            return;
        };
        let label = source
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "pack.zip".into());
        let id = self.next_job_id;
        self.next_job_id += 1;
        self.pending_activate = None;
        self.pending_loader = None;
        self.pending_update_apply = false;
        self.show_job_panel = true;
        self.job = Some(JobView::new(
            id,
            if lang == Lang::Chinese {
                format!("导入资源包 {label}")
            } else {
                format!("Import resource pack {label}")
            },
        ));
        progress::reset_cancel();
        progress::set_job(id);
        jobs::start_import_resourcepack(
            self.tx.clone(),
            id,
            self.game_dir.clone(),
            version,
            source,
        );
    }

    fn begin_import_shader(&mut self, source: std::path::PathBuf) {
        let lang = self.lang();
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                lang.pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return;
        }
        let Some(version) = self.settings.active_version.clone() else {
            self.toast(
                LogLevel::Warn,
                lang.pick(
                    "Select an installed version first",
                    "请先选择一个已安装版本",
                ),
            );
            return;
        };
        let label = source
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "shader.zip".into());
        let id = self.next_job_id;
        self.next_job_id += 1;
        self.pending_activate = None;
        self.pending_loader = None;
        self.pending_update_apply = false;
        self.show_job_panel = true;
        self.job = Some(JobView::new(
            id,
            if lang == Lang::Chinese {
                format!("导入光影包 {label}")
            } else {
                format!("Import shader pack {label}")
            },
        ));
        progress::reset_cancel();
        progress::set_job(id);
        jobs::start_import_shader(self.tx.clone(), id, self.game_dir.clone(), version, source);
    }

    fn begin_install_resourcepack(&mut self, slug: String, title: String) {
        let lang = self.lang();
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                lang.pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return;
        }
        let Some(version) = self.settings.active_version.clone() else {
            return;
        };
        let id = self.next_job_id;
        self.next_job_id += 1;
        self.pending_activate = None;
        self.pending_loader = None;
        self.pending_update_apply = false;
        self.pending_shader_loader = None;
        self.show_job_panel = true;
        self.job = Some(JobView::new(
            id,
            if lang == Lang::Chinese {
                format!("安装资源包 {title}")
            } else {
                format!("Install resource pack {title}")
            },
        ));
        progress::reset_cancel();
        progress::set_job(id);
        jobs::start_install_resourcepack(
            self.tx.clone(),
            id,
            self.game_dir.clone(),
            version,
            slug,
            title,
        );
    }

    fn begin_install_shader(&mut self, slug: String, title: String, support: ShaderSupport) {
        let lang = self.lang();
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                lang.pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return;
        }
        let Some(version) = self.settings.active_version.clone() else {
            return;
        };
        let id = self.next_job_id;
        self.next_job_id += 1;
        self.pending_activate = None;
        self.pending_loader = None;
        self.pending_update_apply = false;
        self.pending_shader_loader = Some((title.clone(), support));
        self.show_job_panel = true;
        self.job = Some(JobView::new(
            id,
            if lang == Lang::Chinese {
                format!("安装光影包 {title}")
            } else {
                format!("Install shader pack {title}")
            },
        ));
        progress::reset_cancel();
        progress::set_job(id);
        jobs::start_install_shader(
            self.tx.clone(),
            id,
            self.game_dir.clone(),
            version,
            slug,
            title,
        );
    }

    fn begin_optimize_mods(&mut self) {
        let lang = self.lang();
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                lang.pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return;
        }
        let Some(version) = self.settings.active_version.clone() else {
            return;
        };
        let loader = self.active_loader_id().or_else(|| {
            InstanceManager::new(&self.game_dir)
                .loaders(&version)
                .first()
                .cloned()
        });
        let Some(loader) = loader else {
            self.toast(
                LogLevel::Warn,
                lang.pick(
                    "Install Fabric, NeoForge or Forge before adding optimization mods",
                    "请先安装 Fabric、NeoForge 或 Forge 再添加优化模组",
                ),
            );
            return;
        };
        let id = self.next_job_id;
        self.next_job_id += 1;
        self.pending_activate = None;
        self.pending_loader = None;
        self.pending_update_apply = false;
        self.pending_shader_loader = None;
        self.show_job_panel = true;
        self.job = Some(JobView::new(
            id,
            if lang == Lang::Chinese {
                format!("安装优化模组 · {version}")
            } else {
                format!("Install optimization mods · {version}")
            },
        ));
        progress::reset_cancel();
        progress::set_job(id);
        jobs::start_optimize_mods(
            self.tx.clone(),
            id,
            self.game_dir.clone(),
            self.settings.clone(),
            version,
            loader,
        );
    }

    fn prompt_shader_loader(&mut self, title: String, support: ShaderSupport) {
        let lang = self.lang();
        let Some(version) = self.settings.active_version.clone() else {
            return;
        };
        let status =
            crate::content::ContentManager::new(&self.game_dir).shader_loader_status(&version);
        let loader = self.active_loader_id().or_else(|| {
            InstanceManager::new(&self.game_dir)
                .loaders(&version)
                .first()
                .cloned()
        });
        if support.iris && !status.iris && !(support.optifine && status.optifine) {
            match loader
                .as_deref()
                .and_then(crate::discover::shader_loader_mod)
            {
                Some((mod_slug, mod_title)) => {
                    let mut lines = vec![
                        if lang == Lang::Chinese {
                            format!("{} 支持 {}", title, support.summary(true))
                        } else {
                            format!("{} supports {}.", title, support.summary(false))
                        },
                        if lang == Lang::Chinese {
                            format!("当前实例尚未安装 Iris（{}）。", version)
                        } else {
                            format!("Iris is not installed for {}.", version)
                        },
                        lang.pick(
                            "Install Iris now? Required dependencies are installed automatically.",
                            "现在安装 Iris？必需依赖会自动安装。",
                        )
                        .into(),
                    ];
                    if support.optifine && !status.optifine {
                        lines.push(
                            lang.pick(
                                "OptiFine is also supported; install it manually if you prefer it.",
                                "也支持 OptiFine；如果偏好 OptiFine 可手动安装。",
                            )
                            .into(),
                        );
                    }
                    self.modal = Some(Modal::Confirm {
                        title: lang.pick("Install shader loader", "安装光影加载器").into(),
                        lines,
                        action: ConfirmAction::InstallMod {
                            slug: mod_slug.to_string(),
                            title: mod_title.to_string(),
                        },
                        danger: false,
                    });
                }
                None => self.log_and_toast(
                    LogLevel::Warn,
                    lang.pick(
                        "Install Fabric, NeoForge or Forge first to use Iris.",
                        "请先安装 Fabric、NeoForge 或 Forge 再使用 Iris。",
                    ),
                ),
            }
            return;
        }
        if !support.iris && support.optifine && !status.optifine {
            self.modal = Some(Modal::Confirm {
                title: lang.pick("OptiFine required", "需要 OptiFine").into(),
                lines: vec![
                    if lang == Lang::Chinese {
                        format!("{} 仅支持 OptiFine。", title)
                    } else {
                        format!("{} supports OptiFine only.", title)
                    },
                    lang.pick(
                        "Download and install OptiFine manually from https://optifine.net/downloads; the launcher does not run its installer.",
                        "请从 https://optifine.net/downloads 手动下载并安装 OptiFine；启动器不会运行它的安装器。",
                    )
                    .into(),
                ],
                action: ConfirmAction::Dismiss,
                danger: false,
            });
            return;
        }
        if support.is_empty() {
            self.log_and_toast(
                LogLevel::Warn,
                lang.pick(
                    "Shader loader support is unknown; check the project page.",
                    "光影加载器支持未知；请查看项目页面。",
                ),
            );
        }
    }

    fn handle_accounts_key(&mut self, key: KeyEvent) -> AppCommand {
        match key.code {
            KeyCode::Up => {
                if !self.accounts.is_empty() {
                    self.account_sel = self.account_sel.saturating_sub(1);
                    self.account_state.select(Some(self.account_sel));
                }
                AppCommand::None
            }
            KeyCode::Down => {
                if !self.accounts.is_empty() {
                    self.account_sel = (self.account_sel + 1).min(self.accounts.len() - 1);
                    self.account_state.select(Some(self.account_sel));
                }
                AppCommand::None
            }
            KeyCode::Char('a') => {
                self.open_offline_input();
                AppCommand::None
            }
            KeyCode::Char('m') => {
                if self.job.as_ref().is_some_and(JobView::is_running) {
                    self.toast(
                        LogLevel::Warn,
                        self.lang()
                            .pick("Wait for the current task to finish", "请等待当前任务完成"),
                    );
                } else {
                    self.modal = Some(Modal::Login { selected: 0 });
                }
                AppCommand::None
            }
            KeyCode::Char('d') => {
                self.set_selected_default();
                AppCommand::None
            }
            KeyCode::Char('x') => {
                let lang = self.lang();
                self.modal = Some(Modal::Confirm {
                    title: lang.pick("Log out", "登出").into(),
                    lines: vec![
                        lang.pick(
                            "Remove all local accounts and stored Microsoft tokens?",
                            "移除所有本地账号和已保存的 Microsoft 令牌？",
                        )
                        .into(),
                        lang.pick(
                            "This does not revoke the session on Microsoft's servers.",
                            "这不会在 Microsoft 服务器端撤销会话。",
                        )
                        .into(),
                    ],
                    action: ConfirmAction::Logout,
                    danger: true,
                });
                AppCommand::None
            }
            KeyCode::Char('r') => {
                self.refresh_accounts();
                self.log_and_toast(
                    LogLevel::Info,
                    self.lang().pick("Accounts refreshed", "账号已刷新"),
                );
                AppCommand::None
            }
            _ => AppCommand::None,
        }
    }

    fn handle_settings_key(&mut self, key: KeyEvent) -> AppCommand {
        match key.code {
            KeyCode::Up => {
                self.settings_cursor = self.settings_cursor.saturating_sub(1);
                AppCommand::None
            }
            KeyCode::Down => {
                self.settings_cursor = (self.settings_cursor + 1).min(SETTINGS_ROWS - 1);
                AppCommand::None
            }
            KeyCode::Left => {
                self.settings_adjust(-1);
                AppCommand::None
            }
            KeyCode::Right => {
                self.settings_adjust(1);
                AppCommand::None
            }
            KeyCode::Enter => {
                self.settings_activate();
                AppCommand::None
            }
            KeyCode::Char('s') => {
                self.save_settings();
                AppCommand::None
            }
            KeyCode::Char('R') => {
                let lang = self.lang();
                self.modal = Some(Modal::Confirm {
                    title: lang.pick("Reset settings", "重置设置").into(),
                    lines: vec![
                        lang.pick(
                            "Restore every TUI preference to its default?",
                            "将所有 TUI 偏好恢复为默认值？",
                        )
                        .into(),
                        lang.pick("Your active version is kept.", "活跃版本会保留。")
                            .into(),
                    ],
                    action: ConfirmAction::ResetSettings,
                    danger: false,
                });
                AppCommand::None
            }
            _ => AppCommand::None,
        }
    }

    fn handle_about_key(&mut self, key: KeyEvent) -> AppCommand {
        match key.code {
            KeyCode::Char('g') => open_url(crate::GITHUB_REPO),
            KeyCode::Char('b') => open_url(crate::CODEBERG_REPO),
            _ => {}
        }
        AppCommand::None
    }

    fn handle_logs_key(&mut self, key: KeyEvent) -> AppCommand {
        let page = 12;
        match key.code {
            KeyCode::Up => {
                self.log_follow = false;
                self.log_offset = (self.log_offset + 1).min(self.logs.len().saturating_sub(1));
                AppCommand::None
            }
            KeyCode::Down => {
                self.log_offset = self.log_offset.saturating_sub(1);
                if self.log_offset == 0 {
                    self.log_follow = true;
                }
                AppCommand::None
            }
            KeyCode::PageUp => {
                self.log_follow = false;
                self.log_offset = (self.log_offset + page).min(self.logs.len().saturating_sub(1));
                AppCommand::None
            }
            KeyCode::PageDown => {
                self.log_offset = self.log_offset.saturating_sub(page);
                if self.log_offset == 0 {
                    self.log_follow = true;
                }
                AppCommand::None
            }
            KeyCode::Char('g') => {
                self.log_follow = false;
                self.log_offset = self.logs.len().saturating_sub(1);
                AppCommand::None
            }
            KeyCode::Char('G') => {
                self.log_follow = true;
                self.log_offset = 0;
                AppCommand::None
            }
            KeyCode::Char('f') => {
                self.log_follow = !self.log_follow;
                if self.log_follow {
                    self.log_offset = 0;
                }
                AppCommand::None
            }
            KeyCode::Char('c') => {
                let lang = self.lang();
                self.modal = Some(Modal::Confirm {
                    title: lang.pick("Clear activity", "清空活动").into(),
                    lines: vec![lang
                        .pick(
                            "Clear all entries from the activity log?",
                            "清空活动日志中的所有条目？",
                        )
                        .into()],
                    action: ConfirmAction::ClearLogs,
                    danger: false,
                });
                AppCommand::None
            }
            _ => AppCommand::None,
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> AppCommand {
        if key.code == KeyCode::Esc {
            self.search.open = false;
            return AppCommand::None;
        }
        if key.code == KeyCode::Tab {
            self.search.focus = match self.search.focus {
                SearchFocus::Input => SearchFocus::Results,
                SearchFocus::Results => SearchFocus::Input,
            };
            return AppCommand::None;
        }
        match self.search.focus {
            SearchFocus::Input => match key.code {
                KeyCode::Enter => self.submit_search(),
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.search.input.clear();
                }
                KeyCode::Char(ch) if text_modifier(key.modifiers) => {
                    self.search.input.insert_char(ch);
                }
                KeyCode::Backspace => self.search.input.backspace(),
                KeyCode::Delete => self.search.input.delete(),
                KeyCode::Left => self.search.input.left(),
                KeyCode::Right => self.search.input.right(),
                KeyCode::Home => self.search.input.home(),
                KeyCode::End => self.search.input.end(),
                KeyCode::Up | KeyCode::Down if !self.search.results.is_empty() => {
                    self.search.focus = SearchFocus::Results;
                }
                _ => {}
            },
            SearchFocus::Results => match key.code {
                KeyCode::Up => {
                    if !self.search.results.is_empty() {
                        self.search.selected = self.search.selected.saturating_sub(1);
                        self.search.list_state.select(Some(self.search.selected));
                    }
                }
                KeyCode::Down => {
                    if !self.search.results.is_empty() {
                        self.search.selected =
                            (self.search.selected + 1).min(self.search.results.len() - 1);
                        self.search.list_state.select(Some(self.search.selected));
                    }
                }
                KeyCode::Enter => self.confirm_search_install(),
                KeyCode::Char('o') => {
                    if let Some(hit) = self.search.results.get(self.search.selected) {
                        let path = match self.search.kind {
                            DiscoverKind::Mods => "mod",
                            DiscoverKind::ResourcePacks => "resourcepack",
                            DiscoverKind::Shaders => "shader",
                        };
                        open_url(&format!("https://modrinth.com/{path}/{}", hit.slug));
                    }
                }
                KeyCode::Char('[') => self.page_search(-1),
                KeyCode::Char(']') => self.page_search(1),
                KeyCode::Char('b') => self.browse_search(),
                KeyCode::Char('c') => self.cycle_search_category(),
                KeyCode::Char('r') => self.request_discover(),
                KeyCode::Char('i') | KeyCode::Char('/') => self.search.focus = SearchFocus::Input,
                _ => {}
            },
        }
        AppCommand::None
    }

    fn handle_modal_key(&mut self, key: KeyEvent) -> AppCommand {
        let Some(mut modal) = self.modal.take() else {
            return AppCommand::None;
        };
        // Keep enough of an input dialog to restore it verbatim when validation
        // fails, so the user can correct the value instead of losing context.
        let input_snapshot = match &modal {
            Modal::Input {
                title,
                hint,
                input,
                action,
                ..
            } => Some((
                title.clone(),
                hint.clone(),
                *action,
                input.value().to_string(),
            )),
            _ => None,
        };
        let mut request: Option<ModalRequest> = None;
        let mut close = false;
        match &mut modal {
            Modal::Help { scroll } => match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => close = true,
                KeyCode::Up => *scroll = scroll.saturating_sub(1),
                KeyCode::Down => *scroll = scroll.saturating_add(1),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(8),
                KeyCode::PageDown => *scroll = scroll.saturating_add(8),
                _ => {}
            },
            Modal::Confirm { action, .. } => match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    request = Some(ModalRequest::Confirm(action.clone()));
                    close = true;
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => close = true,
                _ => {}
            },
            Modal::Input { input, action, .. } => match key.code {
                KeyCode::Esc => close = true,
                KeyCode::Enter => {
                    request = Some(ModalRequest::Input(*action, input.value().to_string()));
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.kill_to_start();
                }
                KeyCode::Char(ch) if text_modifier(key.modifiers) => input.insert_char(ch),
                KeyCode::Backspace => input.backspace(),
                KeyCode::Delete => input.delete(),
                KeyCode::Left => input.left(),
                KeyCode::Right => input.right(),
                KeyCode::Home => input.home(),
                KeyCode::End => input.end(),
                _ => {}
            },
            Modal::Picker(picker) => match key.code {
                KeyCode::Esc => close = true,
                KeyCode::Up => picker.move_selection(-1),
                KeyCode::Down => picker.move_selection(1),
                KeyCode::PageUp => picker.page(-1),
                KeyCode::PageDown => picker.page(1),
                KeyCode::Enter => {
                    if let Some(item) = picker.selected_item() {
                        request = Some(ModalRequest::Pick(picker.kind, item.id.clone()));
                        close = true;
                    }
                }
                KeyCode::Char('r') if picker.kind == PickerKind::RemoteVersions => {
                    request = Some(ModalRequest::ReloadManifest);
                }
                KeyCode::Backspace => {
                    picker.input.backspace();
                    picker.refilter();
                }
                KeyCode::Delete => {
                    picker.input.delete();
                    picker.refilter();
                }
                KeyCode::Left => picker.input.left(),
                KeyCode::Right => picker.input.right(),
                KeyCode::Home => picker.input.home(),
                KeyCode::End => picker.input.end(),
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    picker.input.kill_to_start();
                    picker.refilter();
                }
                KeyCode::Char(ch) if text_modifier(key.modifiers) => {
                    picker.input.insert_char(ch);
                    picker.refilter();
                }
                _ => {}
            },
            Modal::Login { selected } => match key.code {
                KeyCode::Esc => close = true,
                KeyCode::Up | KeyCode::Left => *selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Right => *selected = (*selected + 1).min(1),
                KeyCode::Enter => {
                    let mode = if *selected == 0 {
                        LoginMode::Browser
                    } else {
                        LoginMode::DeviceCode
                    };
                    request = Some(ModalRequest::Login(mode));
                    close = true;
                }
                _ => {}
            },
        }
        if !close {
            self.modal = Some(modal);
        }
        match request {
            Some(ModalRequest::Confirm(action)) => {
                let suspend = self.apply_confirm(action);
                return suspend;
            }
            Some(ModalRequest::Input(action, value)) => match self.apply_input(action, &value) {
                Ok(()) => {
                    self.modal = None;
                }
                Err(error) => {
                    if let Some((title, hint, action, value)) = input_snapshot {
                        self.modal = Some(Modal::Input {
                            title,
                            hint,
                            input: TextInput::new(value),
                            action,
                            error: Some(error),
                        });
                    } else {
                        self.modal = Some(Modal::Input {
                            title: "Input".into(),
                            hint: String::new(),
                            input: TextInput::new(value),
                            action,
                            error: Some(error),
                        });
                    }
                }
            },
            Some(ModalRequest::Pick(kind, id)) => {
                self.apply_pick(kind, &id);
            }
            Some(ModalRequest::ReloadManifest) => {
                let lang = self.lang();
                self.modal = Some(Modal::Picker(PickerState::new(
                    lang.pick("Download Minecraft", "下载 Minecraft"),
                    lang.pick(
                        "Pick a version to download · r refresh · esc close",
                        "选择要下载的版本 · r 刷新 · esc 关闭",
                    ),
                    PickerKind::RemoteVersions,
                    Vec::new(),
                )));
                self.request_manifest(true);
            }
            Some(ModalRequest::Login(mode)) => {
                return AppCommand::Suspend(SuspendTask::Login(mode));
            }
            None => {}
        }
        AppCommand::None
    }

    fn request_manifest(&mut self, force: bool) {
        if !force {
            if let Some(items) = self.remote_versions.clone() {
                if let Some(Modal::Picker(picker)) = self.modal.as_mut() {
                    picker.set_items(items);
                    picker.loading = false;
                    picker.error = None;
                }
                return;
            }
        }
        let request = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.manifest_request = request;
        if let Some(Modal::Picker(picker)) = self.modal.as_mut() {
            picker.loading = true;
            picker.error = None;
        }
        jobs::start_manifest(self.tx.clone(), request, self.game_dir.clone());
    }

    fn apply_confirm(&mut self, action: ConfirmAction) -> AppCommand {
        match action {
            ConfirmAction::Quit => {
                progress::request_cancel();
                self.quit = true;
                AppCommand::None
            }
            ConfirmAction::UninstallMod(file) => {
                if let Some(version) = self.settings.active_version.clone() {
                    let removed = ModManager::new(&self.game_dir).uninstall_mod(&file, &version);
                    if removed.is_empty() {
                        self.log_and_toast(
                            LogLevel::Error,
                            self.lang()
                                .pick("Could not remove that mod", "无法移除该模组"),
                        );
                    } else {
                        self.refresh_mods();
                        let message = if self.lang() == Lang::Chinese {
                            format!("已移除 {}", removed.join(", "))
                        } else {
                            format!("Removed {}", removed.join(", "))
                        };
                        self.log_and_toast(LogLevel::Ok, message);
                    }
                }
                AppCommand::None
            }
            ConfirmAction::Logout => {
                match AccountManager::logout_metadata(&self.game_dir) {
                    Ok(()) => {
                        self.refresh_accounts();
                        self.log_and_toast(
                            LogLevel::Ok,
                            self.lang().pick("Local accounts cleared", "本地账号已清除"),
                        );
                    }
                    Err(error) => {
                        let prefix = self.lang().pick("Logout", "登出");
                        self.log_and_toast(LogLevel::Error, format!("{prefix}: {error}"))
                    }
                }
                AppCommand::None
            }
            ConfirmAction::ClearLogs => {
                self.logs.clear();
                self.log_offset = 0;
                self.log_follow = true;
                self.toast(
                    LogLevel::Ok,
                    self.lang().pick("Activity cleared", "活动日志已清空"),
                );
                AppCommand::None
            }
            ConfirmAction::DownloadVersion(version) => {
                self.begin_download(version);
                AppCommand::None
            }
            ConfirmAction::InstallLoader(loader) => {
                self.begin_loader_install(loader);
                AppCommand::None
            }
            ConfirmAction::InstallMod { slug, title } => {
                self.begin_mod_install(slug, title);
                AppCommand::None
            }
            ConfirmAction::ResetSettings => {
                self.reset_settings();
                AppCommand::None
            }
            ConfirmAction::RemoveVersion(version) => {
                if self.job.as_ref().is_some_and(JobView::is_running) {
                    self.toast(
                        LogLevel::Warn,
                        self.lang()
                            .pick("Wait for the current task to finish", "请等待当前任务完成"),
                    );
                    return AppCommand::None;
                }
                let lang = self.lang();
                let id = self.next_job_id;
                self.next_job_id += 1;
                self.show_job_panel = true;
                self.job = Some(JobView::new(
                    id,
                    if lang == Lang::Chinese {
                        format!("删除 {version}")
                    } else {
                        format!("Remove {version}")
                    },
                ));
                progress::reset_cancel();
                progress::set_job(id);
                jobs::start_remove_version(
                    self.tx.clone(),
                    id,
                    self.game_dir.clone(),
                    version.clone(),
                );
                if self.settings.active_version.as_deref() == Some(version.as_str()) {
                    self.settings.active_version = None;
                    self.save_settings_silent();
                }
                AppCommand::None
            }
            ConfirmAction::ApplyUpdates => {
                self.begin_apply_updates();
                AppCommand::None
            }
            ConfirmAction::DeleteSave(folder) => {
                let lang = self.lang();
                let Some(version) = self.settings.active_version.clone() else {
                    return AppCommand::None;
                };
                match crate::content::ContentManager::new(&self.game_dir)
                    .delete_save(&version, &folder)
                {
                    Ok(freed) => {
                        self.refresh_content();
                        let message = if lang == Lang::Chinese {
                            format!("已删除存档 {folder}，释放 {}", theme::format_bytes(freed))
                        } else {
                            format!(
                                "Deleted world {folder} ({} freed)",
                                theme::format_bytes(freed)
                            )
                        };
                        self.log_and_toast(LogLevel::Ok, message);
                    }
                    Err(error) => {
                        let prefix = lang.pick("Delete failed", "删除失败");
                        self.log_and_toast(LogLevel::Error, format!("{prefix}: {error}"));
                    }
                }
                AppCommand::None
            }
            ConfirmAction::DeleteResourcePack(file) => {
                let lang = self.lang();
                let Some(version) = self.settings.active_version.clone() else {
                    return AppCommand::None;
                };
                match crate::content::ContentManager::new(&self.game_dir)
                    .delete_resourcepack(&version, &file)
                {
                    Ok(freed) => {
                        self.refresh_content();
                        let message = if lang == Lang::Chinese {
                            format!("已删除资源包 {file}，释放 {}", theme::format_bytes(freed))
                        } else {
                            format!(
                                "Deleted resource pack {file} ({} freed)",
                                theme::format_bytes(freed)
                            )
                        };
                        self.log_and_toast(LogLevel::Ok, message);
                    }
                    Err(error) => {
                        let prefix = lang.pick("Delete failed", "删除失败");
                        self.log_and_toast(LogLevel::Error, format!("{prefix}: {error}"));
                    }
                }
                AppCommand::None
            }
            ConfirmAction::DeleteShader(file) => {
                let lang = self.lang();
                let Some(version) = self.settings.active_version.clone() else {
                    return AppCommand::None;
                };
                match crate::content::ContentManager::new(&self.game_dir)
                    .delete_shader(&version, &file)
                {
                    Ok(freed) => {
                        self.refresh_content();
                        let message = if lang == Lang::Chinese {
                            format!("已删除光影包 {file}，释放 {}", theme::format_bytes(freed))
                        } else {
                            format!(
                                "Deleted shader pack {file} ({} freed)",
                                theme::format_bytes(freed)
                            )
                        };
                        self.log_and_toast(LogLevel::Ok, message);
                    }
                    Err(error) => {
                        let prefix = lang.pick("Delete failed", "删除失败");
                        self.log_and_toast(LogLevel::Error, format!("{prefix}: {error}"));
                    }
                }
                AppCommand::None
            }
            ConfirmAction::InstallResourcePack { slug, title } => {
                self.begin_install_resourcepack(slug, title);
                AppCommand::None
            }
            ConfirmAction::InstallShader {
                slug,
                title,
                support,
            } => {
                self.begin_install_shader(slug, title, support);
                AppCommand::None
            }
            ConfirmAction::InstallOptimizations => {
                self.begin_optimize_mods();
                AppCommand::None
            }
            ConfirmAction::Dismiss => AppCommand::None,
        }
    }

    fn apply_input(&mut self, action: InputAction, value: &str) -> Result<(), String> {
        match action {
            InputAction::OfflineName => {
                let name = value.trim();
                AccountManager::set_offline_metadata(&self.game_dir, name)
                    .map_err(|e| e.to_string())?;
                self.refresh_accounts();
                let message = if self.lang() == Lang::Chinese {
                    format!("离线账号「{name}」已保存")
                } else {
                    format!("Offline account '{name}' saved")
                };
                self.log_and_toast(LogLevel::Ok, message);
                Ok(())
            }
            InputAction::JavaPath => {
                let lang = self.lang();
                let (resolved, major) = resolve_java_setting(value.trim(), lang)?;
                self.settings.java = resolved.clone();
                self.settings.normalize();
                self.settings_autosave_blocked = false;
                self.save_settings_silent();
                let message = if resolved.is_empty() {
                    lang.pick("Java will be auto-detected", "Java 将自动检测")
                        .to_string()
                } else if let Some(major) = major {
                    if lang == Lang::Chinese {
                        format!("已选择 Java {major}：{resolved}")
                    } else {
                        format!("Java {major} selected: {resolved}")
                    }
                } else if lang == Lang::Chinese {
                    format!("Java 路径已设为 {resolved}")
                } else {
                    format!("Java path set to {resolved}")
                };
                self.toast(LogLevel::Ok, message);
                Ok(())
            }
            InputAction::JvmArgs => {
                let lang = self.lang();
                let trimmed = value.trim();
                let args = crate::split_args(trimmed)?;
                self.settings.jvm_args = trimmed.to_string();
                self.settings.normalize();
                self.settings_autosave_blocked = false;
                self.save_settings_silent();
                let message = if args.is_empty() {
                    lang.pick("JVM arguments cleared", "JVM 参数已清空")
                        .to_string()
                } else if lang == Lang::Chinese {
                    format!("已保存 {} 个 JVM 参数", args.len())
                } else {
                    format!("{} JVM argument(s) saved", args.len())
                };
                self.toast(LogLevel::Ok, message);
                Ok(())
            }
            InputAction::RenameVersion => {
                let lang = self.lang();
                let from = self
                    .rename_source
                    .clone()
                    .ok_or_else(|| lang.pick("No instance selected", "未选择实例").to_string())?;
                let to = value.trim();
                crate::util::validate_component(to)?;
                if from == to {
                    self.rename_source = None;
                    return Ok(());
                }
                InstanceManager::new(&self.game_dir)
                    .rename(&from, to)
                    .map_err(|error| error.to_string())?;
                self.rename_source = None;
                if self.settings.active_version.as_deref() == Some(from.as_str()) {
                    self.settings.active_version = Some(to.to_string());
                    self.save_settings_silent();
                }
                self.refresh_local();
                let message = if lang == Lang::Chinese {
                    format!("实例已重命名：{from} -> {to}")
                } else {
                    format!("Instance renamed: {from} -> {to}")
                };
                self.log_and_toast(LogLevel::Ok, message);
                Ok(())
            }
            InputAction::CopyVersion => {
                let lang = self.lang();
                let from = self
                    .copy_source
                    .clone()
                    .ok_or_else(|| lang.pick("No instance selected", "未选择实例").to_string())?;
                let to = value.trim().to_string();
                crate::util::validate_component(&to)?;
                if self.job.as_ref().is_some_and(JobView::is_running) {
                    return Err(lang
                        .pick("Wait for the current task to finish", "请等待当前任务完成")
                        .to_string());
                }
                self.copy_source = None;
                let id = self.next_job_id;
                self.next_job_id += 1;
                self.show_job_panel = true;
                self.job = Some(JobView::new(
                    id,
                    if lang == Lang::Chinese {
                        format!("复制 {from} -> {to}")
                    } else {
                        format!("Copy {from} -> {to}")
                    },
                ));
                progress::reset_cancel();
                progress::set_job(id);
                jobs::start_copy_version(self.tx.clone(), id, self.game_dir.clone(), from, to);
                Ok(())
            }
            InputAction::ImportSave => {
                let lang = self.lang();
                let raw = value.trim();
                if raw.is_empty() {
                    return Err(lang
                        .pick("World archive path is empty", "存档压缩包路径为空")
                        .to_string());
                }
                let path = crate::util::absolute_path(std::path::Path::new(raw))
                    .map_err(|error| error.to_string())?;
                if !path.is_file() {
                    return Err(if lang == Lang::Chinese {
                        format!("找不到存档压缩包：{}", path.display())
                    } else {
                        format!("World archive not found: {}", path.display())
                    });
                }
                self.begin_import_save(path);
                Ok(())
            }
            InputAction::ImportResourcePack => {
                let lang = self.lang();
                let raw = value.trim();
                if raw.is_empty() {
                    return Err(lang
                        .pick("Resource pack path is empty", "资源包路径为空")
                        .to_string());
                }
                let path = crate::util::absolute_path(std::path::Path::new(raw))
                    .map_err(|error| error.to_string())?;
                if !path.exists() {
                    return Err(if lang == Lang::Chinese {
                        format!("找不到资源包：{}", path.display())
                    } else {
                        format!("Resource pack not found: {}", path.display())
                    });
                }
                self.begin_import_resourcepack(path);
                Ok(())
            }
            InputAction::ImportShader => {
                let lang = self.lang();
                let raw = value.trim();
                if raw.is_empty() {
                    return Err(lang
                        .pick("Shader pack path is empty", "光影包路径为空")
                        .to_string());
                }
                let path = crate::util::absolute_path(std::path::Path::new(raw))
                    .map_err(|error| error.to_string())?;
                if !path.exists() {
                    return Err(if lang == Lang::Chinese {
                        format!("找不到光影包：{}", path.display())
                    } else {
                        format!("Shader pack not found: {}", path.display())
                    });
                }
                self.begin_import_shader(path);
                Ok(())
            }
            InputAction::ExportModpack => {
                let lang = self.lang();
                let id = self
                    .export_source
                    .clone()
                    .ok_or_else(|| lang.pick("No instance selected", "未选择实例").to_string())?;
                let raw = value.trim();
                if raw.is_empty() {
                    return Err(lang
                        .pick("Output path is empty", "输出路径为空")
                        .to_string());
                }
                let mut path = std::path::PathBuf::from(raw);
                if !path.is_absolute() {
                    path = self.game_dir.join(path);
                }
                if !path
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .ends_with(".mrpack")
                {
                    path.set_extension("mrpack");
                }
                self.begin_export_modpack(id, path);
                self.export_source = None;
                Ok(())
            }
        }
    }

    fn apply_pick(&mut self, kind: PickerKind, id: &str) -> AppCommand {
        match kind {
            PickerKind::RemoteVersions => {
                let installed = self.versions.iter().any(|row| row.id == id);
                let lang = self.lang();
                self.modal = Some(Modal::Confirm {
                    title: format!("Minecraft {id}"),
                    lines: vec![
                        if installed {
                            lang.pick(
                                "This version is already installed; missing files will be verified.",
                                "该版本已安装，将校验缺失文件。",
                            )
                        } else {
                            lang.pick(
                                "Download the client, libraries and assets now?",
                                "现在下载客户端、库和资源文件？",
                            )
                        }
                        .into(),
                        lang.pick(
                            "Progress is shown below the screen while it runs.",
                            "运行进度会显示在界面底部。",
                        )
                        .into(),
                    ],
                    action: ConfirmAction::DownloadVersion(id.to_string()),
                    danger: false,
                });
            }
            PickerKind::Loaders => {
                if id == "auto" {
                    self.settings.active_loader = None;
                    self.settings_autosave_blocked = false;
                    self.save_settings_silent();
                    self.log_and_toast(
                        LogLevel::Info,
                        self.lang().pick("Launch loader: Auto", "启动加载器：自动"),
                    );
                } else if self.loader_installed(id) {
                    self.settings.active_loader = Some(id.to_string());
                    self.settings_autosave_blocked = false;
                    self.save_settings_silent();
                    self.log_and_toast(
                        LogLevel::Info,
                        if self.lang() == Lang::Chinese {
                            format!("启动加载器：{}", loader_label(id))
                        } else {
                            format!("Launch loader: {}", loader_label(id))
                        },
                    );
                } else {
                    let label = loader_label(id);
                    let lang = self.lang();
                    self.modal = Some(Modal::Confirm {
                        title: if lang == Lang::Chinese {
                            format!("安装 {label}")
                        } else {
                            format!("Install {label}")
                        },
                        lines: vec![
                            if lang == Lang::Chinese {
                                format!("为当前版本运行 {label} 安装器？")
                            } else {
                                format!("Run the {label} installer for the active version?")
                            },
                            lang.pick(
                                "Forge-family installers run third-party code; use trusted builds.",
                                "Forge 系列安装器会运行第三方代码，请只使用可信构建。",
                            )
                            .into(),
                        ],
                        action: ConfirmAction::InstallLoader(id.to_string()),
                        danger: false,
                    });
                }
            }
            PickerKind::InstalledVersions => {
                self.set_active_version(id.to_string());
            }
            PickerKind::JavaRuntimes => {
                let lang = self.lang();
                if id == "__browse__" {
                    self.open_java_input();
                    return AppCommand::None;
                }
                if id == "__auto__" {
                    self.settings.java.clear();
                    self.settings_autosave_blocked = false;
                    self.save_settings_silent();
                    self.log_and_toast(
                        LogLevel::Info,
                        lang.pick("Java will be auto-detected", "Java 将自动检测"),
                    );
                    return AppCommand::None;
                }
                match resolve_java_setting(id, lang) {
                    Ok((resolved, major)) => {
                        self.settings.java = resolved.clone();
                        self.settings_autosave_blocked = false;
                        self.save_settings_silent();
                        let message = match major {
                            Some(major) if lang == Lang::Chinese => {
                                format!("已选择 Java {major}：{resolved}")
                            }
                            Some(major) => format!("Java {major} selected: {resolved}"),
                            None => format!("Java path set to {resolved}"),
                        };
                        self.log_and_toast(LogLevel::Ok, message);
                    }
                    Err(error) => self.log_and_toast(LogLevel::Error, error),
                }
            }
        }
        AppCommand::None
    }

    // ── actions ───────────────────────────────────────────────────

    pub fn cancel_job(&mut self) {
        let Some(job) = self.job.as_mut() else {
            return;
        };
        if job.is_running() {
            job.cancel_requested = true;
            progress::request_cancel();
            self.add_log(
                LogLevel::Warn,
                self.lang().pick("Cancellation requested", "已请求取消任务"),
            );
            self.toast(LogLevel::Warn, self.lang().pick("Cancelling…", "正在取消…"));
        } else {
            self.job = None;
        }
    }

    pub fn request_quit(&mut self) {
        if self.job.as_ref().is_some_and(|job| job.is_running()) {
            let lang = self.lang();
            self.modal = Some(Modal::Confirm {
                title: lang.pick("Quit while busy?", "任务进行中，退出？").into(),
                lines: vec![
                    lang.pick(
                        "A background task is still running.",
                        "仍有后台任务正在运行。",
                    )
                    .into(),
                    lang.pick(
                        "It will be cancelled when the launcher exits.",
                        "退出启动器时会取消该任务。",
                    )
                    .into(),
                ],
                action: ConfirmAction::Quit,
                danger: true,
            });
        } else {
            self.quit = true;
        }
    }

    fn open_offline_input(&mut self) {
        let lang = self.lang();
        self.modal = Some(Modal::Input {
            title: lang.pick("Add offline account", "添加离线账号").into(),
            hint: lang
                .pick(
                    "1–16 ASCII letters, digits or underscores",
                    "1–16 位 ASCII 字母、数字或下划线",
                )
                .into(),
            input: TextInput::new("").with_max_chars(16),
            action: InputAction::OfflineName,
            error: None,
        });
    }

    fn open_java_picker(&mut self) {
        let lang = self.lang();
        let current = self.settings.java.clone();
        let mut items = Vec::new();
        items.push(PickerItem {
            id: "__auto__".into(),
            label: lang.pick("Auto-detect", "自动检测").to_string(),
            meta: lang
                .pick(
                    "pick the best installed runtime",
                    "自动选择本机最合适的运行时",
                )
                .to_string(),
            tag: "auto".into(),
            installed: current.trim().is_empty(),
        });
        let required = self.current_version().and_then(|row| row.java_major);
        let infos = crate::java::detect_java(Some(&self.game_dir));
        let recommended = crate::java::best_java(&infos, required).map(|info| info.path.clone());
        for info in infos {
            let installed = !current.trim().is_empty() && info.path == current;
            let tag = if recommended.as_deref() == Some(info.path.as_str()) {
                lang.pick("recommended", "推荐").to_string()
            } else {
                info.origin.label().to_string()
            };
            items.push(PickerItem {
                id: info.path.clone(),
                label: info.label(),
                meta: info.meta(),
                tag,
                installed,
            });
        }
        items.push(PickerItem {
            id: "__browse__".into(),
            label: lang
                .pick("Enter a path manually", "手动输入路径")
                .to_string(),
            meta: lang
                .pick(
                    "java executable or JDK folder",
                    "Java 可执行文件或 JDK 目录",
                )
                .to_string(),
            tag: "input".into(),
            installed: false,
        });
        self.modal = Some(Modal::Picker(PickerState::new(
            lang.pick("Java runtime", "Java 运行时").to_string(),
            lang.pick(
                "enter select · type to filter · esc close",
                "回车选择 · 输入筛选 · esc 关闭",
            )
            .to_string(),
            PickerKind::JavaRuntimes,
            items,
        )));
    }

    fn open_jvm_input(&mut self) {
        let lang = self.lang();
        self.modal = Some(Modal::Input {
            title: lang.pick("JVM arguments", "JVM 参数").to_string(),
            hint: lang
                .pick(
                    "Extra JVM flags, e.g. -Dfoo=bar -XX:+UseStringDeduplication",
                    "附加 JVM 参数，例如 -Dfoo=bar -XX:+UseStringDeduplication",
                )
                .to_string(),
            input: TextInput::new(self.settings.jvm_args.clone()).with_max_chars(1024),
            action: InputAction::JvmArgs,
            error: None,
        });
    }

    fn open_java_input(&mut self) {
        let lang = self.lang();
        self.modal = Some(Modal::Input {
            title: lang.pick("Java executable", "Java 路径").into(),
            hint: lang
                .pick(
                    "Path to java or a JDK/JRE folder; empty auto-detects",
                    "Java 可执行文件或 JDK/JRE 目录；留空则自动检测",
                )
                .into(),
            input: TextInput::new(self.settings.java.clone()).with_max_chars(512),
            action: InputAction::JavaPath,
            error: None,
        });
    }

    fn set_selected_default(&mut self) {
        let Some(account) = self.selected_account() else {
            self.toast(
                LogLevel::Warn,
                self.lang().pick("No account selected", "未选择账号"),
            );
            return;
        };
        let key = account.key.clone();
        match AccountManager::set_default_metadata(&self.game_dir, &key) {
            Ok(()) => {
                self.refresh_accounts();
                let lang = self.lang();
                let message = if lang == Lang::Chinese {
                    format!("默认账号已切换为 {key}")
                } else {
                    format!("Default account switched to {key}")
                };
                self.log_and_toast(LogLevel::Ok, message);
            }
            Err(error) => {
                let prefix = self.lang().pick("Switch account", "切换账号");
                self.log_and_toast(LogLevel::Error, format!("{prefix}: {error}"));
            }
        }
    }

    pub fn set_active_version(&mut self, version: String) {
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                self.lang()
                    .pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return;
        }
        if !self.versions.iter().any(|row| row.id == version) {
            self.toast(
                LogLevel::Warn,
                self.lang()
                    .pick("That version is not installed", "该版本尚未安装"),
            );
            return;
        }
        self.settings.active_version = Some(version.clone());
        if let Some(index) = self.versions.iter().position(|row| row.id == version) {
            self.version_sel = index;
            self.version_state.select(Some(index));
        }
        let loaders = self
            .versions
            .iter()
            .find(|row| row.id == version)
            .map(|row| row.loaders.clone())
            .unwrap_or_default();
        if let Some(preference) = self.settings.active_loader.as_deref() {
            let label = loader_label(preference);
            if !loaders
                .iter()
                .any(|value| value.eq_ignore_ascii_case(label))
            {
                self.settings.active_loader = None;
            }
        }
        self.settings_autosave_blocked = false;
        self.save_settings_silent();
        self.refresh_mods();
        let lang = self.lang();
        let message = if lang == Lang::Chinese {
            format!("活跃版本：{version}")
        } else {
            format!("Active version: {version}")
        };
        self.log_and_toast(LogLevel::Info, message);
    }

    fn play_active(&mut self) -> AppCommand {
        if self.versions.is_empty() {
            self.open_remote_picker();
            return AppCommand::None;
        }
        if self.current_account().is_none() {
            self.screen = Screen::Accounts;
            self.toast(
                LogLevel::Warn,
                self.lang()
                    .pick("Add an account before launching", "请先添加账号再启动"),
            );
            return AppCommand::None;
        }
        let Some(version) = self.settings.active_version.clone() else {
            self.toast(
                LogLevel::Warn,
                self.lang().pick("No active version", "没有活跃版本"),
            );
            return AppCommand::None;
        };
        self.launch_version(version)
    }

    fn launch_version(&mut self, version: String) -> AppCommand {
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                self.lang()
                    .pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return AppCommand::None;
        }
        if self.current_account().is_none() {
            self.screen = Screen::Accounts;
            self.toast(
                LogLevel::Warn,
                self.lang()
                    .pick("Add an account before launching", "请先添加账号再启动"),
            );
            return AppCommand::None;
        }
        self.settings.active_version = Some(version);
        self.save_settings_silent();
        AppCommand::Suspend(SuspendTask::Launch)
    }

    pub fn begin_download(&mut self, version: String) {
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                self.lang()
                    .pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return;
        }
        let id = self.next_job_id;
        self.next_job_id += 1;
        self.pending_activate = Some(version.clone());
        self.pending_loader = None;
        self.show_job_panel = true;
        let lang = self.lang();
        self.job = Some(JobView::new(
            id,
            if lang == Lang::Chinese {
                format!("下载 Minecraft {version}")
            } else {
                format!("Download Minecraft {version}")
            },
        ));
        progress::reset_cancel();
        progress::set_job(id);
        jobs::start_download(
            self.tx.clone(),
            id,
            self.game_dir.clone(),
            version,
            self.settings.threads,
            false,
        );
    }

    pub fn begin_loader_install(&mut self, loader: String) {
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                self.lang()
                    .pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return;
        }
        let Some(version) = self.settings.active_version.clone() else {
            self.toast(
                LogLevel::Warn,
                self.lang().pick(
                    "Select an installed version first",
                    "请先选择一个已安装版本",
                ),
            );
            return;
        };
        let id = self.next_job_id;
        self.next_job_id += 1;
        self.pending_activate = None;
        self.pending_loader = Some(loader.clone());
        self.show_job_panel = true;
        let label = match loader.as_str() {
            "forge" => "Forge",
            "neoforge" => "NeoForge",
            other => other,
        };
        let lang = self.lang();
        self.job = Some(JobView::new(
            id,
            if lang == Lang::Chinese {
                format!("为 {version} 安装 {label}")
            } else {
                format!("Install {label} for {version}")
            },
        ));
        progress::reset_cancel();
        progress::set_job(id);
        if loader == "fabric" {
            jobs::start_fabric(self.tx.clone(), id, self.game_dir.clone(), version, None);
        } else {
            jobs::start_loader_install(
                self.tx.clone(),
                id,
                self.game_dir.clone(),
                self.settings.clone(),
                loader,
                version,
            );
        }
    }

    fn begin_mod_install(&mut self, slug: String, title: String) {
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                self.lang()
                    .pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return;
        }
        let Some(version) = self.settings.active_version.clone() else {
            self.toast(
                LogLevel::Warn,
                self.lang().pick(
                    "Select an installed version first",
                    "请先选择一个已安装版本",
                ),
            );
            return;
        };
        if self.active_loader().is_none() {
            self.toast(
                LogLevel::Warn,
                self.lang().pick(
                    "Install Fabric, Forge or NeoForge before adding mods",
                    "请先安装 Fabric、Forge 或 NeoForge 再添加模组",
                ),
            );
            self.open_loader_picker();
            return;
        }
        let id = self.next_job_id;
        self.next_job_id += 1;
        self.pending_activate = None;
        self.pending_loader = None;
        self.show_job_panel = true;
        let lang = self.lang();
        self.job = Some(JobView::new(
            id,
            if lang == Lang::Chinese {
                format!("安装 {title}")
            } else {
                format!("Install {title}")
            },
        ));
        progress::reset_cancel();
        progress::set_job(id);
        jobs::start_mod_install(
            self.tx.clone(),
            id,
            jobs::ModInstallJob {
                game_dir: self.game_dir.clone(),
                settings: self.settings.clone(),
                slug,
                mc_version: version,
                loader: self.active_loader_id(),
                mod_version: None,
            },
        );
    }

    fn open_rename_input(&mut self) {
        let Some(version) = self.selected_version().map(|row| row.id.clone()) else {
            self.toast(
                LogLevel::Warn,
                self.lang().pick("No version selected", "未选择版本"),
            );
            return;
        };
        let lang = self.lang();
        self.rename_source = Some(version.clone());
        self.modal = Some(Modal::Input {
            title: lang.pick("Rename instance", "重命名实例").to_string(),
            hint: lang
                .pick(
                    "New id for the version directory; no slashes",
                    "版本目录的新 ID；不能包含斜杠",
                )
                .to_string(),
            input: TextInput::new(version).with_max_chars(64),
            action: InputAction::RenameVersion,
            error: None,
        });
    }

    fn open_copy_input(&mut self) {
        let Some(version) = self.selected_version().map(|row| row.id.clone()) else {
            self.toast(
                LogLevel::Warn,
                self.lang().pick("No version selected", "未选择版本"),
            );
            return;
        };
        let lang = self.lang();
        self.copy_source = Some(version.clone());
        self.modal = Some(Modal::Input {
            title: lang.pick("Copy instance", "复制实例").to_string(),
            hint: lang
                .pick(
                    "New id for the copy (saves and mods are included)",
                    "副本的新 ID（包含存档和模组）",
                )
                .to_string(),
            input: TextInput::new(format!("{version}-copy")).with_max_chars(64),
            action: InputAction::CopyVersion,
            error: None,
        });
    }

    fn open_export_input(&mut self) {
        let Some(version) = self.selected_version().map(|row| row.id.clone()) else {
            self.toast(
                LogLevel::Warn,
                self.lang().pick("No version selected", "未选择版本"),
            );
            return;
        };
        let lang = self.lang();
        let default_path = self
            .game_dir
            .join("modpacks")
            .join(format!(
                "{}.mrpack",
                crate::instance::sanitize_instance_id(&version)
            ))
            .display()
            .to_string();
        self.export_source = Some(version);
        self.modal = Some(Modal::Input {
            title: lang.pick("Export modpack", "导出整合包").to_string(),
            hint: lang
                .pick(
                    "Destination .mrpack path (relative paths use the game dir)",
                    "导出的 .mrpack 路径（相对路径基于游戏目录）",
                )
                .to_string(),
            input: TextInput::new(default_path).with_max_chars(512),
            action: InputAction::ExportModpack,
            error: None,
        });
    }

    /// Check every installed jar against Modrinth in the background.
    pub fn begin_update_check(&mut self) {
        let lang = self.lang();
        let Some(version) = self.settings.active_version.clone() else {
            self.toast(
                LogLevel::Warn,
                lang.pick(
                    "Select an installed version first",
                    "请先选择一个已安装版本",
                ),
            );
            return;
        };
        if self.updates_loading {
            self.toast(
                LogLevel::Info,
                lang.pick("Update check already running", "更新检查正在进行"),
            );
            return;
        }
        self.updates_loading = true;
        self.updates_request = self.updates_request.wrapping_add(1);
        let request = self.updates_request;
        self.add_log(
            LogLevel::Info,
            if lang == Lang::Chinese {
                format!("正在检查 {version} 的模组更新…")
            } else {
                format!("Checking mod updates for {version}…")
            },
        );
        jobs::start_update_check(
            self.tx.clone(),
            request,
            self.game_dir.clone(),
            version,
            self.active_loader_id(),
        );
    }

    pub fn confirm_apply_updates(&mut self) {
        let lang = self.lang();
        if self.updates_loading {
            self.toast(
                LogLevel::Info,
                lang.pick("Wait for the update check", "请等待更新检查完成"),
            );
            return;
        }
        if self.mod_updates.is_empty() {
            self.toast(
                LogLevel::Info,
                lang.pick(
                    "No updates found. Press u to check.",
                    "没有可用更新。按 u 检查。",
                ),
            );
            return;
        }
        let count = self.mod_updates.len();
        self.modal = Some(Modal::Confirm {
            title: lang.pick("Apply mod updates", "应用模组更新").into(),
            lines: vec![
                if lang == Lang::Chinese {
                    format!("下载并替换 {count} 个模组？")
                } else {
                    format!("Download and replace {count} mod(s)?")
                },
                lang.pick(
                    "Disabled mods stay disabled; old files are removed.",
                    "已禁用的模组仍保持禁用；旧文件会被删除。",
                )
                .into(),
            ],
            action: ConfirmAction::ApplyUpdates,
            danger: false,
        });
    }

    fn begin_apply_updates(&mut self) {
        let lang = self.lang();
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                lang.pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return;
        }
        let Some(version) = self.settings.active_version.clone() else {
            return;
        };
        let updates = self.mod_updates.clone();
        if updates.is_empty() {
            return;
        }
        let id = self.next_job_id;
        self.next_job_id += 1;
        self.pending_activate = None;
        self.pending_loader = None;
        self.pending_update_apply = true;
        self.show_job_panel = true;
        self.job = Some(JobView::new(
            id,
            if lang == Lang::Chinese {
                format!("更新 {} 个模组", updates.len())
            } else {
                format!("Update {} mod(s)", updates.len())
            },
        ));
        progress::reset_cancel();
        progress::set_job(id);
        jobs::start_apply_updates(self.tx.clone(), id, self.game_dir.clone(), version, updates);
    }

    fn begin_export_modpack(&mut self, id: String, path: std::path::PathBuf) {
        let lang = self.lang();
        if self.job.as_ref().is_some_and(JobView::is_running) {
            self.toast(
                LogLevel::Warn,
                lang.pick("Wait for the current task to finish", "请等待当前任务完成"),
            );
            return;
        }
        let job_id = self.next_job_id;
        self.next_job_id += 1;
        self.pending_activate = None;
        self.pending_loader = None;
        self.pending_update_apply = false;
        self.show_job_panel = true;
        self.job = Some(JobView::new(
            job_id,
            if lang == Lang::Chinese {
                format!("导出整合包 {id}")
            } else {
                format!("Export modpack {id}")
            },
        ));
        progress::reset_cancel();
        progress::set_job(job_id);
        jobs::start_export_modpack(self.tx.clone(), job_id, self.game_dir.clone(), id, path);
    }

    pub fn open_remote_picker(&mut self) {
        let lang = self.lang();
        let mut picker = PickerState::new(
            lang.pick("Download Minecraft", "下载 Minecraft"),
            lang.pick(
                "Type to filter · enter select · r refresh · esc close",
                "输入以筛选 · 回车选择 · r 刷新 · esc 关闭",
            ),
            PickerKind::RemoteVersions,
            Vec::new(),
        );
        if let Some(items) = self.remote_versions.clone() {
            picker.set_items(items);
            self.modal = Some(Modal::Picker(picker));
        } else {
            picker.loading = true;
            self.modal = Some(Modal::Picker(picker));
            self.request_manifest(false);
        }
    }

    pub fn open_loader_picker(&mut self) {
        let Some(version) = self.settings.active_version.clone() else {
            self.toast(
                LogLevel::Warn,
                self.lang().pick(
                    "Download a Minecraft version before choosing a loader",
                    "请先下载 Minecraft 版本再选择加载器",
                ),
            );
            return;
        };
        let row = self.versions.iter().find(|row| row.id == version);
        let loaders = row.map(|row| row.loaders.clone()).unwrap_or_default();
        let lang = self.lang();
        let mut items = vec![PickerItem {
            id: "auto".into(),
            label: lang
                .pick("Auto · first detected", "自动 · 首个已安装")
                .into(),
            meta: lang
                .pick(
                    "Use the first installed loader, or vanilla if none",
                    "使用第一个已安装加载器；没有则使用原版",
                )
                .into(),
            tag: if self.settings.active_loader.is_none() {
                "active".into()
            } else {
                String::new()
            },
            installed: true,
        }];
        items.extend(
            [
                (
                    "fabric",
                    "Fabric",
                    lang.pick(
                        "Lightweight, fast updates, great mod compatibility",
                        "轻量、更新快、模组兼容性好",
                    ),
                ),
                (
                    "forge",
                    "Forge",
                    lang.pick(
                        "Classic modding platform for large packs",
                        "经典大型整合包模组平台",
                    ),
                ),
                (
                    "neoforge",
                    "NeoForge",
                    lang.pick(
                        "Modern fork of Forge for 1.20.2+",
                        "面向 1.20.2+ 的现代 Forge 分支",
                    ),
                ),
            ]
            .into_iter()
            .map(|(id, label, meta)| {
                let installed = loaders
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(label));
                PickerItem {
                    id: id.into(),
                    label: format!("{label}  ·  Minecraft {version}"),
                    meta: meta.into(),
                    tag: if self.settings.active_loader.as_deref() == Some(id) {
                        "active".into()
                    } else if installed {
                        "installed".into()
                    } else {
                        "new".into()
                    },
                    installed,
                }
            }),
        );
        self.modal = Some(Modal::Picker(PickerState::new(
            lang.pick("Launch loader", "启动加载器"),
            lang.pick(
                "enter select installed · uninstalled entries install · esc close",
                "回车选择已安装项 · 未安装项会启动安装 · esc 关闭",
            ),
            PickerKind::Loaders,
            items,
        )));
    }

    fn open_installed_version_picker(&mut self) {
        if self.versions.is_empty() {
            self.toast(
                LogLevel::Warn,
                self.lang()
                    .pick("No versions installed", "尚未安装任何版本"),
            );
            return;
        }
        let active = self.settings.active_version.clone();
        let items = self
            .versions
            .iter()
            .map(|row| PickerItem {
                id: row.id.clone(),
                label: row.id.clone(),
                meta: if self.lang() == Lang::Chinese {
                    format!(
                        "{} · {} 个模组{}",
                        version_kind_label(self.lang(), &row.kind),
                        row.mods_enabled,
                        if row.loaders.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", row.loaders.join("/"))
                        }
                    )
                } else {
                    format!(
                        "{} · {} mods{}",
                        row.kind,
                        row.mods_enabled,
                        if row.loaders.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", row.loaders.join("/"))
                        }
                    )
                },
                tag: if Some(&row.id) == active.as_ref() {
                    "active".into()
                } else {
                    String::new()
                },
                installed: true,
            })
            .collect();
        let lang = self.lang();
        self.modal = Some(Modal::Picker(PickerState::new(
            lang.pick("Active Minecraft version", "活跃 Minecraft 版本"),
            lang.pick(
                "Used by the mods and launch screens · enter select · esc close",
                "用于模组和启动界面 · 回车选择 · esc 关闭",
            ),
            PickerKind::InstalledVersions,
            items,
        )));
    }

    pub fn open_search(&mut self) {
        self.open_discover(DiscoverKind::Mods);
    }

    /// Open the discover overlay in browse mode for mods, packs or shaders.
    pub fn open_discover(&mut self, kind: DiscoverKind) {
        if self.settings.active_version.is_none() {
            self.toast(
                LogLevel::Warn,
                self.lang().pick(
                    "Select an installed version first",
                    "请先选择一个已安装版本",
                ),
            );
            return;
        }
        self.search = SearchState::default();
        self.search.open = true;
        self.search.kind = kind;
        self.search.browse = true;
        self.search.sort = "downloads".into();
        self.search.focus = SearchFocus::Input;
        self.request_discover();
    }

    fn request_discover(&mut self) {
        let Some(version) = self.settings.active_version.clone() else {
            self.toast(
                LogLevel::Warn,
                self.lang().pick(
                    "Select an installed version first",
                    "请先选择一个已安装版本",
                ),
            );
            return;
        };
        self.search.request = self.next_request_id;
        self.next_request_id += 1;
        self.search.loading = true;
        self.search.error = None;
        self.search.focus = SearchFocus::Input;
        let query = if self.search.browse {
            String::new()
        } else {
            self.search.input.value().trim().to_string()
        };
        jobs::start_search(
            self.tx.clone(),
            self.search.request,
            self.search.kind,
            query,
            Some(version),
            self.active_loader_id(),
            self.search.page,
            self.search.sort.clone(),
            self.search.category.clone(),
            self.game_dir.clone(),
        );
    }

    fn submit_search(&mut self) {
        let query = self.search.input.value().trim().to_string();
        self.search.browse = query.is_empty();
        self.search.page = 0;
        self.search.sort = if self.search.browse {
            "downloads".into()
        } else {
            "relevance".into()
        };
        self.request_discover();
    }

    fn browse_search(&mut self) {
        self.search.input.clear();
        self.search.browse = true;
        self.search.page = 0;
        self.search.sort = "downloads".into();
        self.request_discover();
    }

    fn page_search(&mut self, delta: i32) {
        let next = (self.search.page as i32 + delta).max(0) as u32;
        if next == self.search.page {
            return;
        }
        self.search.page = next;
        self.request_discover();
    }

    fn cycle_search_category(&mut self) {
        if self.search.kind != DiscoverKind::Mods {
            return;
        }
        self.search.category = if self.search.category.is_some() {
            None
        } else {
            Some("optimization".into())
        };
        self.search.page = 0;
        self.request_discover();
    }

    fn confirm_search_install(&mut self) {
        let Some(hit) = self.search.results.get(self.search.selected).cloned() else {
            self.toast(
                LogLevel::Warn,
                self.lang()
                    .pick("No search result selected", "未选择搜索结果"),
            );
            return;
        };
        let version = self
            .settings
            .active_version
            .clone()
            .unwrap_or_else(|| "?".into());
        let kind = self.search.kind;
        self.search.open = false;
        let lang = self.lang();
        if kind == DiscoverKind::Mods && hit.installed {
            let suffix = hit
                .installed_version
                .as_deref()
                .map(|version| format!(" ({version})"))
                .unwrap_or_default();
            self.log_and_toast(
                LogLevel::Info,
                if lang == Lang::Chinese {
                    format!(
                        "{} 已安装{}；用 update-mods --apply 更新。",
                        hit.title, suffix
                    )
                } else {
                    format!(
                        "{} is already installed{}; use update-mods --apply to update.",
                        hit.title, suffix
                    )
                },
            );
            return;
        }
        match kind {
            DiscoverKind::Mods => {
                let loader_line = self
                    .active_loader()
                    .map(|loader| {
                        if lang == Lang::Chinese {
                            format!("加载器：{loader}")
                        } else {
                            format!("Loader: {loader}")
                        }
                    })
                    .unwrap_or_else(|| {
                        lang.pick(
                            "No mod loader detected; vanilla may ignore mods.",
                            "未检测到模组加载器；原版可能忽略模组。",
                        )
                        .into()
                    });
                self.modal = Some(Modal::Confirm {
                    title: if lang == Lang::Chinese {
                        format!("安装 {}", hit.title)
                    } else {
                        format!("Install {}", hit.title)
                    },
                    lines: vec![
                        if lang == Lang::Chinese {
                            format!("为 Minecraft {version} 安装 {}？", hit.title)
                        } else {
                            format!("Install {} for Minecraft {version}?", hit.title)
                        },
                        loader_line,
                        lang.pick(
                            "Required dependencies are installed automatically.",
                            "必需的依赖会自动安装。",
                        )
                        .into(),
                    ],
                    action: ConfirmAction::InstallMod {
                        slug: hit.slug,
                        title: hit.title,
                    },
                    danger: false,
                });
            }
            DiscoverKind::ResourcePacks => {
                self.modal = Some(Modal::Confirm {
                    title: if lang == Lang::Chinese {
                        format!("安装资源包 {}", hit.title)
                    } else {
                        format!("Install resource pack {}", hit.title)
                    },
                    lines: vec![
                        if lang == Lang::Chinese {
                            format!("为 Minecraft {version} 下载 {}？", hit.title)
                        } else {
                            format!("Download {} for Minecraft {version}?", hit.title)
                        },
                        lang.pick(
                            "It is saved to resourcepacks/ and enabled for the next launch.",
                            "将保存到 resourcepacks/ 并在下次启动时启用。",
                        )
                        .into(),
                    ],
                    action: ConfirmAction::InstallResourcePack {
                        slug: hit.slug,
                        title: hit.title,
                    },
                    danger: false,
                });
            }
            DiscoverKind::Shaders => {
                let support = ShaderSupport::from_labels(hit.loaders.iter().map(String::as_str));
                let status = crate::content::ContentManager::new(&self.game_dir)
                    .shader_loader_status(&version);
                let mut lines = vec![
                    if lang == Lang::Chinese {
                        format!("为 Minecraft {version} 下载 {}？", hit.title)
                    } else {
                        format!("Download {} for Minecraft {version}?", hit.title)
                    },
                    if lang == Lang::Chinese {
                        format!("光影加载器支持：{}", support.summary(true))
                    } else {
                        format!("Shader loaders: {}", support.summary(false))
                    },
                    if lang == Lang::Chinese {
                        format!(
                            "Iris：{} · OptiFine：{}",
                            if status.iris {
                                "已安装"
                            } else {
                                "未安装"
                            },
                            if status.optifine {
                                "已安装"
                            } else {
                                "未安装"
                            }
                        )
                    } else {
                        format!(
                            "Iris: {} · OptiFine: {}",
                            if status.iris { "installed" } else { "missing" },
                            if status.optifine {
                                "installed"
                            } else {
                                "missing"
                            }
                        )
                    },
                    lang.pick(
                        "It is saved to shaderpacks/ and enabled for the next launch.",
                        "将保存到 shaderpacks/ 并在下次启动时启用。",
                    )
                    .into(),
                ];
                if support.iris && !status.iris {
                    lines.push(
                        lang.pick(
                            "You will be offered to install Iris after the download.",
                            "下载后会提示安装 Iris。",
                        )
                        .into(),
                    );
                }
                if !support.iris && support.optifine && !status.optifine {
                    lines.push(
                        lang.pick(
                            "OptiFine is required; install it manually from optifine.net.",
                            "需要 OptiFine；请从 optifine.net 手动安装。",
                        )
                        .into(),
                    );
                }
                self.modal = Some(Modal::Confirm {
                    title: if lang == Lang::Chinese {
                        format!("安装光影包 {}", hit.title)
                    } else {
                        format!("Install shader pack {}", hit.title)
                    },
                    lines,
                    action: ConfirmAction::InstallShader {
                        slug: hit.slug,
                        title: hit.title,
                        support,
                    },
                    danger: false,
                });
            }
        }
    }

    fn confirm_optimize_mods(&mut self) {
        let lang = self.lang();
        let Some(version) = self.settings.active_version.clone() else {
            self.toast(
                LogLevel::Warn,
                lang.pick(
                    "Select an installed version first",
                    "请先选择一个已安装版本",
                ),
            );
            return;
        };
        let loader = self.active_loader_id().or_else(|| {
            InstanceManager::new(&self.game_dir)
                .loaders(&version)
                .first()
                .cloned()
        });
        let Some(loader) = loader else {
            self.toast(
                LogLevel::Warn,
                lang.pick(
                    "Install Fabric, NeoForge or Forge before adding optimization mods",
                    "请先安装 Fabric、NeoForge 或 Forge 再添加优化模组",
                ),
            );
            return;
        };
        self.modal = Some(Modal::Confirm {
            title: lang
                .pick("Install optimization mods", "安装优化模组")
                .into(),
            lines: vec![
                if lang == Lang::Chinese {
                    format!("为 Minecraft {version}（{loader}）安装推荐的优化模组？")
                } else {
                    format!(
                        "Install the recommended optimization mods for {version} ({loader})?"
                    )
                },
                lang.pick(
                    "Already installed mods are skipped; required dependencies are installed automatically.",
                    "已安装的模组会跳过；必需依赖会自动安装。",
                )
                .into(),
            ],
            action: ConfirmAction::InstallOptimizations,
            danger: false,
        });
    }

    fn toggle_selected_mod(&mut self) {
        let Some(version) = self.settings.active_version.clone() else {
            return;
        };
        let Some(mod_row) = self.selected_mod().cloned() else {
            return;
        };
        let manager = ModManager::new(&self.game_dir);
        let changed = if mod_row.enabled {
            manager.disable_mod(&mod_row.file, &version)
        } else {
            manager.enable_mod(&mod_row.file, &version)
        };
        if changed {
            self.refresh_mods();
            let lang = self.lang();
            let message = if lang == Lang::Chinese {
                format!(
                    "{} 已{}",
                    mod_row.display,
                    if mod_row.enabled { "禁用" } else { "启用" }
                )
            } else {
                format!(
                    "{} {}",
                    mod_row.display,
                    if mod_row.enabled {
                        "disabled"
                    } else {
                        "enabled"
                    }
                )
            };
            self.log_and_toast(LogLevel::Ok, message);
        } else {
            self.toast(
                LogLevel::Warn,
                self.lang().pick(
                    "Could not toggle that mod (another file may conflict)",
                    "无法切换该模组（可能与其他文件冲突）",
                ),
            );
        }
    }

    // ── backend events ────────────────────────────────────────────

    pub fn handle_backend(&mut self, event: BackendEvent) {
        self.note_activity();
        match event {
            BackendEvent::JobStarted { id, title } => {
                if let Some(job) = self.job.as_mut() {
                    if job.id == id {
                        // The frontend already created a localized title.
                        let display = job.title.clone();
                        job.add_log(LogLevel::Info, display);
                        let _ = title;
                    }
                }
            }
            BackendEvent::Phase {
                id,
                index,
                total,
                label,
            } => {
                let is_current = self.job.as_ref().is_some_and(|job| job.id == id);
                if is_current {
                    if let Some(job) = self.job.as_mut() {
                        job.on_phase(index, total, label.clone());
                    }
                    self.add_log(
                        LogLevel::Info,
                        format!(
                            "[{index}/{total}] {}",
                            i18n::localize_job_phase(self.lang(), &label)
                        ),
                    );
                }
            }
            BackendEvent::Log { id, level, text } => {
                let text = i18n::localize_job_log(self.lang(), &text);
                if self.job.as_ref().is_some_and(|job| job.id == id) {
                    if let Some(job) = self.job.as_mut() {
                        job.add_log(level, text.clone());
                    }
                    self.add_log(level, text);
                } else {
                    self.add_log(level, text);
                }
            }
            BackendEvent::Transfer { id, transfer } => {
                if let Some(job) = self.job.as_mut() {
                    if job.id == id {
                        job.on_transfer(transfer);
                    }
                }
            }
            BackendEvent::Finished { id, result } => self.finish_job(id, result),
            BackendEvent::Search {
                request,
                query,
                result,
            } => {
                if request != self.search.request {
                    return;
                }
                self.search.loading = false;
                match result {
                    Ok(value) => {
                        self.search.total_hits = value["total_hits"].as_u64().unwrap_or(0);
                        self.search.results = parse_search_hits(&value);
                        self.search.selected = 0;
                        self.search.list_state = ListState::default();
                        if !self.search.results.is_empty() {
                            self.search.list_state.select(Some(0));
                            self.search.focus = SearchFocus::Results;
                        }
                        let kind = self.search.kind;
                        let message = if self.lang() == Lang::Chinese {
                            if query.is_empty() {
                                format!(
                                    "Modrinth {}：第 {} 页，{} 个结果",
                                    kind.title(true),
                                    self.search.page + 1,
                                    self.search.results.len()
                                )
                            } else {
                                format!(
                                    "Modrinth {}：{} 个结果，关键词 \"{}\"",
                                    kind.title(true),
                                    self.search.results.len(),
                                    query
                                )
                            }
                        } else if query.is_empty() {
                            format!(
                                "Modrinth {}: page {}, {} result(s)",
                                kind.title(false),
                                self.search.page + 1,
                                self.search.results.len()
                            )
                        } else {
                            format!(
                                "Modrinth {}: {} result(s) for \"{}\"",
                                kind.title(false),
                                self.search.results.len(),
                                query
                            )
                        };
                        self.add_log(LogLevel::Info, message);
                    }
                    Err(error) => {
                        let error = theme::one_line(&error, 400);
                        self.search.error = Some(error.clone());
                        let prefix = self.lang().pick("Search failed", "搜索失败");
                        self.add_log(LogLevel::Error, format!("{prefix}: {error}"));
                        self.toast(LogLevel::Error, error);
                    }
                }
            }
            BackendEvent::Manifest { request, result } => {
                if request != self.manifest_request {
                    return;
                }
                match result {
                    Ok(value) => {
                        let items = parse_remote_versions(&value, &self.versions);
                        self.remote_versions = Some(items.clone());
                        if let Some(Modal::Picker(picker)) = self.modal.as_mut() {
                            if picker.kind == PickerKind::RemoteVersions {
                                picker.set_items(items);
                                picker.loading = false;
                                picker.error = None;
                            }
                        }
                        let count = self.remote_versions.as_ref().map(Vec::len).unwrap_or(0);
                        self.add_log(
                            LogLevel::Info,
                            if self.lang() == Lang::Chinese {
                                format!("版本清单已加载（{count} 个版本）")
                            } else {
                                format!("Version manifest loaded ({count} versions)")
                            },
                        );
                    }
                    Err(error) => {
                        let error = theme::one_line(&error, 400);
                        if let Some(Modal::Picker(picker)) = self.modal.as_mut() {
                            if picker.kind == PickerKind::RemoteVersions {
                                picker.loading = false;
                                picker.error = Some(error.clone());
                            }
                        }
                        let prefix = self
                            .lang()
                            .pick("Version manifest failed", "版本清单加载失败");
                        self.add_log(LogLevel::Error, format!("{prefix}: {error}"));
                        self.toast(LogLevel::Error, error);
                    }
                }
            }
            BackendEvent::ModUpdates { request, result } => {
                if request != self.updates_request {
                    return;
                }
                self.updates_loading = false;
                let lang = self.lang();
                match result {
                    Ok(updates) => {
                        let count = updates.len();
                        self.mod_updates = updates;
                        self.refresh_mods();
                        if count == 0 {
                            self.log_and_toast(
                                LogLevel::Ok,
                                lang.pick(
                                    "All installed mods are up to date",
                                    "所有已安装模组均为最新",
                                ),
                            );
                        } else {
                            let lines: Vec<String> = self
                                .mod_updates
                                .iter()
                                .map(|update| {
                                    format!(
                                        "{} {} -> {}",
                                        update.project, update.current, update.latest
                                    )
                                })
                                .collect();
                            for line in lines {
                                self.add_log(LogLevel::Info, line);
                            }
                            let message = if lang == Lang::Chinese {
                                format!("{count} 个模组可更新 · 按 U 应用")
                            } else {
                                format!("{count} update(s) available · press U to apply")
                            };
                            self.log_and_toast(LogLevel::Ok, message);
                        }
                    }
                    Err(error) => {
                        let error = theme::one_line(&error, 400);
                        let prefix = lang.pick("Update check failed", "更新检查失败");
                        self.add_log(LogLevel::Error, format!("{prefix}: {error}"));
                        self.toast(LogLevel::Error, error);
                    }
                }
            }
            BackendEvent::Java { path, major } => {
                self.java_probing = false;
                self.java_path = path;
                self.java_major = major;
                if let Some(path) = self.java_path.as_ref() {
                    let major = major.map(|m| m.to_string()).unwrap_or_else(|| "?".into());
                    self.add_log(
                        LogLevel::Debug,
                        if self.lang() == Lang::Chinese {
                            format!("Java {major}：{path}")
                        } else {
                            format!("Java {major} at {path}")
                        },
                    );
                } else {
                    self.add_log(
                        LogLevel::Warn,
                        self.lang()
                            .pick("No Java installation detected", "未检测到 Java 安装"),
                    );
                }
            }
        }
    }

    fn finish_job(&mut self, id: u64, result: Result<String, String>) {
        if self.job.as_ref().is_none_or(|job| job.id != id) {
            return;
        }
        progress::set_job(0);
        let cancelled =
            matches!(&result, Err(error) if error.to_ascii_lowercase().contains("cancel"));
        match result {
            Ok(message) => {
                let message = i18n::localize_job_result(self.lang(), &message);
                if let Some(job) = self.job.as_mut() {
                    job.finished = Some(Ok(message.clone()));
                    job.finished_at = Some(Instant::now());
                }
                self.log_and_toast(LogLevel::Ok, message);
                self.spawn_particles(theme::GREEN, 34);
                let activated = self.pending_activate.take();
                if let Some(version) = activated.as_ref() {
                    self.settings.active_version = Some(version.clone());
                    self.save_settings_silent();
                }
                if let Some(loader) = self.pending_loader.take() {
                    self.settings.active_loader = Some(loader);
                    self.save_settings_silent();
                }
                if self.pending_update_apply {
                    self.pending_update_apply = false;
                    self.mod_updates.clear();
                    self.refresh_mods();
                }
                self.refresh_local();
                if let Some((title, support)) = self.pending_shader_loader.take() {
                    self.prompt_shader_loader(title, support);
                }
                if let Some(version) = activated {
                    if let Some(index) = self.versions.iter().position(|row| row.id == version) {
                        self.version_sel = index;
                        self.version_state.select(Some(index));
                    }
                }
            }
            Err(error) => {
                if cancelled {
                    self.job = None;
                    self.pending_activate = None;
                    self.pending_loader = None;
                    self.pending_update_apply = false;
                    self.pending_shader_loader = None;
                    self.log_and_toast(
                        LogLevel::Warn,
                        self.lang().pick("Task cancelled", "任务已取消"),
                    );
                    return;
                }
                let error = theme::one_line(&error, 400);
                self.pending_activate = None;
                self.pending_loader = None;
                self.pending_update_apply = false;
                self.pending_shader_loader = None;
                if let Some(job) = self.job.as_mut() {
                    job.finished = Some(Err(error.clone()));
                    job.finished_at = Some(Instant::now());
                }
                self.log_and_toast(LogLevel::Error, error);
                self.refresh_local();
            }
        }
    }

    // ── suspended terminal tasks ─────────────────────────────────

    pub fn perform_suspended(&mut self, task: SuspendTask) -> Result<String, String> {
        match task {
            SuspendTask::Launch => self.perform_launch(),
            SuspendTask::Login(mode) => self.perform_login(mode),
        }
    }

    fn perform_launch(&mut self) -> Result<String, String> {
        let lang = self.lang();
        let version = self.settings.active_version.clone().ok_or_else(|| {
            lang.pick("No active version selected", "未选择活跃版本")
                .to_string()
        })?;
        // An explicit loader choice is validated by refresh_local; auto mode
        // lets the launcher core pick the first valid detected profile.
        let loader = self.settings.active_loader.clone();
        let (width, height) = self.settings.window();
        let ram = self.settings.ram_mb;

        if lang == Lang::Chinese {
            println!("\n  Hot-RS-Launcher  ·  正在启动 Minecraft {version}");
            println!("  游戏退出后终端会回到这里。\n");
        } else {
            println!("\n  Hot-RS-Launcher  ·  starting Minecraft {version}");
            println!("  Terminal control returns here when the game exits.\n");
        }
        if !self.settings.java.trim().is_empty() {
            self.launcher
                .set_java(self.settings.java.trim())
                .map_err(|e| {
                    if lang == Lang::Chinese {
                        format!("Java 路径无效：{e}")
                    } else {
                        format!("Invalid Java path: {e}")
                    }
                })?;
        }
        // Account metadata may have been edited natively by the TUI. Reload
        // from disk so logout/login/default changes are not lost or bypassed.
        let accounts = AccountManager::new(&self.game_dir).map_err(|e| {
            if lang == Lang::Chinese {
                format!("无法加载账号：{e}")
            } else {
                format!("Cannot load accounts: {e}")
            }
        })?;
        self.launcher.accounts = accounts;
        let account = self.launcher.accounts.get_default().ok_or_else(|| {
            lang.pick(
                "No account with usable credentials. Log in again.",
                "没有可用账号，请重新登录。",
            )
            .to_string()
        })?;
        let _ = std::io::stdout().flush();
        let options = crate::launcher::LaunchOptions {
            version: Some(version.clone()),
            ram_mb: ram,
            loader,
            width,
            height,
            extra_jvm: self.settings.jvm_arguments(),
            gc: self.settings.gc_preset(),
            fullscreen: false,
            server: None,
            dry_run: false,
            // The TUI prints a localized diagnosis itself.
            diagnose: false,
        };
        let result = self.launcher.launch_with(options, Some(account));
        match result {
            Ok(0) => {
                let message = if lang == Lang::Chinese {
                    format!("Minecraft {version} 已退出")
                } else {
                    format!("Minecraft {version} exited")
                };
                println!("\n  {message}");
                wait_for_enter(lang);
                Ok(message)
            }
            Ok(code) => {
                let message = if lang == Lang::Chinese {
                    format!("Minecraft {version} 已退出，退出码 {code}")
                } else {
                    format!("Minecraft {version} exited with code {code}")
                };
                println!("\n  {message}");
                if let Some(diagnosis) = crate::diagnose::analyze_instance(&self.game_dir, &version)
                {
                    println!("\n  {}", diagnosis.source);
                    for finding in &diagnosis.findings {
                        println!(
                            "  [{}] {}",
                            finding.severity.label(),
                            finding.title_localized(lang == Lang::Chinese)
                        );
                        if !finding.hint.is_empty() {
                            println!(
                                "        → {}",
                                finding.hint_localized(lang == Lang::Chinese)
                            );
                        }
                    }
                    self.pending_diagnosis = Some(diagnosis);
                }
                wait_for_enter(lang);
                Err(message)
            }
            Err(error) => {
                let message = if lang == Lang::Chinese {
                    format!("启动失败：{error}")
                } else {
                    format!("Launch failed: {error}")
                };
                println!("\n  {message}");
                wait_for_enter(lang);
                Err(message)
            }
        }
    }

    fn perform_login(&mut self, mode: LoginMode) -> Result<String, String> {
        let lang = self.lang();
        if lang == Lang::Chinese {
            println!("\n  Hot-RS-Launcher  ·  Microsoft 登录");
            println!("  按下方提示操作，完成后会回到 TUI。\n");
        } else {
            println!("\n  Hot-RS-Launcher  ·  Microsoft login");
            println!("  Follow the instructions below; the TUI returns afterwards.\n");
        }
        let mut auth = MicrosoftAuth::new();
        let login_error = |e: String| {
            if lang == Lang::Chinese {
                format!("Microsoft 登录失败：{e}")
            } else {
                format!("Microsoft login failed: {e}")
            }
        };
        match mode {
            LoginMode::Browser => auth.login().map_err(|e| login_error(e.to_string()))?,
            LoginMode::DeviceCode => auth
                .device_code_login()
                .map_err(|e| login_error(e.to_string()))?,
        };
        let uid = crate::util::format_uuid(&auth.uuid);
        let mut accounts = AccountManager::new(&self.game_dir).map_err(|e| {
            if lang == Lang::Chinese {
                format!("无法加载账号：{e}")
            } else {
                format!("Cannot load accounts: {e}")
            }
        })?;
        accounts
            .set_msa(
                &auth.username,
                &uid,
                &auth.mc_token,
                &auth.refresh_token,
                auth.expires_at,
            )
            .map_err(|e| {
                if lang == Lang::Chinese {
                    format!("保存凭据失败：{e}")
                } else {
                    format!("Saving credentials failed: {e}")
                }
            })?;
        self.launcher.accounts = accounts;
        if lang == Lang::Chinese {
            println!("\n  [OK] 已登录为 {}", auth.username);
            wait_for_enter(lang);
            Ok(format!("已登录为 {}", auth.username))
        } else {
            println!("\n  [OK] Logged in as {}", auth.username);
            wait_for_enter(lang);
            Ok(format!("Signed in as {}", auth.username))
        }
    }

    pub fn after_suspended(&mut self, task: SuspendTask, result: Result<String, String>) {
        match result {
            Ok(message) => {
                self.log_and_toast(LogLevel::Ok, message);
                if task == SuspendTask::Launch {
                    self.spawn_particles(theme::GOLD, 26);
                }
            }
            Err(error) => {
                self.log_and_toast(LogLevel::Error, error);
                if task == SuspendTask::Launch {
                    if let Some(diagnosis) = self.pending_diagnosis.take() {
                        for finding in diagnosis.findings.iter().take(4) {
                            let text = format!(
                                "{} — {}",
                                finding.title_localized(self.lang() == Lang::Chinese),
                                finding.hint_localized(self.lang() == Lang::Chinese)
                            );
                            self.add_log(
                                match finding.severity {
                                    crate::diagnose::Severity::Error => LogLevel::Error,
                                    crate::diagnose::Severity::Warn => LogLevel::Warn,
                                    crate::diagnose::Severity::Info => LogLevel::Info,
                                },
                                text,
                            );
                        }
                        self.toast(
                            LogLevel::Warn,
                            self.lang()
                                .pick("Crash analysis added to Activity", "崩溃诊断已写入活动日志"),
                        );
                    }
                }
            }
        }
        self.refresh_local();
    }

    pub fn account_expiry_label(&self, account: &AccountRow) -> String {
        let Some(expires) = account.expires_at else {
            return String::new();
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let remaining = expires - now;
        let lang = self.lang();
        if remaining <= 0.0 {
            return lang.pick("session expired", "会话已过期").into();
        }
        let hours = remaining / 3600.0;
        if lang == Lang::Chinese {
            if hours >= 48.0 {
                format!("会话剩余 ~{:.0} 天", hours / 24.0)
            } else if hours >= 1.0 {
                format!("会话剩余 ~{hours:.0} 小时")
            } else {
                format!("会话剩余 ~{:.0} 分钟", remaining / 60.0)
            }
        } else if hours >= 48.0 {
            format!("session ~{:.0}d", hours / 24.0)
        } else if hours >= 1.0 {
            format!("session ~{hours:.0}h")
        } else {
            format!("session ~{:.0}m", remaining / 60.0)
        }
    }
}

#[derive(Debug)]
enum ModalRequest {
    Confirm(ConfirmAction),
    Input(InputAction, String),
    Pick(PickerKind, String),
    ReloadManifest,
    Login(LoginMode),
}

fn version_kind_label(lang: Lang, kind: &str) -> String {
    match kind {
        "release" => lang.pick("release", "正式版").to_string(),
        "snapshot" => lang.pick("snapshot", "快照").to_string(),
        other => other.to_string(),
    }
}

fn loader_label(id: &str) -> &'static str {
    match id {
        "fabric" => "Fabric",
        "forge" => "Forge",
        "neoforge" => "NeoForge",
        _ => "Auto",
    }
}

fn loader_id(label: &str) -> &'static str {
    if label.eq_ignore_ascii_case("Fabric") {
        "fabric"
    } else if label.eq_ignore_ascii_case("Forge") {
        "forge"
    } else if label.eq_ignore_ascii_case("NeoForge") {
        "neoforge"
    } else {
        "auto"
    }
}

fn text_modifier(modifiers: KeyModifiers) -> bool {
    !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

/// Accept either a `java` executable or a JDK/JRE home directory. The result is
/// probed with `java -version` so a typo is rejected while the settings dialog
/// is open instead of when the game fails to start.
fn resolve_java_setting(value: &str, lang: Lang) -> Result<(String, Option<u32>), String> {
    if value.is_empty() {
        return Ok((String::new(), None));
    }
    let path = std::path::PathBuf::from(value);
    let executable = if path.is_dir() {
        let exe = if cfg!(windows) { "java.exe" } else { "java" };
        let candidate = path.join("bin").join(exe);
        if !candidate.is_file() {
            return Err(lang
                .pick(
                    "That directory has no bin/java. Pick the executable or a JDK/JRE folder.",
                    "该目录下没有 bin/java。请选择 java 可执行文件或 JDK/JRE 目录。",
                )
                .to_string());
        }
        candidate
    } else if path.is_file() {
        path
    } else {
        return Err(lang
            .pick("No such file or directory", "文件或目录不存在")
            .to_string());
    };
    let text = executable.to_string_lossy().to_string();
    match crate::java::java_major(&text) {
        Some(major) => Ok((text, Some(major))),
        None => Err(lang
            .pick(
                "That file did not answer 'java -version'; pick a working Java executable.",
                "该文件无法运行 java -version；请选择可用的 Java 可执行文件。",
            )
            .to_string()),
    }
}

fn cycle_preset(presets: &[u32], current: u32, direction: i32) -> u32 {
    let index = presets
        .iter()
        .position(|preset| *preset == current)
        .unwrap_or(0);
    let next = (index as i32 + direction).rem_euclid(presets.len() as i32) as usize;
    presets[next]
}

/// Open a link in the user's browser without blocking the UI thread.
fn open_url(url: &str) {
    let url = url.to_string();
    let _ = std::thread::Builder::new()
        .name("mc-tui-browser".into())
        .spawn(move || {
            let _ = webbrowser::open(&url);
        });
}

/// Open a local directory with the system file manager.
fn open_path(path: &std::path::Path) {
    if let Ok(url) = url::Url::from_file_path(path) {
        open_url(url.as_str());
    }
}

fn wait_for_enter(lang: Lang) {
    if lang == Lang::Chinese {
        print!("\n  按回车返回启动器... ");
    } else {
        print!("\n  Press Enter to return to the launcher... ");
    }
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    println!();
}

pub fn parse_search_hits(value: &serde_json::Value) -> Vec<SearchHit> {
    value["hits"]
        .as_array()
        .map(|hits| {
            hits.iter()
                .filter_map(|hit| {
                    let slug = hit["slug"].as_str()?.to_string();
                    let title = hit["title"].as_str().unwrap_or(slug.as_str()).to_string();
                    let labels: Vec<&str> = hit["categories"]
                        .as_array()
                        .map(|values| values.iter().filter_map(|value| value.as_str()).collect())
                        .unwrap_or_default();
                    let loaders = labels
                        .iter()
                        .filter(|label| {
                            matches!(
                                label.to_ascii_lowercase().as_str(),
                                "iris"
                                    | "optifine"
                                    | "fabric"
                                    | "forge"
                                    | "neoforge"
                                    | "quilt"
                                    | "canvas"
                                    | "vanilla"
                            )
                        })
                        .map(|label| label.to_string())
                        .collect();
                    Some(SearchHit {
                        title,
                        slug,
                        description: hit["description"].as_str().unwrap_or("").to_string(),
                        author: hit["author"].as_str().unwrap_or("unknown").to_string(),
                        downloads: hit["downloads"].as_u64().unwrap_or(0),
                        categories: labels.join(", "),
                        loaders,
                        installed: hit["_installed"].as_bool().unwrap_or(false),
                        installed_version: hit["_installed_version"].as_str().map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_remote_versions(
    value: &serde_json::Value,
    installed: &[VersionRow],
) -> Vec<PickerItem> {
    let installed: HashSet<&str> = installed.iter().map(|row| row.id.as_str()).collect();
    let mut versions: Vec<RemoteVersion> = value["versions"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let id = row["id"].as_str()?.to_string();
                    let kind = row["type"].as_str().unwrap_or("release").to_string();
                    let released = row["releaseTime"]
                        .as_str()
                        .or_else(|| row["time"].as_str())
                        .unwrap_or("")
                        .chars()
                        .take(10)
                        .collect();
                    Some(RemoteVersion { id, kind, released })
                })
                .collect()
        })
        .unwrap_or_default();
    versions.sort_by(|a, b| b.released.cmp(&a.released));
    versions
        .into_iter()
        .map(|version| PickerItem {
            id: version.id.clone(),
            label: version.id.clone(),
            meta: format!("released {}", version.released),
            tag: version.kind,
            installed: installed.contains(version.id.as_str()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn launcher(dir: &Path) -> MinecraftLauncher {
        MinecraftLauncher::new(dir, 2).unwrap()
    }

    #[test]
    fn app_starts_with_empty_workspace_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = jobs::channel();
        let mut app = App::new(
            launcher(dir.path()),
            tx,
            Bootstrap {
                no_splash: true,
                ..Bootstrap::default()
            },
        );
        assert!(app.versions.is_empty());
        assert!(app.mods.is_empty());
        assert!(app.accounts.is_empty());
        assert_eq!(app.screen, Screen::Dashboard);
        app.tick();
        app.skip_splash();
        assert!(!app.splash_active());
    }

    #[cfg(unix)]
    #[test]
    fn invalid_version_directory_names_are_skipped_without_exiting() {
        let dir = tempfile::tempdir().unwrap();
        for id in ["1.21.4", "CON"] {
            let version = dir.path().join("versions").join(id);
            std::fs::create_dir_all(&version).unwrap();
            std::fs::write(
                version.join(format!("{id}.json")),
                format!(
                    r#"{{"id":"{id}","type":"release","mainClass":"Main","libraries":[],"downloads":{{"client":{{"url":"https://example.org/x"}}}}}}"#
                ),
            )
            .unwrap();
        }
        let (tx, _rx) = jobs::channel();
        let app = App::new(
            launcher(dir.path()),
            tx,
            Bootstrap {
                no_splash: true,
                ..Bootstrap::default()
            },
        );
        assert_eq!(app.versions.len(), 1);
        assert_eq!(app.versions[0].id, "1.21.4");
        assert!(app
            .logs
            .iter()
            .any(|line| line.text.contains("invalid version")));
    }

    #[test]
    fn requested_version_is_selected_when_installed() {
        let dir = tempfile::tempdir().unwrap();
        for id in ["1.21.4", "1.20.1"] {
            let version = dir.path().join("versions").join(id);
            std::fs::create_dir_all(&version).unwrap();
            std::fs::write(
                version.join(format!("{id}.json")),
                format!(
                    r#"{{"id":"{id}","type":"release","mainClass":"Main","libraries":[],"downloads":{{"client":{{"url":"https://example.org/x"}}}}}}"#
                ),
            )
            .unwrap();
        }
        let (tx, _rx) = jobs::channel();
        let app = App::new(
            launcher(dir.path()),
            tx,
            Bootstrap {
                version: Some("1.20.1".into()),
                no_splash: true,
                ..Bootstrap::default()
            },
        );
        assert_eq!(app.settings.active_version.as_deref(), Some("1.20.1"));
    }

    #[test]
    fn loader_preference_only_uses_installed_loaders() {
        let dir = tempfile::tempdir().unwrap();
        let version = dir.path().join("versions/1.21.4");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::write(
            version.join("1.21.4.json"),
            br#"{"id":"1.21.4","type":"release","mainClass":"Main","libraries":[],"downloads":{"client":{"url":"https://example.org/x"}}}"#,
        )
        .unwrap();
        let fabric = dir
            .path()
            .join("libraries/fabric/fabric-profile-1.21.4.json");
        std::fs::create_dir_all(fabric.parent().unwrap()).unwrap();
        std::fs::write(&fabric, b"{}").unwrap();
        let (tx, _rx) = jobs::channel();
        let mut app = App::new(
            launcher(dir.path()),
            tx,
            Bootstrap {
                no_splash: true,
                ..Bootstrap::default()
            },
        );
        assert_eq!(app.settings.active_version.as_deref(), Some("1.21.4"));
        assert_eq!(app.active_loader().as_deref(), Some("Fabric"));
        assert_eq!(app.active_loader_id().as_deref(), Some("fabric"));
        assert!(app.loader_installed("fabric"));
        assert!(!app.loader_installed("forge"));

        app.apply_pick(PickerKind::Loaders, "fabric");
        assert_eq!(app.settings.active_loader.as_deref(), Some("fabric"));
        assert_eq!(app.active_loader().as_deref(), Some("Fabric"));
        assert_eq!(app.active_loader_id().as_deref(), Some("fabric"));
        app.apply_pick(PickerKind::Loaders, "auto");
        assert_eq!(app.settings.active_loader, None);

        app.apply_pick(PickerKind::Loaders, "forge");
        assert!(matches!(app.modal, Some(Modal::Confirm { .. })));

        app.settings.active_loader = Some("neoforge".into());
        app.refresh_local();
        assert_eq!(app.settings.active_loader, None);
    }

    #[test]
    fn malformed_config_is_not_overwritten_until_an_explicit_save() {
        let dir = tempfile::tempdir().unwrap();
        let version = dir.path().join("versions/1.21.4");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::write(
            version.join("1.21.4.json"),
            br#"{"id":"1.21.4","type":"release","mainClass":"Main","libraries":[],"downloads":{"client":{"url":"https://example.org/x"}}}"#,
        )
        .unwrap();
        let config = Settings::path(dir.path());
        std::fs::write(&config, b"broken json").unwrap();
        let (tx, _rx) = jobs::channel();
        let mut app = App::new(
            launcher(dir.path()),
            tx,
            Bootstrap {
                no_splash: true,
                ..Bootstrap::default()
            },
        );
        assert_eq!(std::fs::read(&config).unwrap(), b"broken json");
        assert_eq!(app.settings.active_version.as_deref(), Some("1.21.4"));
        app.save_settings();
        let saved = std::fs::read(&config).unwrap();
        let parsed: Settings = serde_json::from_slice(&saved).unwrap();
        assert_eq!(parsed.active_version.as_deref(), Some("1.21.4"));
    }

    #[test]
    fn explicit_cli_overrides_are_applied_to_settings() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = jobs::channel();
        let app = App::new(
            launcher(dir.path()),
            tx,
            Bootstrap {
                ram: Some(6144),
                threads: Some(8),
                java: Some("/opt/java/bin/java".into()),
                width: Some(1600),
                height: Some(900),
                version: None,
                source: Some(SourceMode::BmclApi),
                no_splash: true,
            },
        );
        assert_eq!(app.settings.ram_mb, 6144);
        assert_eq!(app.settings.threads, 8);
        assert_eq!(app.settings.java, "/opt/java/bin/java");
        assert_eq!(app.settings.width, 1600);
        assert_eq!(app.settings.height, 900);
        assert_eq!(app.settings.source, "bmcl");
    }

    #[test]
    fn search_results_and_manifest_parsing_are_tolerant() {
        let hits = parse_search_hits(&serde_json::json!({
            "hits": [
                {"slug": "sodium", "title": "Sodium", "description": "Fast", "author": "jelly", "downloads": 1234, "categories": ["fabric", "optimization"], "project_id": "AABB"},
                {"title": "missing slug"},
            ]
        }));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].slug, "sodium");
        assert_eq!(hits[0].categories, "fabric, optimization");

        let remote = parse_remote_versions(
            &serde_json::json!({"versions": [
                {"id": "1.21.4", "type": "release", "releaseTime": "2024-12-03T10:00:00Z"},
                {"id": "24w01a", "type": "snapshot", "releaseTime": "2024-01-03T10:00:00Z"}
            ]}),
            &[],
        );
        assert_eq!(remote.len(), 2);
        assert_eq!(remote[0].id, "1.21.4");
        assert!(remote[0].label.contains("1.21.4"));
    }

    #[test]
    fn picker_filter_is_case_insensitive_and_bounds_selection() {
        let mut picker = PickerState::new(
            "t",
            "h",
            PickerKind::RemoteVersions,
            vec![
                PickerItem {
                    id: "1.21.4".into(),
                    label: "1.21.4".into(),
                    meta: "release".into(),
                    tag: "release".into(),
                    installed: false,
                },
                PickerItem {
                    id: "1.20.1".into(),
                    label: "1.20.1".into(),
                    meta: "release".into(),
                    tag: "release".into(),
                    installed: true,
                },
            ],
        );
        picker.input.set("1.20");
        picker.refilter();
        assert_eq!(picker.filtered.len(), 1);
        picker.move_selection(10);
        assert_eq!(picker.selected, 0);
        assert_eq!(picker.selected_item().unwrap().id, "1.20.1");
    }

    #[test]
    fn settings_presets_cycle() {
        assert_eq!(cycle_preset(&[0, 10, 20], 10, 1), 20);
        assert_eq!(cycle_preset(&[0, 10, 20], 20, 1), 0);
        assert_eq!(cycle_preset(&[0, 10, 20], 0, -1), 20);
        assert_eq!(cycle_preset(&[0, 10], 99, 1), 10);
    }

    #[test]
    fn shader_install_prompts_for_iris_or_optifine() {
        let dir = tempfile::tempdir().unwrap();
        let version = dir.path().join("versions").join("1.21.4");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::write(
            version.join("1.21.4.json"),
            br#"{"id":"1.21.4","type":"release","mainClass":"Main","libraries":[],"downloads":{"client":{"url":"https://example.org/x"}}}"#,
        )
        .unwrap();
        let profile = dir
            .path()
            .join("libraries")
            .join("fabric")
            .join("fabric-profile-1.21.4.json");
        std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
        std::fs::write(profile, b"{}").unwrap();
        let (tx, _rx) = jobs::channel();
        let mut app = App::new(
            launcher(dir.path()),
            tx,
            Bootstrap {
                no_splash: true,
                ..Bootstrap::default()
            },
        );
        app.settings.active_version = Some("1.21.4".into());
        app.refresh_local();

        app.pending_shader_loader = Some((
            "BSL Shaders".into(),
            ShaderSupport {
                iris: true,
                optifine: false,
            },
        ));
        app.job = Some(JobView::new(9, "install shader"));
        app.finish_job(9, Ok("Installed shader".into()));
        assert!(matches!(
            &app.modal,
            Some(Modal::Confirm {
                action: ConfirmAction::InstallMod { slug, .. },
                ..
            }) if slug == "iris"
        ));

        app.modal = None;
        app.pending_shader_loader = Some((
            "OptiFine Only".into(),
            ShaderSupport {
                iris: false,
                optifine: true,
            },
        ));
        app.job = Some(JobView::new(10, "install shader"));
        app.finish_job(10, Ok("Installed shader".into()));
        assert!(matches!(
            &app.modal,
            Some(Modal::Confirm {
                action: ConfirmAction::Dismiss,
                ..
            })
        ));
    }

    #[test]
    fn java_setting_accepts_executables_and_jdk_folders() {
        assert_eq!(resolve_java_setting("", Lang::English).unwrap().0, "");
        assert!(resolve_java_setting("/definitely/missing/java", Lang::English).is_err());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path().join("jdk-21");
            std::fs::create_dir_all(home.join("bin")).unwrap();
            let java = home.join("bin").join("java");
            std::fs::write(
                &java,
                "#!/bin/sh\nif [ \"$1\" = '-version' ]; then echo 'openjdk version \"21.0.1\"' >&2; fi\nexit 0\n",
            )
            .unwrap();
            std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o700)).unwrap();

            let (resolved, major) =
                resolve_java_setting(home.to_str().unwrap(), Lang::English).unwrap();
            assert_eq!(resolved, java.to_string_lossy());
            assert_eq!(major, Some(21));

            let (resolved, _) =
                resolve_java_setting(java.to_str().unwrap(), Lang::Chinese).unwrap();
            assert_eq!(resolved, java.to_string_lossy());

            let not_java = home.join("bin").join("readme");
            std::fs::write(&not_java, b"not executable").unwrap();
            assert!(resolve_java_setting(not_java.to_str().unwrap(), Lang::English).is_err());

            let empty_home = dir.path().join("empty-jdk");
            std::fs::create_dir_all(&empty_home).unwrap();
            assert!(resolve_java_setting(empty_home.to_str().unwrap(), Lang::Chinese).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn settings_expose_java_picker_jvm_args_and_gc() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("java").join("java-runtime-gamma");
        std::fs::create_dir_all(home.join("bin")).unwrap();
        let exe = home.join("bin").join("java");
        std::fs::write(
            &exe,
            "#!/bin/sh\nif [ \"$1\" = '-version' ]; then echo 'openjdk version \"21.0.1\"' >&2; fi\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(
            home.join("release"),
            b"IMPLEMENTOR=\"Test Vendor\"\nJAVA_VERSION=\"21.0.1\"\nOS_ARCH=\"x86_64\"\n",
        )
        .unwrap();

        let (tx, _rx) = jobs::channel();
        let mut app = App::new(
            launcher(dir.path()),
            tx,
            Bootstrap {
                no_splash: true,
                ..Bootstrap::default()
            },
        );

        app.settings_cursor = 6;
        app.settings_activate();
        let items = match &app.modal {
            Some(Modal::Picker(picker)) => picker.items.clone(),
            _ => panic!("expected the Java picker"),
        };
        assert!(items.iter().any(|item| item.id == "__auto__"));
        assert!(items.iter().any(|item| item.id == "__browse__"));
        let java_id = exe.to_string_lossy().to_string();
        assert!(
            items.iter().any(|item| item.id == java_id),
            "items: {items:?}"
        );

        app.apply_pick(PickerKind::JavaRuntimes, &java_id);
        assert_eq!(app.settings.java, java_id);
        app.apply_pick(PickerKind::JavaRuntimes, "__auto__");
        assert!(app.settings.java.is_empty());

        app.settings_cursor = 7;
        app.settings_activate();
        assert!(matches!(
            app.modal,
            Some(Modal::Input {
                action: InputAction::JvmArgs,
                ..
            })
        ));
        app.apply_input(InputAction::JvmArgs, "-Dfoo=bar -Xss2M")
            .unwrap();
        assert_eq!(app.settings.jvm_arguments(), vec!["-Dfoo=bar", "-Xss2M"]);
        assert!(app
            .apply_input(InputAction::JvmArgs, "'unterminated")
            .is_err());

        app.settings_cursor = 8;
        app.settings_adjust(1);
        assert_eq!(app.settings.gc, "g1");
        app.settings_adjust(1);
        assert_eq!(app.settings.gc, "zgc");
        app.settings_adjust(-1);
        assert_eq!(app.settings.gc, "g1");
    }

    #[test]
    fn versions_screen_renames_and_deletes_instances() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let dir = tempfile::tempdir().unwrap();
        let version = dir.path().join("versions").join("1.21.4");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::write(
            version.join("1.21.4.json"),
            br#"{"id":"1.21.4","type":"release","mainClass":"Main","libraries":[],"downloads":{"client":{"url":"https://example.org/x"}}}"#,
        )
        .unwrap();
        let (tx, rx) = jobs::channel();
        let mut app = App::new(
            launcher(dir.path()),
            tx,
            Bootstrap {
                no_splash: true,
                ..Bootstrap::default()
            },
        );
        assert_eq!(app.versions.len(), 1);
        app.screen = Screen::Versions;
        app.version_sel = 0;

        app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
        assert!(matches!(
            app.modal,
            Some(Modal::Input {
                action: InputAction::RenameVersion,
                ..
            })
        ));
        assert!(app
            .apply_input(InputAction::RenameVersion, "../escape")
            .is_err());
        assert!(
            app.rename_source.is_some(),
            "the source instance must survive a validation error"
        );
        app.apply_input(InputAction::RenameVersion, "my-pack")
            .unwrap();
        app.modal = None;
        assert!(dir.path().join("versions/my-pack/my-pack.json").is_file());
        assert_eq!(app.settings.active_version.as_deref(), Some("my-pack"));

        app.refresh_local();
        app.version_sel = 0;
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        let confirmed = match app.modal.take() {
            Some(Modal::Confirm {
                action: ConfirmAction::RemoveVersion(id),
                ..
            }) => id,
            _ => panic!("expected a delete confirmation"),
        };
        app.apply_confirm(ConfirmAction::RemoveVersion(confirmed));
        // Removal runs on a worker thread; drain its events until it finishes.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            std::thread::sleep(std::time::Duration::from_millis(10));
            app.tick();
            while let Ok(event) = rx.try_recv() {
                app.handle_backend(event);
            }
            if !dir.path().join("versions/my-pack").exists()
                && app.job.as_ref().is_none_or(|job| job.finished.is_some())
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "background removal did not finish"
            );
        }
        assert!(!dir.path().join("versions/my-pack").exists());
        assert!(app.versions.is_empty());
    }

    #[test]
    fn mod_updates_are_matched_to_installed_jars() {
        let dir = tempfile::tempdir().unwrap();
        let version = dir.path().join("versions").join("1.21.4");
        std::fs::create_dir_all(version.join("mods")).unwrap();
        std::fs::write(
            version.join("1.21.4.json"),
            br#"{"id":"1.21.4","type":"release","mainClass":"Main","libraries":[],"downloads":{"client":{"url":"https://example.org/x"}}}"#,
        )
        .unwrap();
        std::fs::write(version.join("mods").join("sodium.jar"), b"jar").unwrap();
        let (tx, _rx) = jobs::channel();
        let mut app = App::new(
            launcher(dir.path()),
            tx,
            Bootstrap {
                no_splash: true,
                ..Bootstrap::default()
            },
        );
        assert_eq!(app.mods.len(), 1);
        app.mod_updates = vec![crate::update::ModUpdate {
            file: version.join("mods").join("sodium.jar"),
            filename: "sodium-2.jar".into(),
            enabled: true,
            project: "Sodium".into(),
            current: "0.5.0".into(),
            latest: "0.6.0".into(),
            download_url: "https://example.invalid/sodium.jar".into(),
            download_name: "sodium-2.jar".into(),
            download_sha1: None,
        }];
        app.refresh_mods();
        assert_eq!(app.mods[0].update.as_deref(), Some("0.6.0"));
        app.confirm_apply_updates();
        assert!(matches!(
            app.modal,
            Some(Modal::Confirm {
                action: ConfirmAction::ApplyUpdates,
                ..
            })
        ));
    }
}
