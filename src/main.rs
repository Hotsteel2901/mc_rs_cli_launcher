//! MC CLI Launcher -- Rust rewrite
//!
//! Simple Minecraft CLI launcher: Microsoft login, offline mode,
//! Fabric/Forge/NeoForge loaders, and Modrinth mods.

mod account;
mod auth;
mod content;
mod diagnose;
mod discover;
mod error;
mod fabric;
mod forge;
mod http;
mod installer;
mod instance;
mod java;
mod launcher;
pub mod log;
mod maintenance;
mod mod_manager;
mod modrinth;
mod neoforge;
mod progress;
mod tui;
mod update;
mod util;
mod verify;
mod version;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::parser::ValueSource;
use clap::{arg, value_parser, ArgAction, Command};

use crate::auth::MicrosoftAuth;
use crate::content::{game_mode_label, ContentManager};
use crate::fabric::FabricManager;
use crate::forge::ForgeManager;
use crate::http::SourceMode;
use crate::instance::InstanceManager;
use crate::launcher::MinecraftLauncher;
use crate::mod_manager::{DependencyMode, ModManager};
use crate::modrinth as mr;
use crate::neoforge::NeoForgeManager;
use crate::version::VersionManager;

/// Parse a RAM string like "4G", "2048M", or plain integer in MB.
fn parse_ram(value: &str) -> Result<u32, String> {
    let original = value.trim();
    let upper = original.to_ascii_uppercase();
    // Accept the same grammar as the old regex: decimal number + optional
    // G/GB/M/MB. Parsing by hand avoids compiling a regex for every CLI run.
    let (number, scale) = if let Some(rest) = upper.strip_suffix("GB") {
        (rest, 1024.0)
    } else if let Some(rest) = upper.strip_suffix('G') {
        (rest, 1024.0)
    } else if let Some(rest) = upper.strip_suffix("MB") {
        (rest, 1.0)
    } else if let Some(rest) = upper.strip_suffix('M') {
        (rest, 1.0)
    } else {
        (upper.as_str(), 1.0)
    };
    let number = number.trim();
    if number.is_empty()
        || number.matches('.').count() > 1
        || !number.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        return Err(format!(
            "Invalid RAM value: '{original}'. Use format like 4G, 2048M, or a plain number."
        ));
    }
    let num: f64 = number.parse().map_err(|_| {
        format!("Invalid RAM value: '{original}'. Use format like 4G, 2048M, or a plain number.")
    })?;
    let mb = num * scale;
    if !mb.is_finite() || mb < util::MIN_RAM_MB as f64 || mb > util::MAX_RAM_MB as f64 {
        return Err(format!(
            "RAM must be between {} MB and {} MB",
            util::MIN_RAM_MB,
            util::MAX_RAM_MB
        ));
    }
    Ok(mb as u32)
}

fn parse_server_arg(value: &str) -> Result<String, String> {
    crate::launcher::parse_server(value)?;
    Ok(value.to_string())
}

/// Split a command-line fragment into arguments, honouring single/double quotes.
fn split_args(value: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut has_content = false;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {
                current.push(ch);
                has_content = true;
            }
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    has_content = true;
                }
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                        has_content = true;
                    }
                }
                c if c.is_whitespace() => {
                    if has_content {
                        args.push(std::mem::take(&mut current));
                        has_content = false;
                    }
                }
                c => {
                    current.push(c);
                    has_content = true;
                }
            },
        }
    }
    if quote.is_some() {
        return Err("Unterminated quote in --jvm-args".into());
    }
    if has_content {
        args.push(current);
    }
    Ok(args)
}

fn parse_component(value: &str) -> Result<String, String> {
    util::validate_component(value)?;
    Ok(value.to_string())
}

fn parse_threads(value: &str) -> Result<usize, String> {
    let n = value
        .parse::<usize>()
        .map_err(|_| "Threads must be an integer from 1 to 64")?;
    if !(1..=64).contains(&n) {
        return Err("Threads must be from 1 to 64".into());
    }
    Ok(n)
}

/// Repository locations shown by `--about` and by the TUI About screen. Both
/// hosts keep the same repository name.
const GITHUB_REPO: &str = "https://github.com/Hotsteel2901/mc_rs_cli_launcher";
const CODEBERG_REPO: &str = "https://codeberg.org/hotsteel/mc_rs_cli_launcher";

fn print_about() {
    let bold = |text: &str| crate::log::clr("\x1b[1m", text);
    let accent = |text: &str| crate::log::clr("\x1b[92m", text);
    let dim = |text: &str| crate::log::clr("\x1b[90m", text);
    let row = |label: &str, value: String| println!("  {:<9}{}", bold(label), value);
    println!();
    println!(
        "  {} {}",
        bold("Hot-RS-Launcher"),
        dim(env!("CARGO_PKG_VERSION"))
    );
    println!(
        "  {}",
        dim("A Rust Minecraft launcher: Microsoft/offline accounts, Fabric/Forge/NeoForge,")
    );
    println!(
        "  {}",
        dim("Modrinth mods and an optional full-screen terminal UI.")
    );
    println!();
    row("Author", "Hotsteel2901".into());
    row(
        "GitHub",
        format!("{}  (@Hotsteel2901)", accent(GITHUB_REPO)),
    );
    row(
        "Codeberg",
        format!("{}  (@hotsteel)", accent(CODEBERG_REPO)),
    );
    row("License", "GPL-3.0-only".into());
    println!();
    println!(
        "  {}",
        dim("Both repositories use the same name: mc_rs_cli_launcher")
    );
    println!();
}

fn build_cli() -> Command {
    Command::new("hot-rs-launcher")
        .disable_version_flag(true)
        .version(env!("CARGO_PKG_VERSION"))
        .about(
            "Hot-RS-Launcher -- a Rust Minecraft launcher: Microsoft + offline + \
             Fabric/Forge/NeoForge + Modrinth mods + optional TUI",
        )
        .after_help(
            "Examples:\n  \
             hot-rs-launcher login                       # Microsoft login (browser)\n  \
             hot-rs-launcher login --device-code         # Device code login\n  \
             hot-rs-launcher offline Steve               # Offline mode\n  \
             hot-rs-launcher play                        # Launch with saved account\n  \
             hot-rs-launcher play -v 1.21.4              # Launch specific version\n  \
             hot-rs-launcher play -v 1.21.4 --ram 4G     # Allocate 4 GB RAM\n  \
             hot-rs-launcher play --fabric               # Launch with Fabric\n  \
             hot-rs-launcher play --forge                # Launch with Forge\n  \
             hot-rs-launcher play --neoforge             # Launch with NeoForge\n  \
             hot-rs-launcher accounts                    # Show saved accounts\n  \
             hot-rs-launcher download                    # Download latest version\n  \
             hot-rs-launcher download -v 1.20.1 --no-assets\n  \
             hot-rs-launcher list-versions               # List all MC versions\n  \
             hot-rs-launcher list-loaders                # List all mod loaders\n  \
             hot-rs-launcher search sodium               # Search mods on Modrinth\n  \
             hot-rs-launcher search-more sodium          # Detailed mod info\n  \
             hot-rs-launcher install-fabric -v 1.21.4    # Install Fabric\n  \
             hot-rs-launcher install-forge -v 1.20.1     # Install Forge\n  \
             hot-rs-launcher install-neoforge -v 1.21.4  # Install NeoForge\n  \
             hot-rs-launcher install-mod sodium -v 1.21.4\n  \
             hot-rs-launcher list-installed              # List installed versions\n  \
             hot-rs-launcher list-mods -v 1.21.4         # List mods\n  \
             hot-rs-launcher disable-mod sodium -v 1.21.4\n  \
             hot-rs-launcher enable-mod sodium -v 1.21.4\n  \
             hot-rs-launcher uninstall-mod sodium -v 1.21.4\n  \
             hot-rs-launcher logout                      # Clear saved session\n  \
             hot-rs-launcher list-java                   # Show detected Java runtimes\n  \
             hot-rs-launcher install-java --java-major 21\n  \
             hot-rs-launcher remove-version -v 1.20.1 --keep-saves\n  \
             hot-rs-launcher rename-version -v 1.21.4 --name my-pack\n  \
             hot-rs-launcher export-modpack -v 1.21.4 -o pack.mrpack\n  \
             hot-rs-launcher import-modpack pack.mrpack\n  \
             hot-rs-launcher update-mods -v 1.21.4 --apply\n  \
             hot-rs-launcher backup -v 1.21.4 --with-mods\n  \
             hot-rs-launcher diagnose -v 1.21.4          # Analyze the latest crash\n  \
             hot-rs-launcher play --dry-run --gc g1       # Print the command only\n  \
             hot-rs-launcher list-saves -v 1.21.4         # List worlds\n  \
             hot-rs-launcher backup-save 'My World' -v 1.21.4\n  \
             hot-rs-launcher import-save ./MyWorld.zip -v 1.21.4\n  \
             hot-rs-launcher list-resourcepacks -v 1.21.4\n  \
             hot-rs-launcher enable-resourcepack MyPack.zip -v 1.21.4\n  \
             hot-rs-launcher import-resourcepack ./MyPack.zip -v 1.21.4\n  \
             hot-rs-launcher search --browse -v 1.21.4    # Browse popular mods\n  \
             hot-rs-launcher search --browse --category optimization -v 1.21.4\n  \
             hot-rs-launcher search-resourcepacks -v 1.21.4\n  \
             hot-rs-launcher download-resourcepack translations-for-sodium -v 1.21.4 --enable\n  \
             hot-rs-launcher search-shaders -v 1.21.4     # Shows Iris/OptiFine support\n  \
             hot-rs-launcher download-shader bsl-shaders -v 1.21.4 --enable\n  \
             hot-rs-launcher list-shaders -v 1.21.4\n  \
             hot-rs-launcher optimize-mods -v 1.21.4 --limit 8\n  \
             hot-rs-launcher optimize-mods -v 1.21.4 --apply --yes\n  \
             hot-rs-launcher tui                         # Full-screen terminal UI",
        )
        // Global options
        .arg(arg!(-d --dir <DIR> "Game directory"))
        .arg(
            arg!(-t --threads <N> "Parallel download threads (assets scale to 2x, cap 64)")
                .value_parser(parse_threads)
                .default_value("16"),
        )
        .arg(arg!(-j --java <PATH> "Path to Java executable"))
        .arg(arg!(-v --version <VERSION> "Minecraft version").value_parser(parse_component))
        .arg(
            arg!(-l --loader <LOADER> "Mod loader filter (fabric, forge, neoforge)")
                .value_parser(["fabric", "forge", "neoforge"]),
        )
        .arg(arg!(--"loader-version" <VERSION> "Specific loader version ID"))
        .arg(arg!(--"mod-version" <ID> "Specific mod version ID"))
        .arg(
            arg!(-r --ram <RAM> "RAM allocation, 128M-64G (e.g. 4G, 2048M)")
                .value_parser(parse_ram)
                .default_value("4096"),
        )
        .arg(arg!(--width <PIXELS> "Game window width").value_parser(value_parser!(u32).range(1..)))
        .arg(
            arg!(--height <PIXELS> "Game window height")
                .value_parser(value_parser!(u32).range(1..)),
        )
        .arg(arg!(--fabric "Launch with Fabric loader").action(ArgAction::SetTrue))
        .arg(arg!(--forge "Launch with Forge loader").action(ArgAction::SetTrue))
        .arg(arg!(--neoforge "Launch with NeoForge loader").action(ArgAction::SetTrue))
        .arg(arg!(--"no-assets" "Skip asset downloads").action(ArgAction::SetTrue))
        .arg(
            arg!(--official "Force use official Mojang/Forge URLs (no mirror)")
                .action(ArgAction::SetTrue),
        )
        .arg(arg!(--bmcl "Use BMCLAPI where supported (大陆用户推荐)").action(ArgAction::SetTrue))
        .arg(
            arg!(--proxy <URL> "HTTP(S) proxy, e.g. http://127.0.0.1:7890 (also reads HTTPS_PROXY)"),
        )
        .arg(arg!(--"device-code" "Use device code login").action(ArgAction::SetTrue))
        .arg(
            arg!(--limit <N> "Max search results")
                .value_parser(value_parser!(u32).range(1..=100))
                .default_value("10"),
        )
        .arg(
            arg!(--"java-major" <N> "Java major version for install-java (e.g. 8, 17, 21)")
                .value_parser(value_parser!(u32).range(8..=99)),
        )
        .arg(
            arg!(--"jvm-args" <ARGS> "Extra JVM arguments (quotes supported)")
                .allow_hyphen_values(true),
        )
        .arg(
            arg!(--gc <PRESET> "GC preset: auto, g1, zgc, parallel, serial, none")
                .value_parser(clap::builder::ValueParser::new(
                    crate::launcher::GcPreset::parse,
                ))
                .default_value("auto"),
        )
        .arg(arg!(--fullscreen "Start the game in fullscreen").action(ArgAction::SetTrue))
        .arg(
            arg!(--server <ADDR> "Join a server directly (host or host:port)")
                .value_parser(parse_server_arg),
        )
        .arg(
            arg!(--"dry-run" "Print the launch command without starting the game")
                .action(ArgAction::SetTrue),
        )
        .arg(
            arg!(--"no-diagnose" "Skip crash analysis after a failed launch")
                .action(ArgAction::SetTrue),
        )
        .arg(arg!(--apply "Apply available mod updates").action(ArgAction::SetTrue))
        .arg(arg!(--all "Include disabled mods when checking updates").action(ArgAction::SetTrue))
        .arg(arg!(-o --output <PATH> "Output file for export-modpack / backup"))
        .arg(arg!(--name <NAME> "New instance id (rename/copy) or pack name (import)"))
        .arg(
            arg!(--"keep-saves" "Keep saves/ when removing an instance").action(ArgAction::SetTrue),
        )
        .arg(arg!(--"keep-mods" "Keep mods/ when removing an instance").action(ArgAction::SetTrue))
        .arg(arg!(--"with-mods" "Include mods/ in backups").action(ArgAction::SetTrue))
        .arg(
            arg!(--"install-loader" "Install the loader needed by an imported modpack or shader")
                .action(ArgAction::SetTrue),
        )
        .arg(
            arg!(--json "Print machine-readable JSON for list commands").action(ArgAction::SetTrue),
        )
        .arg(
            arg!(--yes "Confirm destructive operations and auto-install required mod dependencies")
                .action(ArgAction::SetTrue),
        )
        .arg(
            arg!(--"no-deps" "Do not install or prompt for required mod dependencies")
                .action(ArgAction::SetTrue),
        )
        .arg(
            arg!(--page <N> "Search/browse page, 1-based")
                .value_parser(value_parser!(u32).range(1..=1000))
                .default_value("1"),
        )
        .arg(
            arg!(--sort <SORT> "Search sort: relevance, downloads, newest, updated")
                .value_parser(["relevance", "downloads", "newest", "updated"])
                .default_value("relevance"),
        )
        .arg(
            arg!(--category <CAT> "Modrinth category filter (e.g. optimization)")
                .value_parser(parse_component),
        )
        .arg(arg!(--browse "Browse popular projects without a query").action(ArgAction::SetTrue))
        .arg(
            arg!(--deep "Re-hash cached files instead of only checking presence (repair)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            arg!(--enable "Enable a downloaded resource pack or shader pack")
                .action(ArgAction::SetTrue),
        )
        // Positional: action
        .arg(
            arg!([ACTION] "Action to perform")
                .default_value("play")
                .value_parser([
                    "login",
                    "offline",
                    "play",
                    "launch",
                    "download",
                    "logout",
                    "accounts",
                    "list-versions",
                    "list-loaders",
                    "list-installed",
                    "list-mods",
                    "search",
                    "search-more",
                    "search-resourcepacks",
                    "search-shaders",
                    "download-resourcepack",
                    "download-shader",
                    "optimize-mods",
                    "install-fabric",
                    "install-forge",
                    "install-neoforge",
                    "install-mod",
                    "disable-mod",
                    "enable-mod",
                    "uninstall-mod",
                    "list-java",
                    "install-java",
                    "remove-version",
                    "delete-version",
                    "rename-version",
                    "copy-version",
                    "export-modpack",
                    "import-modpack",
                    "update-mods",
                    "diagnose",
                    "backup",
                    "list-backups",
                    "restore-backup",
                    "list-saves",
                    "backup-save",
                    "import-save",
                    "delete-save",
                    "list-resourcepacks",
                    "enable-resourcepack",
                    "disable-resourcepack",
                    "import-resourcepack",
                    "delete-resourcepack",
                    "list-shaders",
                    "enable-shader",
                    "disable-shader",
                    "import-shader",
                    "delete-shader",
                    "doctor",
                    "prune",
                    "repair",
                    "completions",
                    "tui",
                    "ui",
                ]),
        )
        // Positional: query
        .arg(arg!([QUERY] "Username (offline) or mod search query / mod slug"))
        .arg(arg!(--"launcher-version" "Print launcher version").action(ArgAction::Version))
        .arg(
            arg!(--about "Show project, author and repository information")
                .action(ArgAction::SetTrue),
        )
        .arg(arg!(--verbose "Enable diagnostic logging").action(ArgAction::SetTrue))
        .group(
            clap::ArgGroup::new("loader-choice")
                .args(["loader", "fabric", "forge", "neoforge"])
                .multiple(false),
        )
        .group(
            clap::ArgGroup::new("source-choice")
                .args(["official", "bmcl"])
                .multiple(false),
        )
}

/// Restore the default SIGPIPE behaviour so piping into head/tail exits
/// quietly instead of panicking on a closed stdout.
#[cfg(unix)]
fn reset_sigpipe() {
    extern "C" {
        fn signal(signum: i32, handler: usize) -> usize;
    }
    const SIGPIPE: i32 = 13;
    const SIG_DFL: usize = 0;
    unsafe {
        signal(SIGPIPE, SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

fn main() {
    reset_sigpipe();
    let matches = build_cli().get_matches();
    crate::log::set_verbose(matches.get_flag("verbose"));
    // Must happen before the lazily-built HTTP agent handles its first request.
    if let Some(proxy) = matches.get_one::<String>("proxy") {
        http::set_proxy(Some(proxy)).unwrap_or_else(|error| crate::die!(error));
    }
    if matches.get_flag("about") {
        print_about();
        return;
    }

    // Extract options
    let game_dir = PathBuf::from(
        matches
            .get_one::<String>("dir")
            .cloned()
            .unwrap_or_else(|| {
                crate::util::default_game_dir()
                    .to_string_lossy()
                    .to_string()
            }),
    );
    let game_dir = util::absolute_path(&game_dir).unwrap_or_else(|e| crate::die!(e));
    let threads = *matches.get_one::<usize>("threads").unwrap();
    let mut action = matches.get_one::<String>("ACTION").unwrap().clone();
    let query = matches.get_one::<String>("QUERY").cloned();
    let mc_version = matches.get_one::<String>("version").cloned();
    let loader_arg = matches.get_one::<String>("loader").cloned();
    let loader_version = matches.get_one::<String>("loader-version").cloned();
    let mod_version = matches.get_one::<String>("mod-version").cloned();
    let ram_mb = *matches.get_one::<u32>("ram").unwrap();
    let java_path = matches.get_one::<String>("java").cloned();
    let width = matches.get_one::<u32>("width").copied();
    let height = matches.get_one::<u32>("height").copied();
    let no_assets = matches.get_flag("no-assets");
    let device_code = matches.get_flag("device-code");
    let official_flag = matches.get_flag("official");
    let bmcl_flag = matches.get_flag("bmcl");
    let fabric_flag = matches.get_flag("fabric");
    let forge_flag = matches.get_flag("forge");
    let neoforge_flag = matches.get_flag("neoforge");
    let limit = *matches.get_one::<u32>("limit").unwrap();
    let java_major = matches.get_one::<u32>("java-major").copied();
    let jvm_args = match matches.get_one::<String>("jvm-args") {
        Some(value) => split_args(value).unwrap_or_else(|e| crate::die!(e)),
        None => Vec::new(),
    };
    let gc = matches
        .get_one::<crate::launcher::GcPreset>("gc")
        .copied()
        .unwrap_or_default();
    let fullscreen = matches.get_flag("fullscreen");
    let server = matches.get_one::<String>("server").cloned();
    let dry_run = matches.get_flag("dry-run");
    let diagnose = !matches.get_flag("no-diagnose");
    let apply_updates = matches.get_flag("apply");
    let include_disabled = matches.get_flag("all");
    let output = matches
        .get_one::<String>("output")
        .cloned()
        .map(PathBuf::from);
    let new_name = matches.get_one::<String>("name").cloned();
    let keep_saves = matches.get_flag("keep-saves");
    let keep_mods = matches.get_flag("keep-mods");
    let with_mods = matches.get_flag("with-mods");
    let install_loader = matches.get_flag("install-loader");
    let json_output = matches.get_flag("json");
    let assume_yes = matches.get_flag("yes");
    let no_deps = matches.get_flag("no-deps");
    let page = *matches.get_one::<u32>("page").unwrap();
    let sort = matches
        .get_one::<String>("sort")
        .cloned()
        .unwrap_or_else(|| "relevance".into());
    let category = matches.get_one::<String>("category").cloned();
    let browse = matches.get_flag("browse");
    let enable_pack = matches.get_flag("enable");
    let deep = matches.get_flag("deep");

    // Resolve source mode: manual > auto-detect
    let manual_mode = if official_flag && bmcl_flag {
        crate::die!("Cannot use both --official and --bmcl.");
    } else if official_flag {
        Some(SourceMode::Official)
    } else if bmcl_flag {
        Some(SourceMode::BmclApi)
    } else {
        None
    };

    let resolved_mode = http::resolve_source_mode(manual_mode);
    http::set_source_mode(resolved_mode);
    let mode_label = match resolved_mode {
        SourceMode::Official => "official",
        SourceMode::BmclApi => "BMCLAPI mirror",
        _ => "auto",
    };
    crate::debug_msg!("Download source: {}", mode_label);

    // Normalize alias
    if action == "launch" {
        action = "play".to_string();
    }
    if action == "ui" {
        action = "tui".to_string();
    }
    if action == "tui" && !tui::terminal_available() {
        crate::die!(
            "The TUI needs an interactive terminal with ANSI support.",
            "  Run it from a normal terminal (not TERM=dumb), or use 'hot-rs-launcher play' / '--help'."
        );
    }

    // Resolve loader from flags
    let loader = resolve_loader(loader_arg, fabric_flag, forge_flag, neoforge_flag);

    if no_assets && action != "download" {
        crate::die!("--no-assets is only supported for the download action.");
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
        .unwrap_or_else(|e| crate::die!(format!("Cannot initialize download workers: {e}")));
    let mut launcher =
        MinecraftLauncher::new(&game_dir, threads).unwrap_or_else(|e| crate::die!(e));
    if let Some(ref j) = java_path {
        launcher.set_java(j).unwrap_or_else(|e| crate::die!(e));
    }

    // --- Dispatch ---
    match action.as_str() {
        "logout" => cmd_logout(&mut launcher),
        "accounts" => cmd_accounts(&launcher),
        "list-versions" => cmd_list_versions(json_output),
        "list-loaders" => cmd_list_loaders(),
        "list-installed" => cmd_list_installed(&game_dir, &launcher, json_output),
        "list-mods" => cmd_list_mods(&game_dir, mc_version, json_output),
        "search" => cmd_search(
            &game_dir,
            query,
            limit,
            mc_version,
            loader.as_deref(),
            page,
            sort.clone(),
            category.clone(),
            browse,
            json_output,
        ),
        "search-resourcepacks" | "search-shaders" => {
            let kind = if action == "search-shaders" {
                crate::discover::DiscoverKind::Shaders
            } else {
                crate::discover::DiscoverKind::ResourcePacks
            };
            cmd_search_kind(
                &game_dir,
                kind,
                query,
                limit,
                mc_version,
                None,
                page,
                sort.clone(),
                None,
                false,
                json_output,
            );
        }
        "download-resourcepack" => {
            let slug = query.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the Modrinth project slug.",
                    "Example: hot-rs-launcher download-resourcepack stay-true -v 1.21.4"
                )
            });
            cmd_download_resourcepack(&game_dir, mc_version.clone(), &slug, enable_pack);
        }
        "download-shader" => {
            let slug = query.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the Modrinth project slug.",
                    "Example: hot-rs-launcher download-shader complementary-reimagined -v 1.21.4"
                )
            });
            cmd_download_shader(
                &game_dir,
                mc_version.clone(),
                &slug,
                enable_pack,
                install_loader,
            );
        }
        "optimize-mods" => cmd_optimize_mods(
            &game_dir,
            mc_version.clone(),
            loader.clone(),
            limit,
            apply_updates,
            assume_yes,
            json_output,
        ),
        "search-more" => cmd_search_more(&game_dir, query),
        "install-fabric" => cmd_install_fabric(&game_dir, mc_version, loader_version.as_deref()),
        "install-forge" => cmd_install_forge(
            &game_dir,
            mc_version,
            loader_version.as_deref(),
            java_path.as_deref(),
        ),
        "install-neoforge" => cmd_install_neoforge(
            &game_dir,
            mc_version,
            loader_version.as_deref(),
            java_path.as_deref(),
        ),
        "install-mod" => cmd_install_mod(
            &game_dir,
            query,
            mc_version,
            loader.as_deref(),
            mod_version.as_deref(),
            if no_deps {
                DependencyMode::Skip
            } else if assume_yes {
                DependencyMode::Auto
            } else {
                DependencyMode::Ask
            },
        ),
        "disable-mod" => cmd_disable_mod(&game_dir, query, mc_version),
        "enable-mod" => cmd_enable_mod(&game_dir, query, mc_version),
        "uninstall-mod" => cmd_uninstall_mod(&game_dir, query, mc_version),
        "login" => cmd_login(&mut launcher, device_code),
        "offline" => cmd_offline(&mut launcher, query.as_deref().unwrap_or("Steve")),
        "download" => cmd_download(&mut launcher, mc_version.as_deref(), no_assets),
        "list-java" => cmd_list_java(&game_dir, mc_version.clone(), json_output),
        "install-java" => cmd_install_java(&game_dir, java_major, threads),
        "remove-version" | "delete-version" => {
            let id = instance_arg(&query, &mc_version);
            cmd_remove_version(&game_dir, &id, keep_saves, keep_mods);
        }
        "rename-version" => {
            let from = instance_arg(&query, &mc_version);
            let to = new_name.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the new instance id with --name.",
                    "Example: hot-rs-launcher rename-version -v 1.21.4 --name my-pack"
                )
            });
            cmd_rename_version(&game_dir, &from, &to);
        }
        "copy-version" => {
            let from = instance_arg(&query, &mc_version);
            let to = new_name.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the new instance id with --name.",
                    "Example: hot-rs-launcher copy-version -v 1.21.4 --name 1.21.4-b"
                )
            });
            cmd_copy_version(&game_dir, &from, &to);
        }
        "export-modpack" => {
            let id = instance_arg(&query, &mc_version);
            cmd_export_modpack(&game_dir, &id, loader.as_deref(), output.clone());
        }
        "import-modpack" => {
            let archive = query.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the .mrpack file to import.",
                    "Example: hot-rs-launcher import-modpack ./pack.mrpack"
                )
            });
            cmd_import_modpack(
                &game_dir,
                std::path::Path::new(&archive),
                new_name.as_deref(),
                install_loader,
                java_path.as_deref(),
            );
        }
        "update-mods" => cmd_update_mods(
            &game_dir,
            mc_version.clone(),
            loader.clone(),
            apply_updates,
            include_disabled,
        ),
        "diagnose" => cmd_diagnose(
            &game_dir,
            query.clone().or_else(|| mc_version.clone()),
            json_output,
        ),
        "backup" => {
            let id = instance_arg(&query, &mc_version);
            cmd_backup(&game_dir, &id, with_mods);
        }
        "list-backups" => cmd_list_backups(&game_dir, json_output),
        "restore-backup" => {
            let archive = query.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the backup archive to restore.",
                    "Example: hot-rs-launcher restore-backup ./backups/1.21.4-20240101-120000.zip"
                )
            });
            cmd_restore_backup(
                &game_dir,
                std::path::Path::new(&archive),
                mc_version.as_deref(),
            );
        }
        "list-saves" => cmd_list_saves(&game_dir, mc_version.clone(), json_output),
        "backup-save" => {
            let folder = query.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the world folder to back up.",
                    "Example: hot-rs-launcher backup-save 'My World' -v 1.21.4"
                )
            });
            cmd_backup_save(&game_dir, mc_version.clone(), &folder, output.clone());
        }
        "import-save" => {
            let archive = query.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the world archive to import.",
                    "Example: hot-rs-launcher import-save ./MyWorld.zip -v 1.21.4"
                )
            });
            cmd_import_save(
                &game_dir,
                mc_version.clone(),
                std::path::Path::new(&archive),
                new_name.as_deref(),
            );
        }
        "delete-save" => {
            let folder = query.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the world folder to delete.",
                    "Example: hot-rs-launcher delete-save 'My World' -v 1.21.4 --yes"
                )
            });
            if !assume_yes {
                crate::die!(
                    "Refusing to delete a world without --yes.",
                    "Re-run with --yes after checking the folder name."
                );
            }
            cmd_delete_save(&game_dir, mc_version.clone(), &folder);
        }
        "list-resourcepacks" => cmd_list_resourcepacks(&game_dir, mc_version.clone(), json_output),
        "enable-resourcepack" | "disable-resourcepack" => {
            let file = query.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the resource pack file name.",
                    "Example: hot-rs-launcher enable-resourcepack MyPack.zip -v 1.21.4"
                )
            });
            cmd_set_resourcepack(
                &game_dir,
                mc_version.clone(),
                &file,
                action == "enable-resourcepack",
            );
        }
        "import-resourcepack" => {
            let source = query.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the resource pack zip or folder to import.",
                    "Example: hot-rs-launcher import-resourcepack ./MyPack.zip -v 1.21.4"
                )
            });
            cmd_import_resourcepack(&game_dir, mc_version.clone(), std::path::Path::new(&source));
        }
        "delete-resourcepack" => {
            let file = query.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the resource pack file name to delete.",
                    "Example: hot-rs-launcher delete-resourcepack MyPack.zip -v 1.21.4 --yes"
                )
            });
            if !assume_yes {
                crate::die!(
                    "Refusing to delete a resource pack without --yes.",
                    "Re-run with --yes after checking the file name."
                );
            }
            cmd_delete_resourcepack(&game_dir, mc_version.clone(), &file);
        }
        "list-shaders" => cmd_list_shaders(&game_dir, mc_version.clone(), json_output),
        "enable-shader" => {
            let file = query.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the shader pack file name.",
                    "Example: hot-rs-launcher enable-shader ComplementaryShaders.zip -v 1.21.4"
                )
            });
            cmd_set_shader(&game_dir, mc_version.clone(), Some(&file), true);
        }
        "disable-shader" => {
            cmd_set_shader(&game_dir, mc_version.clone(), None, false);
        }
        "import-shader" => {
            let source = query.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the shader pack zip or folder to import.",
                    "Example: hot-rs-launcher import-shader ./Complementary.zip -v 1.21.4"
                )
            });
            cmd_import_shader(&game_dir, mc_version.clone(), std::path::Path::new(&source));
        }
        "delete-shader" => {
            let file = query.clone().unwrap_or_else(|| {
                crate::die!(
                    "Provide the shader pack file name to delete.",
                    "Example: hot-rs-launcher delete-shader Complementary.zip -v 1.21.4 --yes"
                )
            });
            if !assume_yes {
                crate::die!(
                    "Refusing to delete a shader pack without --yes.",
                    "Re-run with --yes after checking the file name."
                );
            }
            cmd_delete_shader(&game_dir, mc_version.clone(), &file);
        }
        "play" => {
            let options = crate::launcher::LaunchOptions {
                version: mc_version.clone(),
                ram_mb,
                loader: loader.clone(),
                width,
                height,
                extra_jvm: jvm_args.clone(),
                gc,
                fullscreen,
                server: server.clone(),
                dry_run,
                diagnose,
            };
            cmd_play(&mut launcher, options);
        }
        "doctor" => cmd_doctor(&game_dir, json_output),
        "prune" => cmd_prune(&game_dir, json_output, assume_yes),
        "repair" => cmd_repair(&mut launcher, &game_dir, json_output, deep, assume_yes),
        "completions" => cmd_completions(query.as_deref()),
        "tui" => {
            let explicit_ram = matches.value_source("ram") == Some(ValueSource::CommandLine);
            let explicit_threads =
                matches.value_source("threads") == Some(ValueSource::CommandLine);
            let explicit_java = matches.value_source("java") == Some(ValueSource::CommandLine);
            let explicit_width = matches.value_source("width") == Some(ValueSource::CommandLine);
            let explicit_height = matches.value_source("height") == Some(ValueSource::CommandLine);
            let bootstrap = tui::app::Bootstrap {
                ram: explicit_ram.then_some(ram_mb),
                threads: explicit_threads.then_some(threads),
                java: explicit_java.then(|| java_path.clone().unwrap_or_default()),
                width: if explicit_width { width } else { None },
                height: if explicit_height { height } else { None },
                version: if matches.value_source("version") == Some(ValueSource::CommandLine) {
                    mc_version
                } else {
                    None
                },
                source: manual_mode,
                no_splash: false,
            };
            tui::run(launcher, bootstrap)
                .unwrap_or_else(|e| crate::die!(format!("TUI error: {e}")));
        }
        _ => {
            eprintln!("Unknown action: {}", action);
            std::process::exit(1);
        }
    }
}

fn resolve_loader(
    loader_arg: Option<String>,
    fabric: bool,
    forge: bool,
    neoforge: bool,
) -> Option<String> {
    if forge && neoforge {
        crate::die!("Cannot use both --forge and --neoforge.");
    }
    if let Some(l) = loader_arg {
        return Some(l);
    }
    if neoforge {
        return Some("neoforge".into());
    }
    if forge {
        return Some("forge".into());
    }
    if fabric {
        return Some("fabric".into());
    }
    None
}

// ─── command implementations ──────────────────────────────────────

fn cmd_logout(launcher: &mut MinecraftLauncher) {
    launcher.accounts.clear().unwrap_or_else(|e| crate::die!(e));
    println!("  Cleared all saved accounts.");
}

fn cmd_accounts(launcher: &MinecraftLauncher) {
    crate::log::header("Saved Accounts");
    let accs = launcher.accounts.accounts();
    if accs.as_object().is_none_or(|o| o.is_empty()) {
        crate::warn_msg!("No accounts saved.");
        crate::info!("Login:  hot-rs-launcher login");
        crate::info!("Offline: hot-rs-launcher offline <username>");
        return;
    }

    let default_key = launcher.accounts.default_key();
    for (key, acc) in accs.as_object().unwrap() {
        let is_default = if Some(key) == default_key.as_ref() {
            " (default)"
        } else {
            ""
        };
        let acc_type = acc["type"].as_str().unwrap_or("?");
        let username = acc["username"].as_str().unwrap_or("?");
        println!("  [{}] {}{}", acc_type, username, is_default);

        if acc_type == "msa" {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            let expires = acc["expires_at"].as_f64().unwrap_or(0.0);
            if now > expires {
                println!("         Session expired -- will auto-refresh on next launch");
            } else {
                let remaining = expires - now;
                let hours = (remaining / 3600.0) as u32;
                println!("         Session valid (~{}h remaining)", hours);
            }
        }
    }
}

fn cmd_list_versions(json: bool) {
    let versions = mr::list_game_versions();
    let mut arr: Vec<_> = versions.as_array().cloned().unwrap_or_default();
    arr.sort_by(|a, b| {
        b["date"]
            .as_str()
            .unwrap_or("")
            .cmp(a["date"].as_str().unwrap_or(""))
    });
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "total": arr.len(),
                "versions": arr,
            }))
            .unwrap_or_default()
        );
        return;
    }
    crate::log::header("Minecraft Versions (from Modrinth)");

    for v in &arr {
        let marker = if v["major"].as_bool().unwrap_or(false) {
            " *"
        } else {
            ""
        };
        println!(
            "  {: <12} {: <10} {}{}",
            v["version"].as_str().unwrap_or("?"),
            v["version_type"].as_str().unwrap_or("?"),
            v["date"]
                .as_str()
                .unwrap_or("?")
                .chars()
                .take(10)
                .collect::<String>(),
            marker
        );
    }
    crate::info!("Total: {} versions", arr.len());
}

fn cmd_list_loaders() {
    crate::log::header("Mod Loaders");
    let loaders = mr::list_loaders();
    println!("  Modrinth loaders:");
    for l in &loaders {
        println!("  - {}", l);
    }
    println!("  Built-in loaders:");
    for l in &["fabric", "forge", "neoforge"] {
        println!("  - {}", l);
    }
    crate::info!(
        "Total: {} loaders from Modrinth + built-in loaders",
        loaders.len()
    );
}

fn cmd_list_installed(game_dir: &std::path::Path, launcher: &MinecraftLauncher, json: bool) {
    let instances = InstanceManager::new(game_dir).list();
    // Read account metadata without touching the OS keychain: listing must not
    // trigger a keychain prompt or a slow D-Bus round trip.
    let account = launcher.accounts.default_key().and_then(|key| {
        launcher
            .accounts
            .accounts()
            .get(&key)
            .map(|value| (key, value))
    });
    if json {
        let account = account
            .as_ref()
            .map(|(key, value)| {
                serde_json::json!({
                    "key": key,
                    "username": value["username"],
                    "type": value["type"],
                })
            })
            .unwrap_or(serde_json::Value::Null);
        let value = serde_json::json!({
            "account": account,
            "total": instances.len(),
            "instances": instances.iter().map(|detail| serde_json::json!({
                "id": detail.id,
                "base": detail.base,
                "name": detail.name,
                "java_major": detail.java_major,
                "loaders": detail.loaders,
                "jar_size": detail.jar_size,
                "total_size": detail.total_size,
                "saves_size": detail.saves_size,
                "mods_enabled": detail.mods_enabled,
                "mods_disabled": detail.mods_disabled,
                "last_played": detail.last_played.and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map(|age| age.as_secs()),
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
        return;
    }
    crate::log::header("Locally Installed Minecraft Versions");
    if instances.is_empty() {
        crate::warn_msg!("No versions installed.");
        crate::info!("Download one: hot-rs-launcher download -v <version>");
        return;
    }

    for detail in &instances {
        let loaders = if detail.loaders.is_empty() {
            "vanilla".to_string()
        } else {
            detail.loaders.join("+")
        };
        let mut tags = vec![format!(
            "{:<8} {:<16}",
            detail
                .java_major
                .map(|major| format!("Java {major}"))
                .unwrap_or_else(|| "Java ?".into()),
            loaders
        )];
        let mods = detail.mods_enabled + detail.mods_disabled;
        if mods > 0 {
            tags.push(format!("{mods} mod(s)"));
        }
        if detail.base != detail.id {
            tags.push(format!("base {}", detail.base));
        }
        if let Some(name) = detail.name.as_deref() {
            if name != detail.id && name != detail.base {
                tags.push(format!("\"{name}\""));
            }
        }
        println!(
            "  {:<28} {:>9}  {}",
            detail.id,
            format_size(detail.total_size),
            tags.join("  ")
        );
    }

    println!("\n  Total: {} instance(s)", instances.len());
    if let Some((_, acc)) = &account {
        println!(
            "  Account: {} ({})",
            acc["username"].as_str().unwrap_or("?"),
            acc["type"].as_str().unwrap_or("?")
        );
    } else {
        println!("  No account saved. Run: hot-rs-launcher login");
    }
}

fn cmd_list_mods(game_dir: &std::path::Path, mc_version: Option<String>, json: bool) {
    let mc_version = resolve_mc_version_quiet(mc_version, game_dir, json);
    let mm = ModManager::new(game_dir);
    let mods = mm.list_mods(&mc_version);
    if json {
        let value = serde_json::json!({
            "instance": mc_version,
            "mods": mods.iter().map(|(name, enabled, size)| serde_json::json!({
                "file": name,
                "enabled": enabled,
                "size": size,
            })).collect::<Vec<_>>(),
            "enabled": mods.iter().filter(|(_, enabled, _)| *enabled).count(),
            "disabled": mods.iter().filter(|(_, enabled, _)| !*enabled).count(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
        return;
    }
    crate::log::header(&format!("Mods for {}", mc_version));
    if mods.is_empty() {
        crate::warn_msg!("No mods installed for {}.", mc_version);
        crate::info!(
            "Install: hot-rs-launcher install-mod <slug> -v {}",
            mc_version
        );
        return;
    }

    for (name, enabled, size) in &mods {
        let status = if *enabled { "[enabled] " } else { "[DISABLED]" };
        println!(
            "  {} {: <50} {:>8.1} KB",
            status,
            name,
            *size as f64 / 1024.0
        );
    }
    let enabled_count = mods.iter().filter(|(_, e, _)| *e).count();
    let disabled_count = mods.iter().filter(|(_, e, _)| !*e).count();
    crate::info!("{} enabled, {} disabled", enabled_count, disabled_count);
}

#[allow(clippy::too_many_arguments)]
fn cmd_search(
    game_dir: &std::path::Path,
    query: Option<String>,
    limit: u32,
    mc_version: Option<String>,
    loader: Option<&str>,
    page: u32,
    sort: String,
    category: Option<String>,
    browse: bool,
    json: bool,
) {
    cmd_search_kind(
        game_dir,
        crate::discover::DiscoverKind::Mods,
        query,
        limit,
        mc_version,
        loader,
        page,
        sort,
        category,
        browse,
        json,
    );
}

#[allow(clippy::too_many_arguments)]
fn cmd_search_kind(
    game_dir: &std::path::Path,
    kind: crate::discover::DiscoverKind,
    query: Option<String>,
    limit: u32,
    mc_version: Option<String>,
    loader: Option<&str>,
    page: u32,
    sort: String,
    category: Option<String>,
    browse: bool,
    json: bool,
) {
    let instance = mc_version.or_else(|| {
        ModManager::list_installed_versions(game_dir)
            .last()
            .cloned()
    });
    let query = query.unwrap_or_default();
    let query = query.trim().to_string();
    let browse = browse || query.is_empty();
    let sort = if browse { "downloads" } else { sort.as_str() };
    let mut value = crate::discover::search(
        kind,
        &query,
        sort,
        page.saturating_sub(1),
        limit,
        instance.as_deref(),
        loader,
        category.as_deref(),
    )
    .unwrap_or_else(|error| crate::die!(format!("Search failed: {error}")));
    if kind == crate::discover::DiscoverKind::Mods {
        if let Some(instance) = instance.as_deref() {
            crate::discover::mark_installed_mods(&mut value, game_dir, instance);
        }
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
        return;
    }
    let hits = value["hits"].as_array().cloned().unwrap_or_default();
    let total = value["total_hits"].as_u64().unwrap_or(hits.len() as u64);
    let mode = if browse { "Browse" } else { "Search" };
    let scope = instance
        .as_deref()
        .map(|instance| format!(" · Minecraft {instance}"))
        .unwrap_or_default();
    let loader_scope = loader
        .map(|loader| format!(" · {loader}"))
        .unwrap_or_default();
    crate::log::header(&format!(
        "{mode} {}{scope}{loader_scope}",
        kind.title(false)
    ));
    if hits.is_empty() {
        crate::warn_msg!("No results found.");
        return;
    }
    for (index, hit) in hits.iter().enumerate() {
        let title = hit["title"]
            .as_str()
            .or_else(|| hit["slug"].as_str())
            .unwrap_or("?");
        let slug = hit["slug"].as_str().unwrap_or("?");
        let author = hit["author"].as_str().unwrap_or("?");
        let downloads = hit["downloads"].as_u64().unwrap_or(0);
        let installed = hit["_installed"].as_bool().unwrap_or(false);
        let installed_version = hit["_installed_version"].as_str().unwrap_or("");
        let marker = if installed {
            if installed_version.is_empty() {
                "  [installed]".to_string()
            } else {
                format!("  [installed {installed_version}]")
            }
        } else {
            String::new()
        };
        println!("  [{:>2}.] {}{}", index + 1, title, marker);
        println!(
            "       slug: {slug}  ·  by {author}  ·  {} downloads",
            format_num(downloads)
        );
        let labels: Vec<&str> = hit["categories"]
            .as_array()
            .map(|values| values.iter().filter_map(|value| value.as_str()).collect())
            .unwrap_or_default();
        if kind == crate::discover::DiscoverKind::Shaders {
            let support = crate::content::ShaderSupport::from_labels(labels.iter().copied());
            println!("       shader loaders: {}", support.summary(false));
        }
        if !labels.is_empty() {
            println!("       categories: {}", labels.join(", "));
        }
        if let Some(description) = hit["description"].as_str() {
            if !description.is_empty() {
                println!(
                    "       {}",
                    description.chars().take(110).collect::<String>()
                );
            }
        }
        println!();
    }
    crate::info!("Page {} · {} result(s) · total {}", page, hits.len(), total);
    if (page as u64) * (limit as u64) < total {
        crate::info!("Next page: --page {}", page + 1);
    }
    match kind {
        crate::discover::DiscoverKind::Mods => crate::info!(
            "Install with: hot-rs-launcher install-mod <slug> -v {} --yes",
            instance.as_deref().unwrap_or("<version>")
        ),
        crate::discover::DiscoverKind::ResourcePacks => crate::info!(
            "Install with: hot-rs-launcher download-resourcepack <slug> -v {}",
            instance.as_deref().unwrap_or("<version>")
        ),
        crate::discover::DiscoverKind::Shaders => crate::info!(
            "Install with: hot-rs-launcher download-shader <slug> -v {}",
            instance.as_deref().unwrap_or("<version>")
        ),
    }
}

fn cmd_download_resourcepack(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    slug: &str,
    enable: bool,
) {
    let instance = resolve_mc_version(mc_version, game_dir);
    crate::log::header(&format!("Download resource pack · {}", instance));
    let report = crate::discover::install_resourcepack(game_dir, &instance, slug, enable)
        .unwrap_or_else(|error| crate::die!(format!("Install failed: {error}")));
    crate::success!(
        "Installed '{}' ({}) for Minecraft {}.",
        report.title,
        report.file,
        instance
    );
    if report.enabled {
        crate::info!("Enabled for the next launch.");
    } else {
        crate::info!(
            "Enable with: hot-rs-launcher enable-resourcepack '{}' -v {}",
            report.file,
            instance
        );
    }
}

fn cmd_download_shader(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    slug: &str,
    enable: bool,
    install_loader: bool,
) {
    let instance = resolve_mc_version(mc_version, game_dir);
    crate::log::header(&format!("Download shader pack · {}", instance));
    let report = crate::discover::install_shader(game_dir, &instance, slug, enable)
        .unwrap_or_else(|error| crate::die!(format!("Install failed: {error}")));
    crate::success!(
        "Installed '{}' ({}) for Minecraft {}.",
        report.title,
        report.file,
        instance
    );
    crate::info!("Shader loaders: {}", report.support.summary(false));

    let manager = ContentManager::new(game_dir);
    let status = manager.shader_loader_status(&instance);
    let instance_loader = InstanceManager::new(game_dir)
        .loaders(&instance)
        .first()
        .cloned();
    if report.support.iris && !status.iris {
        match instance_loader
            .as_deref()
            .and_then(crate::discover::shader_loader_mod)
        {
            Some((mod_slug, title)) => {
                if install_loader {
                    crate::info!("Installing {} for Iris support...", title);
                    match ModManager::new(game_dir).install_with(
                        mod_slug,
                        &instance,
                        instance_loader.as_deref(),
                        None,
                        DependencyMode::Auto,
                    ) {
                        Ok(_) => crate::success!("{} installed.", title),
                        Err(error) => {
                            crate::error_msg!("{} install failed: {}", title, error);
                        }
                    }
                } else {
                    crate::info!(
                        "Install {} with: hot-rs-launcher install-mod {} -v {} --loader {} --yes",
                        title,
                        mod_slug,
                        instance,
                        instance_loader.as_deref().unwrap_or("fabric")
                    );
                }
            }
            None => {
                crate::warn_msg!(
                    "This shader supports Iris, but '{}' has no Fabric/NeoForge/Forge loader. Install a loader first.",
                    instance
                );
            }
        }
    }
    if report.support.optifine && !status.optifine {
        crate::info!(
            "OptiFine is also supported; install OptiFine manually from https://optifine.net/downloads (the launcher does not run its installer)."
        );
    }
    if report.support.is_empty() {
        crate::warn_msg!("Shader loader support is unknown; check the project page.");
    }
    if report.enabled {
        crate::info!("Enabled for the next launch.");
    } else {
        crate::info!(
            "Enable with: hot-rs-launcher enable-shader '{}' -v {}",
            report.file,
            instance
        );
    }
}

fn cmd_list_shaders(game_dir: &std::path::Path, mc_version: Option<String>, json: bool) {
    let instance = resolve_mc_version_quiet(mc_version, game_dir, json);
    let manager = ContentManager::new(game_dir);
    let shaders = manager.list_shaders(&instance);
    let status = manager.shader_loader_status(&instance);
    if json {
        let value = serde_json::json!({
            "instance": instance,
            "loaders": {"iris": status.iris, "optifine": status.optifine},
            "shaders": shaders.iter().map(|shader| serde_json::json!({
                "file": shader.file,
                "enabled": shader.enabled,
                "is_dir": shader.is_dir,
                "size": shader.size,
                "title": shader.title,
                "project": shader.project,
                "iris": shader.support.map(|support| support.iris).unwrap_or(false),
                "optifine": shader.support.map(|support| support.optifine).unwrap_or(false),
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
        return;
    }
    crate::log::header(&format!("Shader packs · {}", instance));
    crate::info!("Shader loader: {}", status.summary(false));
    if shaders.is_empty() {
        crate::warn_msg!("No shader packs installed for '{}'.", instance);
        crate::info!(
            "Browse with: hot-rs-launcher search-shaders --browse -v {}",
            instance
        );
        return;
    }
    for shader in &shaders {
        let support = shader
            .support
            .map(|support| support.summary(false))
            .unwrap_or_else(|| "unknown".into());
        let title = shader.title.as_deref().unwrap_or(&shader.file);
        println!(
            "  {} {:<34} {:>9}  {:<20} {}",
            if shader.enabled { "[on] " } else { "[off]" },
            crate::tui::theme::truncate_chars(title, 34),
            crate::tui::theme::format_bytes(shader.size),
            support,
            if shader.is_dir { "folder" } else { "zip" }
        );
    }
    crate::info!(
        "Enable with: hot-rs-launcher enable-shader '<file>' -v {}",
        instance
    );
}

fn cmd_set_shader(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    file: Option<&str>,
    enabled: bool,
) {
    let instance = resolve_mc_version(mc_version, game_dir);
    let manager = ContentManager::new(game_dir);
    if enabled {
        let file = file.unwrap_or_else(|| crate::die!("Provide the shader pack file name."));
        manager
            .set_shader_enabled(&instance, file, true)
            .unwrap_or_else(|error| crate::die!(format!("Shader update failed: {error}")));
        crate::success!("Enabled shader pack '{}' for {}.", file, instance);
    } else {
        manager
            .disable_shader(&instance)
            .unwrap_or_else(|error| crate::die!(format!("Shader update failed: {error}")));
        crate::success!("Disabled shader packs for {}.", instance);
    }
}

fn cmd_import_shader(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    source: &std::path::Path,
) {
    let instance = resolve_mc_version(mc_version, game_dir);
    crate::log::header(&format!("Import shader pack · {}", instance));
    crate::info!("Source: {}", source.display());
    let manager = ContentManager::new(game_dir);
    let name = manager
        .import_shader(&instance, source)
        .unwrap_or_else(|error| crate::die!(format!("Import failed: {error}")));
    crate::success!("Imported shader pack '{}'.", name);
    crate::info!(
        "Enable it with: hot-rs-launcher enable-shader '{}' -v {}",
        name,
        instance
    );
}

fn cmd_delete_shader(game_dir: &std::path::Path, mc_version: Option<String>, file: &str) {
    let instance = resolve_mc_version(mc_version, game_dir);
    crate::log::header(&format!("Delete shader pack · {}", instance));
    let manager = ContentManager::new(game_dir);
    let freed = manager
        .delete_shader(&instance, file)
        .unwrap_or_else(|error| crate::die!(format!("Delete failed: {error}")));
    crate::success!(
        "Deleted shader pack '{}' ({} freed).",
        file,
        crate::tui::theme::format_bytes(freed)
    );
}

fn cmd_optimize_mods(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    loader: Option<String>,
    limit: u32,
    apply: bool,
    assume_yes: bool,
    json: bool,
) {
    let instance = resolve_mc_version(mc_version, game_dir);
    let loader = loader.or_else(|| {
        InstanceManager::new(game_dir)
            .loaders(&instance)
            .first()
            .cloned()
    });
    let Some(loader) = loader else {
        crate::die!(format!(
            "Install Fabric, Forge or NeoForge for '{}' before adding optimization mods.",
            instance
        ));
    };
    let hits =
        crate::discover::recommended_optimizations(game_dir, &instance, Some(&loader), limit)
            .unwrap_or_else(|error| crate::die!(format!("Optimization search failed: {error}")));
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&hits).unwrap_or_default()
        );
        return;
    }
    crate::log::header(&format!("Optimization Mods · {} · {}", instance, loader));
    if hits.is_empty() {
        crate::warn_msg!("No optimization mods found for this version and loader.");
        return;
    }
    for (index, hit) in hits.iter().enumerate() {
        let title = hit["title"]
            .as_str()
            .or_else(|| hit["slug"].as_str())
            .unwrap_or("?");
        let slug = hit["slug"].as_str().unwrap_or("?");
        let downloads = hit["downloads"].as_u64().unwrap_or(0);
        let installed = hit["_installed"].as_bool().unwrap_or(false);
        let version = hit["_installed_version"].as_str().unwrap_or("");
        let marker = if installed {
            if version.is_empty() {
                "  [installed]".to_string()
            } else {
                format!("  [installed {version}]")
            }
        } else {
            String::new()
        };
        println!(
            "  [{:>2}.] {:<34} {:<30} {:>12}{}",
            index + 1,
            title,
            slug,
            format_num(downloads),
            marker
        );
    }
    let missing = hits
        .iter()
        .filter(|hit| !hit["_installed"].as_bool().unwrap_or(false))
        .count();
    if apply {
        if !assume_yes {
            crate::die!(
                "Refusing to install optimization mods without --yes.",
                "Re-run with --yes after reviewing the list."
            );
        }
        let mut installed_count = 0usize;
        let mut failed = 0usize;
        for hit in hits
            .iter()
            .filter(|hit| !hit["_installed"].as_bool().unwrap_or(false))
        {
            let slug = hit["slug"].as_str().unwrap_or("");
            if slug.is_empty() {
                continue;
            }
            let title = hit["title"].as_str().unwrap_or(slug);
            match ModManager::new(game_dir).install_with(
                slug,
                &instance,
                Some(&loader),
                None,
                DependencyMode::Auto,
            ) {
                Ok(_) => {
                    installed_count += 1;
                    crate::success!("Installed {}", title);
                }
                Err(error) => {
                    failed += 1;
                    crate::error_msg!("{}: {}", title, error);
                }
            }
        }
        crate::success!(
            "{} optimization mod(s) installed, {} failed.",
            installed_count,
            failed
        );
    } else {
        crate::info!(
            "{} missing; install with: hot-rs-launcher optimize-mods -v {} --apply --yes",
            missing,
            instance
        );
    }
}

fn cmd_search_more(_game_dir: &std::path::Path, query: Option<String>) {
    let slug = query.unwrap_or_else(|| {
        crate::die!(
            "Please provide the exact mod slug.\n  Example: hot-rs-launcher search-more sodium"
        );
    });

    let project = mr::get_project(&slug);
    let project_id = project["id"].as_str().unwrap_or(&slug);
    let versions = mr::get_project_versions(project_id, None, None);
    let support = ModManager::summarize_loader_support(&versions);

    let title = project["title"].as_str().unwrap_or(&slug);
    crate::log::header(title);

    println!("  slug:        {}", project["slug"].as_str().unwrap_or("?"));
    println!("  project id:  {}", project["id"].as_str().unwrap_or("?"));
    println!(
        "  downloads:   {}",
        format_num(project["downloads"].as_u64().unwrap_or(0))
    );
    println!(
        "  followers:   {}",
        format_num(project["followers"].as_u64().unwrap_or(0))
    );
    println!(
        "  client/server: {} / {}",
        project["client_side"].as_str().unwrap_or("?"),
        project["server_side"].as_str().unwrap_or("?")
    );
    let lic = &project["license"];
    println!("  license:     {}", lic["id"].as_str().unwrap_or("?"));
    println!(
        "  categories:  {}",
        project["categories"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    );
    println!(
        "  updated:     {}",
        project["updated"]
            .as_str()
            .unwrap_or("?")
            .chars()
            .take(10)
            .collect::<String>()
    );
    if let Some(src) = project["source_url"].as_str() {
        println!("  source:      {}", src);
    }
    if let Some(issues) = project["issues_url"].as_str() {
        println!("  issues:      {}", issues);
    }
    if let Some(desc) = project["description"].as_str() {
        println!("\n  {}", desc);
    }

    println!("\n  Loader support (highest game version):");
    let shown_loaders = ["fabric", "forge", "neoforge"];
    for l in &shown_loaders {
        if let Some((mc, modver)) = support.get(*l) {
            println!(
                "    {: <10} <= MC {: <10} latest mod version: {}",
                l, mc, modver
            );
        } else {
            println!("    {: <10} not supported", l);
        }
    }
    for (l, (mc, modver)) in &support {
        if !shown_loaders.contains(&l.as_str()) {
            println!(
                "    {: <10} <= MC {: <10} latest mod version: {}",
                l, mc, modver
            );
        }
    }

    let mc_re = regex::Regex::new(r"^\d+\.\d+(\.\d+)?$").unwrap();
    let all_mc: std::collections::BTreeSet<String> = versions
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .flat_map(|v| {
            v["game_versions"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|gv| {
                    let s = gv.as_str()?;
                    if mc_re.is_match(s) {
                        Some(s.to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();

    if !all_mc.is_empty() {
        let mc_sorted: Vec<_> = all_mc.iter().collect();
        let shown: Vec<_> = mc_sorted.iter().rev().take(12).rev().collect();
        let prefix = if mc_sorted.len() > 12 { "..., " } else { "" };
        println!(
            "\n  Supported MC releases: {}{}",
            prefix,
            shown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let mut versions_arr = versions.as_array().cloned().unwrap_or_default();
    versions_arr.sort_by(|a, b| {
        b["date_published"]
            .as_str()
            .unwrap_or("")
            .cmp(a["date_published"].as_str().unwrap_or(""))
    });

    println!(
        "\n  Recent versions ({} of {}):",
        versions_arr.len().min(8),
        versions_arr.len()
    );
    for v in versions_arr.iter().take(8) {
        let vn = v["version_number"]
            .as_str()
            .unwrap_or(v["id"].as_str().unwrap_or("?"));
        let gv: Vec<&str> = v["game_versions"]
            .as_array()
            .map(|arr| {
                let all: Vec<&str> = arr.iter().filter_map(|x| x.as_str()).collect();
                let start = if all.len() > 4 { all.len() - 4 } else { 0 };
                all[start..].to_vec()
            })
            .unwrap_or_default();
        let gv_str = gv.join(", ");
        let ld: Vec<&str> = v["loaders"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|x| x.as_str()).collect())
            .unwrap_or_default();
        let ld_str = ld.join(", ");
        let date = v["date_published"]
            .as_str()
            .unwrap_or("?")
            .chars()
            .take(10)
            .collect::<String>();
        let vtype = v["version_type"].as_str().unwrap_or("?");
        println!(
            "    {: <36} {: <8} MC {: <24} [{}]  {}",
            vn, vtype, gv_str, ld_str, date
        );
    }
    println!(
        "\n  Install: hot-rs-launcher install-mod {} -v <version> --loader <loader>",
        project["slug"].as_str().unwrap_or(&slug)
    );
}

fn cmd_install_fabric(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    loader_version: Option<&str>,
) {
    let mc_version = resolve_mc_version_or_latest(mc_version, game_dir);
    crate::log::header("Install Fabric Loader");
    crate::info!("Target MC version: {}", mc_version);

    let fm = FabricManager::new(game_dir);
    let (all_jars, profile) = fm
        .install(&mc_version, loader_version)
        .unwrap_or_else(|e| crate::die!(format!("Fabric install failed: {}", e)));

    crate::success!("Fabric Loader installed successfully!");
    println!("    MC Version: {}", mc_version);
    println!("    Profile:    {}", profile["id"].as_str().unwrap_or("?"));
    println!(
        "    Libraries:  {} jars -> {}",
        all_jars.len(),
        game_dir.join("libraries").display()
    );
    println!(
        "\n  Launch with: hot-rs-launcher play -v {} --fabric",
        mc_version
    );
}

fn cmd_install_forge(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    loader_version: Option<&str>,
    java: Option<&str>,
) {
    let mc_version = resolve_mc_version_or_latest(mc_version, game_dir);
    crate::log::header("Install Forge Loader");
    crate::info!("Target MC version: {}", mc_version);

    let fm = ForgeManager::new(game_dir);
    let (installed_id, profile) = fm
        .install(&mc_version, loader_version, java)
        .unwrap_or_else(|e| crate::die!(format!("Forge install failed: {}", e)));

    crate::success!("Forge Loader installed successfully!");
    println!("    MC Version: {}", mc_version);
    println!("    Version ID: {}", installed_id);
    println!(
        "    Main Class: {}",
        profile["mainClass"].as_str().unwrap_or("?")
    );
    println!(
        "\n  Launch with: hot-rs-launcher play -v {} --forge",
        mc_version
    );
}

fn cmd_install_neoforge(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    loader_version: Option<&str>,
    java: Option<&str>,
) {
    let mc_version = resolve_mc_version_or_latest(mc_version, game_dir);
    crate::log::header("Install NeoForge Loader");
    crate::info!("Target MC version: {}", mc_version);

    let nm = NeoForgeManager::new(game_dir);
    let (installed_id, profile) = nm
        .install(&mc_version, loader_version, java)
        .unwrap_or_else(|e| crate::die!(format!("NeoForge install failed: {}", e)));

    crate::success!("NeoForge Loader installed successfully!");
    println!("    MC Version: {}", mc_version);
    println!("    Version ID: {}", installed_id);
    println!(
        "    Main Class: {}",
        profile["mainClass"].as_str().unwrap_or("?")
    );
    println!(
        "\n  Launch with: hot-rs-launcher play -v {} --neoforge",
        mc_version
    );
}

fn cmd_install_mod(
    game_dir: &std::path::Path,
    query: Option<String>,
    mc_version: Option<String>,
    loader: Option<&str>,
    mod_version: Option<&str>,
    dependencies: DependencyMode,
) {
    let slug = query.unwrap_or_else(|| {
        crate::die!(
            "Please provide a mod slug.\n  Example: hot-rs-launcher install-mod sodium -v 1.21.4"
        );
    });
    let mc_version = resolve_mc_version(mc_version, game_dir);

    crate::log::header(&format!("Install Mod for MC {}", mc_version));

    let mm = ModManager::new(game_dir);
    let (_paths, _version_data, project) = mm
        .install_with(&slug, &mc_version, loader, mod_version, dependencies)
        .unwrap_or_else(|error| crate::die!(error));

    let title = project["title"].as_str().unwrap_or(&slug);
    let source = project["source"].as_str().unwrap_or("modrinth");
    crate::success!(
        "{} installed for Minecraft {} (source: {})!",
        title,
        mc_version,
        source
    );

    // Check which loader is installed for launch hint
    for l in &["fabric", "neoforge", "forge"] {
        if game_dir
            .join("libraries")
            .join(l)
            .join(format!("{}-profile-{}.json", l, mc_version))
            .exists()
        {
            println!(
                "  Launch with: hot-rs-launcher play -v {} --{}",
                mc_version, l
            );
            return;
        }
    }
    println!(
        "  Install a loader first, e.g.: hot-rs-launcher install-fabric -v {}",
        mc_version
    );
    println!(
        "  Then launch: hot-rs-launcher play -v {} --fabric",
        mc_version
    );
}

fn cmd_disable_mod(game_dir: &std::path::Path, query: Option<String>, mc_version: Option<String>) {
    let slug = query.unwrap_or_else(|| crate::die!("Provide a mod slug/name to disable."));
    let mc_version = resolve_mc_version(mc_version, game_dir);
    let mm = ModManager::new(game_dir);
    if mm.disable_mod(&slug, &mc_version) {
        crate::success!("Disabled '{}' for Minecraft {}", slug, mc_version);
    } else {
        crate::die!(format!(
            "No mod matching '{}' found for {}",
            slug, mc_version
        ));
    }
}

fn cmd_enable_mod(game_dir: &std::path::Path, query: Option<String>, mc_version: Option<String>) {
    let slug = query.unwrap_or_else(|| crate::die!("Provide a mod slug/name to enable."));
    let mc_version = resolve_mc_version(mc_version, game_dir);
    let mm = ModManager::new(game_dir);
    if mm.enable_mod(&slug, &mc_version) {
        crate::success!("Enabled '{}' for Minecraft {}", slug, mc_version);
    } else {
        crate::die!(format!(
            "No disabled mod matching '{}' found for {}",
            slug, mc_version
        ));
    }
}

fn cmd_uninstall_mod(
    game_dir: &std::path::Path,
    query: Option<String>,
    mc_version: Option<String>,
) {
    let slug = query.unwrap_or_else(|| crate::die!("Provide a mod slug/name to uninstall."));
    let mc_version = resolve_mc_version(mc_version, game_dir);
    let mm = ModManager::new(game_dir);
    let deleted = mm.uninstall_mod(&slug, &mc_version);
    if !deleted.is_empty() {
        for d in &deleted {
            crate::success!("Deleted: {}", d);
        }
    } else {
        crate::die!(format!(
            "No mod matching '{}' found for {}",
            slug, mc_version
        ));
    }
}

fn cmd_login(launcher: &mut MinecraftLauncher, device_code: bool) {
    let mut auth = MicrosoftAuth::new();

    if device_code {
        auth.device_code_login().unwrap_or_else(|e| {
            crate::die!(format!("Login failed: {}", e));
        });
    } else {
        crate::log::header("Microsoft Login (Browser)");
        crate::info!("A browser window will open. Log in with your Microsoft account.");
        crate::info!("Make sure your Microsoft account owns Minecraft!\n");
        crate::info!("Tip: use --device-code for device code login.\n");
        auth.login().unwrap_or_else(|e| {
            crate::die!(format!("Login failed: {}", e));
        });
    }

    let uid = crate::util::format_uuid(&auth.uuid);
    launcher
        .accounts
        .set_msa(
            &auth.username,
            &uid,
            &auth.mc_token,
            &auth.refresh_token,
            auth.expires_at,
        )
        .unwrap_or_else(|e| crate::die!(e));

    crate::success!("Logged in as: {}", auth.username);
    crate::info!("Credentials saved. Next steps:");
    crate::info!("  hot-rs-launcher download -v <version>   # download a Minecraft version");
    crate::info!("  hot-rs-launcher play -v <version>       # launch the game");
}

fn cmd_offline(launcher: &mut MinecraftLauncher, username: &str) {
    crate::log::header(&format!("Offline Mode: {}", username));
    launcher
        .accounts
        .set_offline(username)
        .unwrap_or_else(|e| crate::die!(e));
    crate::success!("Offline account saved: {}", username);
    crate::info!("Next steps:");
    crate::info!("  hot-rs-launcher download -v <version>   # download a Minecraft version");
    crate::info!("  hot-rs-launcher play -v <version>       # launch the game");
}

fn cmd_download(launcher: &mut MinecraftLauncher, mc_version: Option<&str>, no_assets: bool) {
    crate::log::header("Download Only");
    launcher
        .download_version(mc_version, no_assets)
        .unwrap_or_else(|e| {
            crate::die!(format!("Download failed: {}", e));
        });
}

fn content_age(modified: Option<SystemTime>) -> String {
    modified
        .and_then(|time| SystemTime::now().duration_since(time).ok())
        .map(crate::tui::theme::format_duration)
        .unwrap_or_else(|| "-".into())
}

fn cmd_list_saves(game_dir: &std::path::Path, mc_version: Option<String>, json: bool) {
    let instance = resolve_mc_version_quiet(mc_version, game_dir, json);
    let saves = ContentManager::new(game_dir).list_saves(&instance);
    if json {
        let value = serde_json::json!({
            "instance": instance,
            "saves": saves.iter().map(|save| serde_json::json!({
                "folder": save.folder,
                "name": save.name,
                "version": save.version,
                "game_type": save.game_type,
                "game_mode": save.game_type.map(|code| game_mode_label(code, false)),
                "hardcore": save.hardcore,
                "size": save.size,
                "modified": save.modified_unix(),
                "has_icon": save.has_icon,
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
        return;
    }
    crate::log::header(&format!("Saves · {}", instance));
    if saves.is_empty() {
        crate::warn_msg!("No worlds found for '{}'.", instance);
        crate::info!(
            "Worlds live in versions/{}/saves/. Start the game once to create one.",
            instance
        );
        return;
    }
    for save in &saves {
        let label = if save.folder != save.name {
            format!("{} ({})", save.name, save.folder)
        } else {
            save.name.clone()
        };
        let mode = save
            .game_type
            .map(|code| game_mode_label(code, false))
            .unwrap_or("unknown");
        let version = save.version.as_deref().unwrap_or("-");
        let hardcore = if save.hardcore { "  [hardcore]" } else { "" };
        println!(
            "  {:<40} {:<10} {:<10} {:>9}  {}{}",
            crate::tui::theme::truncate_chars(&label, 40),
            version,
            mode,
            crate::tui::theme::format_bytes(save.size),
            content_age(save.modified),
            hardcore
        );
    }
    crate::info!("{} world(s).", saves.len());
    crate::info!(
        "Back up with: hot-rs-launcher backup-save '<folder>' -v {}",
        instance
    );
}

fn cmd_backup_save(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    folder: &str,
    output: Option<PathBuf>,
) {
    let instance = resolve_mc_version(mc_version, game_dir);
    crate::log::header(&format!("Backup world · {}", instance));
    let manager = ContentManager::new(game_dir);
    let path = manager
        .backup_save(&instance, folder, output.as_deref())
        .unwrap_or_else(|error| crate::die!(format!("Backup failed: {error}")));
    let size = path.metadata().map(|meta| meta.len()).unwrap_or(0);
    crate::success!(
        "World backup: {} ({})",
        path.display(),
        crate::tui::theme::format_bytes(size)
    );
    crate::info!(
        "Import with: hot-rs-launcher import-save '{}' -v {}",
        path.display(),
        instance
    );
}

fn cmd_import_save(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    archive: &std::path::Path,
    name: Option<&str>,
) {
    let instance = resolve_mc_version(mc_version, game_dir);
    crate::log::header(&format!("Import world · {}", instance));
    crate::info!("Archive: {}", archive.display());
    let manager = ContentManager::new(game_dir);
    let folder = manager
        .import_save(&instance, archive, name)
        .unwrap_or_else(|error| crate::die!(format!("Import failed: {error}")));
    crate::success!("Imported world as '{}'.", folder);
    crate::info!(
        "Back it up again with: hot-rs-launcher backup-save '{}' -v {}",
        folder,
        instance
    );
}

fn cmd_delete_save(game_dir: &std::path::Path, mc_version: Option<String>, folder: &str) {
    let instance = resolve_mc_version(mc_version, game_dir);
    crate::log::header(&format!("Delete world · {}", instance));
    let manager = ContentManager::new(game_dir);
    let freed = manager
        .delete_save(&instance, folder)
        .unwrap_or_else(|error| crate::die!(format!("Delete failed: {error}")));
    crate::success!(
        "Deleted world '{}' ({} freed).",
        folder,
        crate::tui::theme::format_bytes(freed)
    );
}

fn cmd_list_resourcepacks(game_dir: &std::path::Path, mc_version: Option<String>, json: bool) {
    let instance = resolve_mc_version_quiet(mc_version, game_dir, json);
    let packs = ContentManager::new(game_dir).list_resourcepacks(&instance);
    if json {
        let value = serde_json::json!({
            "instance": instance,
            "resourcepacks": packs.iter().map(|pack| serde_json::json!({
                "file": pack.file,
                "enabled": pack.enabled,
                "incompatible": pack.incompatible,
                "is_dir": pack.is_dir,
                "size": pack.size,
                "pack_format": pack.pack_format,
                "description": pack.description,
                "valid": pack.valid,
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
        return;
    }
    crate::log::header(&format!("Resource packs · {}", instance));
    if packs.is_empty() {
        crate::warn_msg!("No resource packs installed for '{}'.", instance);
        crate::info!(
            "Import one with: hot-rs-launcher import-resourcepack ./pack.zip -v {}",
            instance
        );
        return;
    }
    for pack in &packs {
        let status = if !pack.valid {
            "[bad]"
        } else if pack.enabled {
            "[on] "
        } else {
            "[off]"
        };
        let format = pack
            .pack_format
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".into());
        println!(
            "  {} {:<36} {:>9}  fmt {:<4} {}",
            status,
            crate::tui::theme::truncate_chars(&pack.file, 36),
            crate::tui::theme::format_bytes(pack.size),
            format,
            crate::tui::theme::one_line(&pack.description, 60)
        );
    }
    crate::info!("{} pack(s).", packs.len());
    crate::info!(
        "Toggle with: hot-rs-launcher enable-resourcepack '<file>' -v {}",
        instance
    );
}

fn cmd_set_resourcepack(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    file: &str,
    enabled: bool,
) {
    let instance = resolve_mc_version(mc_version, game_dir);
    let manager = ContentManager::new(game_dir);
    let changed = manager
        .set_resourcepack_enabled(&instance, file, enabled)
        .unwrap_or_else(|error| crate::die!(format!("Resource pack update failed: {error}")));
    let state = if enabled { "Enabled" } else { "Disabled" };
    if changed {
        crate::success!("{state} '{}' for {}.", file, instance);
    } else {
        crate::info!("'{}' is already disabled for {}.", file, instance);
    }
    if let Some(pack) = manager
        .list_resourcepacks(&instance)
        .into_iter()
        .find(|pack| pack.file == file)
    {
        if !pack.valid {
            crate::warn_msg!("This pack has no readable pack.mcmeta; Minecraft may ignore it.");
        }
        if pack.incompatible {
            crate::info!("Minecraft marks this pack as incompatible with the current version.");
        }
    }
}

fn cmd_import_resourcepack(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    source: &std::path::Path,
) {
    let instance = resolve_mc_version(mc_version, game_dir);
    crate::log::header(&format!("Import resource pack · {}", instance));
    crate::info!("Source: {}", source.display());
    let manager = ContentManager::new(game_dir);
    let name = manager
        .import_resourcepack(&instance, source)
        .unwrap_or_else(|error| crate::die!(format!("Import failed: {error}")));
    crate::success!("Imported resource pack '{}'.", name);
    crate::info!(
        "Enable it with: hot-rs-launcher enable-resourcepack '{}' -v {}",
        name,
        instance
    );
}

fn cmd_delete_resourcepack(game_dir: &std::path::Path, mc_version: Option<String>, file: &str) {
    let instance = resolve_mc_version(mc_version, game_dir);
    crate::log::header(&format!("Delete resource pack · {}", instance));
    let manager = ContentManager::new(game_dir);
    let freed = manager
        .delete_resourcepack(&instance, file)
        .unwrap_or_else(|error| crate::die!(format!("Delete failed: {error}")));
    crate::success!(
        "Deleted resource pack '{}' ({} freed).",
        file,
        crate::tui::theme::format_bytes(freed)
    );
}

fn cmd_play(launcher: &mut MinecraftLauncher, options: crate::launcher::LaunchOptions) {
    let account = launcher.accounts.get_default();
    if account.is_none() && !options.dry_run {
        crate::die!(
            "No saved account. Run 'login' or 'offline <name>' first.",
            "  hot-rs-launcher login        # Microsoft login\n  hot-rs-launcher offline Steve # Offline mode"
        );
    }
    let exit_code = launcher.launch_with(options, account).unwrap_or_else(|e| {
        crate::die!(format!("Launch failed: {}", e));
    });
    std::process::exit(exit_code);
}

/// Id for instance commands: positional, then -v.
fn instance_arg(query: &Option<String>, mc_version: &Option<String>) -> String {
    query
        .clone()
        .or_else(|| mc_version.clone())
        .unwrap_or_else(|| {
            crate::die!(
                "Specify the instance id.",
                "Example: hot-rs-launcher remove-version -v 1.20.1 --keep-saves"
            )
        })
}

fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB)
    } else if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Keep the TUI's active instance pointing at something that exists.
fn sync_active_version(game_dir: &std::path::Path, from: &str, to: Option<&str>) {
    let path = crate::tui::config::Settings::path(game_dir);
    if !path.exists() {
        return;
    }
    let (mut settings, _) = crate::tui::config::Settings::load(&path);
    if settings.active_version.as_deref() == Some(from) {
        settings.active_version = to.map(|value| value.to_string());
        if let Err(error) = settings.save(&path) {
            crate::warn_msg!("{}", error);
        }
    }
}

fn cmd_list_java(game_dir: &std::path::Path, mc_version: Option<String>, json: bool) {
    let required = mc_version.as_ref().and_then(|id| {
        std::fs::read(
            game_dir
                .join("versions")
                .join(id)
                .join(format!("{id}.json")),
        )
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value["javaVersion"]["majorVersion"].as_u64())
        .map(|major| major as u32)
    });
    let infos = java::detect_java(Some(game_dir));
    let recommended = required
        .and_then(|major| java::best_java(&infos, Some(major)).map(|info| info.path.clone()));
    if json {
        let value = serde_json::json!({
            "required": required,
            "recommended": recommended,
            "javas": infos.iter().map(|info| serde_json::json!({
                "path": info.path,
                "major": info.major,
                "version": info.version,
                "vendor": info.vendor,
                "arch": info.arch,
                "origin": info.origin.label(),
                "recommended": Some(&info.path) == recommended.as_ref(),
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
        return;
    }
    crate::log::header("Detected Java Runtimes");
    if infos.is_empty() {
        crate::warn_msg!("No Java runtime found.");
        crate::info!("Install Mojang's runtime: hot-rs-launcher install-java --java-major 21");
        return;
    }
    // Only mark a recommendation when a version requirement is known; without
    // one the newest runtime is not necessarily the right choice.
    for info in &infos {
        let marker = if Some(&info.path) == recommended.as_ref() {
            " *"
        } else {
            ""
        };
        println!(
            "  {:<26} {:<32} {}{}",
            info.label(),
            info.meta(),
            info.path,
            marker
        );
    }
    if let Some(required) = required {
        crate::info!(
            "{} runtime(s); * recommended for Java {}.",
            infos.len(),
            required
        );
    } else {
        crate::info!(
            "{} runtime(s). Use --java <path> to pick one; pass -v <id> for a recommendation.",
            infos.len()
        );
    }
}

fn cmd_install_java(game_dir: &std::path::Path, major: Option<u32>, threads: usize) {
    let major = major.unwrap_or_else(|| {
        crate::die!(
            "Specify the Java major version.",
            "Example: hot-rs-launcher install-java --java-major 21"
        )
    });
    crate::log::header(&format!("Install Java {}", major));
    let path = java::download_runtime_for_major(game_dir, major, threads).unwrap_or_else(|| {
        crate::die!(format!(
            "Could not install Java {} (no Mojang runtime for this platform/version).",
            major
        ))
    });
    crate::success!("Java {} installed: {}", major, path);
    crate::info!("Use it with: hot-rs-launcher play --java {}", path);
}

fn cmd_remove_version(game_dir: &std::path::Path, id: &str, keep_saves: bool, keep_mods: bool) {
    let manager = InstanceManager::new(game_dir);
    crate::log::header(&format!("Remove Instance {}", id));
    let report = manager
        .remove(id, keep_saves, keep_mods)
        .unwrap_or_else(|e| crate::die!(format!("Remove failed: {e}")));
    for entry in &report.removed {
        println!("  removed: {}", entry);
    }
    for entry in &report.kept {
        println!("  kept:    {}", entry);
    }
    crate::success!("Freed {}.", format_size(report.freed_bytes));
    sync_active_version(game_dir, id, None);
}

fn cmd_rename_version(game_dir: &std::path::Path, from: &str, to: &str) {
    let manager = InstanceManager::new(game_dir);
    crate::log::header(&format!("Rename Instance {} -> {}", from, to));
    manager
        .rename(from, to)
        .unwrap_or_else(|e| crate::die!(format!("Rename failed: {e}")));
    crate::success!("Renamed '{}' to '{}'.", from, to);
    sync_active_version(game_dir, from, Some(to));
    crate::info!("Launch with: hot-rs-launcher play -v {}", to);
}

fn cmd_copy_version(game_dir: &std::path::Path, from: &str, to: &str) {
    let manager = InstanceManager::new(game_dir);
    crate::log::header(&format!("Copy Instance {} -> {}", from, to));
    let bytes = manager
        .copy(from, to)
        .unwrap_or_else(|e| crate::die!(format!("Copy failed: {e}")));
    crate::success!("Copied '{}' to '{}' ({}).", from, to, format_size(bytes));
    crate::info!("Launch with: hot-rs-launcher play -v {}", to);
}

fn cmd_export_modpack(
    game_dir: &std::path::Path,
    id: &str,
    loader: Option<&str>,
    output: Option<PathBuf>,
) {
    let manager = InstanceManager::new(game_dir);
    let output = output.unwrap_or_else(|| {
        game_dir.join("modpacks").join(format!(
            "{}.mrpack",
            crate::instance::sanitize_instance_id(id)
        ))
    });
    crate::log::header(&format!("Export Modpack from {}", id));
    let report = manager
        .export_modpack(id, loader, &output)
        .unwrap_or_else(|e| crate::die!(format!("Export failed: {e}")));
    crate::success!("Modpack: {}", report.path.display());
    println!(
        "  Downloads: {}   Overrides: {}",
        report.files, report.overrides
    );
    if report.offline {
        crate::warn_msg!("Modrinth was unreachable; all mods were bundled as overrides.");
    }
    crate::info!(
        "Import elsewhere with: hot-rs-launcher import-modpack '{}'",
        report.path.display()
    );
}

fn cmd_import_modpack(
    game_dir: &std::path::Path,
    archive: &std::path::Path,
    name: Option<&str>,
    install_loader: bool,
    java: Option<&str>,
) {
    let manager = InstanceManager::new(game_dir);
    crate::log::header("Import Modpack");
    crate::info!("Archive: {}", archive.display());
    let report = manager
        .import_modpack(archive, name)
        .unwrap_or_else(|e| crate::die!(format!("Import failed: {e}")));
    crate::success!(
        "Imported '{}' (Minecraft {}{})",
        report.id,
        report.base,
        report
            .loader
            .as_deref()
            .map(|loader| format!(" + {loader}"))
            .unwrap_or_default()
    );
    println!("  Downloaded: {} file(s)", report.files);
    println!("  Overrides:  {} file(s)", report.overrides);

    if let Some(loader) = report.loader.as_deref() {
        let installed =
            crate::instance::loader_profile_path(game_dir, loader, &report.id).is_some();
        if installed {
            crate::info!("{} is already installed for this instance.", loader);
        } else if install_loader {
            crate::info!(
                "Installing {} {}...",
                loader,
                report.loader_version.as_deref().unwrap_or("")
            );
            let result = match loader {
                "fabric" => FabricManager::new(game_dir)
                    .install(&report.base, report.loader_version.as_deref())
                    .map(|(jars, _)| format!("{} libraries", jars.len())),
                "forge" => ForgeManager::new(game_dir)
                    .install(&report.base, report.loader_version.as_deref(), java)
                    .map(|(id, _)| id),
                "neoforge" => NeoForgeManager::new(game_dir)
                    .install(&report.base, report.loader_version.as_deref(), java)
                    .map(|(id, _)| id),
                other => Err(crate::error::AppError::Loader(format!(
                    "Unsupported loader {other}"
                ))),
            };
            match result {
                Ok(summary) => crate::success!("{} installed ({}).", loader, summary),
                Err(error) => {
                    crate::error_msg!("Loader install failed: {error}");
                }
            }
        } else {
            let version_flag = report
                .loader_version
                .as_deref()
                .map(|version| format!(" --loader-version {version}"))
                .unwrap_or_default();
            crate::info!(
                "Install the loader with: hot-rs-launcher install-{} -v {}{}",
                loader,
                report.base,
                version_flag
            );
        }
    }
    crate::info!("Launch with: hot-rs-launcher play -v {}", report.id);
}

fn cmd_update_mods(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    loader: Option<String>,
    apply: bool,
    include_disabled: bool,
) {
    let mc_version = resolve_mc_version(mc_version, game_dir);
    let loader = loader.or_else(|| {
        InstanceManager::new(game_dir)
            .loaders(&mc_version)
            .first()
            .cloned()
    });
    crate::log::header(&format!(
        "Mod Updates for {} ({})",
        mc_version,
        loader.as_deref().unwrap_or("auto")
    ));
    let check = crate::update::check(game_dir, &mc_version, loader.as_deref(), include_disabled)
        .unwrap_or_else(|e| crate::die!(format!("Update check failed: {e}")));

    if check.updates.is_empty() {
        crate::success!(
            "All {} known mods are up to date ({} not on Modrinth).",
            check.up_to_date,
            check.unknown
        );
        return;
    }
    for entry in &check.updates {
        println!(
            "  {:<34} {:<14} -> {:<14} {}",
            entry.project, entry.current, entry.latest, entry.filename
        );
    }
    crate::info!(
        "{} update(s); {} up to date; {} not identified by Modrinth.",
        check.updates.len(),
        check.up_to_date,
        check.unknown
    );

    if apply {
        crate::log::header("Applying Updates");
        for entry in &check.updates {
            match crate::update::apply(game_dir, &mc_version, entry) {
                Ok(path) => crate::success!(
                    "{} -> {}",
                    entry.project,
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
                Err(error) => {
                    crate::error_msg!("Failed to update {}: {}", entry.project, error);
                }
            }
        }
    } else {
        crate::info!(
            "Apply with: hot-rs-launcher update-mods -v {} --apply",
            mc_version
        );
    }
}

fn cmd_diagnose(game_dir: &std::path::Path, id: Option<String>, json: bool) {
    let id = id.unwrap_or_else(|| {
        let installed = ModManager::list_installed_versions(game_dir);
        installed.last().cloned().unwrap_or_else(|| {
            crate::die!(
                "Specify an instance with -v <id>.",
                "Example: hot-rs-launcher diagnose -v 1.21.4"
            )
        })
    });
    let diagnosis = crate::diagnose::analyze_instance(game_dir, &id);
    if json {
        let value = match &diagnosis {
            Some(diagnosis) => serde_json::json!({
                "instance": id,
                "source": diagnosis.source,
                "findings": diagnosis.findings.iter().map(|finding| serde_json::json!({
                    "severity": match finding.severity {
                        crate::diagnose::Severity::Info => "info",
                        crate::diagnose::Severity::Warn => "warn",
                        crate::diagnose::Severity::Error => "error",
                    },
                    "title": finding.title,
                    "title_en": finding.title_en,
                    "detail": finding.detail,
                    "hint": finding.hint,
                    "hint_en": finding.hint_en,
                })).collect::<Vec<_>>(),
                "excerpt": diagnosis.excerpt,
            }),
            None => serde_json::json!({
                "instance": id,
                "source": null,
                "findings": [],
                "excerpt": [],
            }),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
        return;
    }
    crate::log::header(&format!("Diagnose {}", id));
    match diagnosis {
        Some(diagnosis) => print!("{}", crate::diagnose::render(&diagnosis)),
        None => {
            crate::warn_msg!("No crash report or logs/latest.log found for '{}'.", id);
            crate::info!("Launch the instance once, then run diagnose again.");
        }
    }
}

fn cmd_doctor(game_dir: &std::path::Path, json: bool) {
    let report = maintenance::doctor(game_dir);
    if json {
        let value = serde_json::json!({
            "ok": report.failures() == 0,
            "failures": report.failures(),
            "warnings": report.warnings(),
            "checks": report.checks.iter().map(|check| serde_json::json!({
                "name": check.name,
                "status": check.status.as_str(),
                "detail": check.detail,
                "hint": check.hint,
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
        if report.failures() > 0 {
            std::process::exit(1);
        }
        return;
    }
    crate::log::header("Environment Check");
    for check in &report.checks {
        let color = match check.status {
            maintenance::Status::Ok => "\x1b[92m",
            maintenance::Status::Warn => "\x1b[93m",
            maintenance::Status::Fail => "\x1b[91m",
            maintenance::Status::Skip => "\x1b[90m",
        };
        println!(
            "  {} {:<22} {}",
            crate::log::clr(color, &format!("[{}]", check.status.label())),
            check.name,
            check.detail
        );
        if let Some(hint) = &check.hint {
            println!("       {} {}", crate::log::clr("\x1b[90m", "→"), hint);
        }
    }
    println!();
    if report.failures() > 0 {
        crate::warn_msg!(
            "{} failure(s), {} warning(s).",
            report.failures(),
            report.warnings()
        );
        std::process::exit(1);
    }
    crate::success!(
        "{} check(s) passed, {} warning(s).",
        report.checks.len(),
        report.warnings()
    );
}

fn cmd_prune(game_dir: &std::path::Path, json: bool, apply: bool) {
    let plan = maintenance::plan_prune(game_dir)
        .unwrap_or_else(|e| crate::die!(format!("Prune scan failed: {e}")));
    if !plan.unreadable.is_empty() {
        crate::die!(format!(
            "Refusing to prune: {} instance(s) have unreadable metadata ({}). \
             Repair them first; deleting libraries they might use could break launching.",
            plan.unreadable.len(),
            plan.unreadable.join(", ")
        ));
    }
    let outcome = if apply {
        Some(
            maintenance::apply_prune(&plan)
                .unwrap_or_else(|e| crate::die!(format!("Prune failed: {e}"))),
        )
    } else {
        None
    };
    if json {
        let value = serde_json::json!({
            "applied": apply,
            "reclaimable": plan.bytes,
            "libraries": plan.libraries,
            "assets": plan.assets,
            "deleted": outcome.map(|(deleted, _)| deleted),
            "freed": outcome.map(|(_, bytes)| bytes),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
        return;
    }
    crate::log::header("Prune Unused Cache");
    if plan.is_empty() {
        crate::success!("Nothing to prune: every cached library and asset is referenced.");
        return;
    }
    println!("  Libraries:   {} file(s)", plan.libraries.len());
    println!("  Assets:      {} file(s)", plan.assets.len());
    println!("  Reclaimable: {}", format_size(plan.bytes));
    if let Some((deleted, bytes)) = outcome {
        crate::success!("Deleted {} file(s), freed {}.", deleted, format_size(bytes));
    } else {
        crate::info!("Dry run: nothing was deleted. Re-run with --yes to clean up.");
    }
}

fn cmd_repair(
    launcher: &mut MinecraftLauncher,
    game_dir: &std::path::Path,
    json: bool,
    deep: bool,
    apply: bool,
) {
    let health = maintenance::inspect_instances(game_dir, deep);
    let broken: Vec<&maintenance::InstanceHealth> =
        health.iter().filter(|item| !item.healthy()).collect();
    let mut repaired = Vec::new();
    let mut failed = Vec::new();
    if apply {
        for item in &broken {
            crate::info!("Repairing {}...", item.id);
            match launcher.download_version(Some(&item.id), false) {
                Ok(_) => repaired.push(item.id.clone()),
                Err(error) => failed.push(format!("{}: {error}", item.id)),
            }
        }
        crate::verify::flush();
    }
    if json {
        let value = serde_json::json!({
            "checked": health.len(),
            "apply": apply,
            "instances": health.iter().map(|item| serde_json::json!({
                "id": item.id,
                "healthy": item.healthy(),
                "issues": item.issues,
            })).collect::<Vec<_>>(),
            "repaired": repaired,
            "failed": failed,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
        if !failed.is_empty() {
            std::process::exit(1);
        }
        return;
    }
    crate::log::header("Repair Instances");
    if health.is_empty() {
        crate::warn_msg!("No instances installed; nothing to check.");
        return;
    }
    for item in &health {
        if item.healthy() {
            println!("  [OK]   {}", item.id);
        } else {
            println!("  [FAIL] {}", item.id);
            for issue in &item.issues {
                println!("         - {issue}");
            }
        }
    }
    if broken.is_empty() {
        crate::success!("All {} instance(s) are complete.", health.len());
        return;
    }
    if !apply {
        crate::info!(
            "Dry run: {} instance(s) need repair. Re-run with --yes to download missing files.",
            broken.len()
        );
        return;
    }
    if failed.is_empty() {
        crate::success!("Repaired {} instance(s).", repaired.len());
    } else {
        crate::error_msg!(
            "{} instance(s) repaired, {} failed: {}",
            repaired.len(),
            failed.len(),
            failed.join("; ")
        );
        std::process::exit(1);
    }
}

fn cmd_completions(shell: Option<&str>) {
    use clap_complete::{generate, shells};
    let shell = shell.unwrap_or_else(|| {
        crate::die!(
            "Specify a shell.",
            "Example: hot-rs-launcher completions fish > ~/.config/fish/completions/hot-rs-launcher.fish"
        )
    });
    let mut command = build_cli();
    let bin = command.get_name().to_string();
    let mut stdout = std::io::stdout();
    match shell {
        "bash" => generate(shells::Bash, &mut command, bin, &mut stdout),
        "zsh" => generate(shells::Zsh, &mut command, bin, &mut stdout),
        "fish" => generate(shells::Fish, &mut command, bin, &mut stdout),
        "powershell" | "pwsh" => generate(shells::PowerShell, &mut command, bin, &mut stdout),
        "elvish" => generate(shells::Elvish, &mut command, bin, &mut stdout),
        other => crate::die!(format!(
            "Unsupported shell '{other}'. Use bash, zsh, fish, powershell or elvish."
        )),
    }
}

fn cmd_backup(game_dir: &std::path::Path, id: &str, with_mods: bool) {
    let manager = InstanceManager::new(game_dir);
    crate::log::header(&format!("Backup {}", id));
    let path = manager
        .backup(id, with_mods)
        .unwrap_or_else(|e| crate::die!(format!("Backup failed: {e}")));
    let size = path.metadata().map(|meta| meta.len()).unwrap_or(0);
    crate::success!("Backup: {} ({})", path.display(), format_size(size));
    crate::info!(
        "Restore with: hot-rs-launcher restore-backup '{}'",
        path.display()
    );
}

fn cmd_list_backups(game_dir: &std::path::Path, json: bool) {
    let manager = InstanceManager::new(game_dir);
    let backups = manager.list_backups();
    if json {
        let value = serde_json::json!({
            "directory": manager.game_dir.join("backups"),
            "total": backups.len(),
            "backups": backups.iter().map(|(path, size, modified)| serde_json::json!({
                "name": path.file_name().map(|name| name.to_string_lossy().to_string()),
                "path": path,
                "size": size,
                "created": modified.and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map(|age| age.as_secs()),
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
        return;
    }
    crate::log::header("Backups");
    if backups.is_empty() {
        crate::warn_msg!("No backups yet.");
        crate::info!("Create one with: hot-rs-launcher backup -v <id>");
        return;
    }
    for (path, size, modified) in &backups {
        let age = modified
            .and_then(|time| SystemTime::now().duration_since(time).ok())
            .map(|elapsed| format!("{} ago", crate::tui::theme::format_duration(elapsed)))
            .unwrap_or_default();
        println!(
            "  {:<54} {:>9}  {}",
            path.file_name().unwrap_or_default().to_string_lossy(),
            format_size(*size),
            age
        );
    }
    crate::info!(
        "{} backup(s) in {}",
        backups.len(),
        manager.game_dir.join("backups").display()
    );
}

fn cmd_restore_backup(game_dir: &std::path::Path, archive: &std::path::Path, target: Option<&str>) {
    let manager = InstanceManager::new(game_dir);
    crate::log::header("Restore Backup");
    let (id, restored) = manager
        .restore(archive, target)
        .unwrap_or_else(|e| crate::die!(format!("Restore failed: {e}")));
    crate::success!("Restored {} file(s) into '{}'.", restored, id);
}

// ─── helpers ───────────────────────────────────────────────────────

fn resolve_mc_version(mc_version: Option<String>, game_dir: &std::path::Path) -> String {
    resolve_mc_version_quiet(mc_version, game_dir, false)
}

/// Same lookup, optionally suppressing the auto-detect message so JSON output
/// stays machine-readable.
fn resolve_mc_version_quiet(
    mc_version: Option<String>,
    game_dir: &std::path::Path,
    quiet: bool,
) -> String {
    if let Some(v) = mc_version {
        return v;
    }
    let installed = ModManager::list_installed_versions(game_dir);
    if installed.is_empty() {
        crate::die!(
            "No versions installed. Specify --version.\n  Download first: hot-rs-launcher download -v <version>"
        );
    }
    let v = installed.last().unwrap().clone();
    if !quiet {
        crate::info!("Auto-detected version: {}", v);
    }
    v
}

fn resolve_mc_version_or_latest(mc_version: Option<String>, game_dir: &std::path::Path) -> String {
    VersionManager::new(game_dir)
        .get_version_info(mc_version.as_deref())
        .unwrap_or_else(|e| crate::die!(format!("Cannot resolve Minecraft version: {e}")))
        .0
}

fn format_num(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    #[test]
    fn command_definition_is_consistent() {
        build_cli().debug_assert();
    }
    #[test]
    fn rejects_conflicts_and_out_of_range_arguments() {
        for args in [
            vec!["--forge", "--fabric"],
            vec!["--loader", "forge", "--neoforge"],
            vec!["--official", "--bmcl"],
            vec!["--threads", "0"],
            vec!["--threads", "65"],
            vec!["--width", "0"],
            vec!["--limit", "101"],
            vec!["-v", "../escape"],
            vec!["--loader", "quilt"],
            vec!["--ram", "0"],
        ] {
            assert!(build_cli()
                .try_get_matches_from(std::iter::once("hot-rs-launcher").chain(args))
                .is_err());
        }
    }
    #[test]
    fn ram_units_and_overflows() {
        assert_eq!(parse_ram("4G").unwrap(), 4096);
        assert_eq!(parse_ram("1.5 gb").unwrap(), 1536);
        assert_eq!(parse_ram("2048").unwrap(), 2048);
        for value in ["", "-1G", "0", "0.1M", "999999999999G", "NaN", "4T"] {
            assert!(parse_ram(value).is_err());
        }
    }
    #[test]
    fn launcher_and_minecraft_version_flags_are_distinct() {
        let parsed = build_cli()
            .try_get_matches_from(["hot-rs-launcher", "download", "-v", "1.21.4"])
            .unwrap();
        assert_eq!(parsed.get_one::<String>("version").unwrap(), "1.21.4");
        assert_eq!(
            build_cli()
                .try_get_matches_from(["hot-rs-launcher", "--launcher-version"])
                .unwrap_err()
                .kind(),
            clap::error::ErrorKind::DisplayVersion
        );
    }

    #[test]
    fn new_actions_and_launch_flags_parse() {
        for args in [
            vec!["list-java"],
            vec!["install-java", "--java-major", "21"],
            vec!["remove-version", "-v", "1.21.4", "--keep-saves"],
            vec!["rename-version", "-v", "1.21.4", "--name", "my-pack"],
            vec!["copy-version", "-v", "1.21.4", "--name", "copy-pack"],
            vec!["export-modpack", "-v", "1.21.4", "-o", "pack.mrpack"],
            vec![
                "import-modpack",
                "pack.mrpack",
                "--name",
                "pack",
                "--install-loader",
            ],
            vec!["update-mods", "-v", "1.21.4", "--apply", "--all"],
            vec!["diagnose", "-v", "1.21.4"],
            vec!["backup", "-v", "1.21.4", "--with-mods"],
            vec!["list-backups"],
            vec!["restore-backup", "b.zip", "-v", "1.21.4"],
            vec![
                "play",
                "--gc",
                "g1",
                "--dry-run",
                "--server",
                "mc.example.com:25565",
                "--jvm-args",
                "-Dfoo=bar",
                "--fullscreen",
                "--no-diagnose",
            ],
        ] {
            assert!(
                build_cli()
                    .try_get_matches_from(std::iter::once("hot-rs-launcher").chain(args.clone()))
                    .is_ok(),
                "should accept {args:?}"
            );
        }
        for args in [
            vec!["play", "--gc", "bogus"],
            vec!["play", "--server", "a b"],
            vec!["install-java", "--java-major", "4"],
            vec!["play", "--server", "host:0"],
        ] {
            assert!(
                build_cli()
                    .try_get_matches_from(std::iter::once("hot-rs-launcher").chain(args.clone()))
                    .is_err(),
                "should reject {args:?}"
            );
        }
    }

    #[test]
    fn jvm_arguments_are_split_like_a_shell() {
        assert_eq!(
            split_args("-Dfoo='a b' -Xmx4G").unwrap(),
            vec!["-Dfoo=a b", "-Xmx4G"]
        );
        assert_eq!(split_args("a 'b c' d").unwrap(), vec!["a", "b c", "d"]);
        assert_eq!(split_args("").unwrap(), Vec::<String>::new());
        assert!(split_args("'unterminated").is_err());
        assert_eq!(split_args("  ").unwrap(), Vec::<String>::new());
    }
}
