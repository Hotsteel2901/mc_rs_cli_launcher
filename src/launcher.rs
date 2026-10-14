//! Minecraft game launcher — classpath construction, JVM args, process launch.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::auth::MicrosoftAuth;
use crate::error::{AppError, AppResult};
use crate::java;
use crate::mod_manager::ModManager;
use crate::util;
use crate::version::VersionManager;

const LAUNCHER_NAME: &str = "simple-mc-cli";
const LAUNCHER_VER: &str = env!("CARGO_PKG_VERSION");

pub struct MinecraftLauncher {
    pub game_dir: PathBuf,
    pub accounts: crate::account::AccountManager,
    pub versions: VersionManager,
    java_path: Option<String>,
    pub threads: usize,
}

impl MinecraftLauncher {
    pub fn new(game_dir: &Path, threads: usize) -> AppResult<Self> {
        let game_dir = util::absolute_path(game_dir)?;
        Ok(MinecraftLauncher {
            game_dir: game_dir.to_path_buf(),
            accounts: crate::account::AccountManager::new(&game_dir)?,
            versions: VersionManager::new(&game_dir),
            java_path: None,
            threads: threads.clamp(1, 32),
        })
    }

    /// Set a specific Java path.
    pub fn set_java(&mut self, path: &str) {
        self.java_path = Some(path.to_string());
    }

    /// Get the Java executable, auto-detecting if necessary.
    pub fn java(&mut self) -> AppResult<String> {
        if let Some(ref j) = self.java_path {
            return Ok(j.clone());
        }
        let j = java::check_java(None).ok_or_else(|| {
            AppError::Generic("Java not found. Install Java 17+ from https://adoptium.net/".into())
        })?;
        self.java_path = Some(j.clone());
        Ok(j)
    }

    /// Select Java matching the version's required major version.
    fn select_java(&mut self, version_data: &serde_json::Value) -> AppResult<String> {
        let jv = &version_data["javaVersion"];
        let required = jv["majorVersion"].as_u64();
        let component = jv["component"].as_str().unwrap_or("java-runtime-gamma");

        // If Java was explicitly set by user
        if let Some(ref explicit) = self.java_path {
            if let Some(req) = required {
                if let Some(major) = java::java_major(explicit) {
                    if major != req as u32 {
                        crate::warn_msg!(
                            "Specified Java is version {}, but Minecraft {} expects Java {}. Mods may crash.",
                            major,
                            version_data["id"].as_str().unwrap_or("?"),
                            req
                        );
                    }
                }
            }
            return Ok(explicit.clone());
        }

        // No specific requirement — use default
        if required.is_none() {
            return self.java();
        }
        let required = required.unwrap() as u32;

        // Try to find an exact match
        if let Some(j) = java::check_java(Some(required)) {
            self.java_path = Some(j.clone());
            return Ok(j);
        }

        // Try cached Mojang runtime
        let exe_name = if cfg!(windows) { "java.exe" } else { "java" };
        let cached = glob::glob(&format!(
            "{}/java/{}/**/bin/{}",
            self.game_dir.display(),
            component,
            exe_name
        ))
        .ok()
        .and_then(|iter| iter.flatten().next())
        .map(|p| p.to_string_lossy().to_string());

        if let Some(j) = cached {
            self.java_path = Some(j.clone());
            return Ok(j);
        }

        // Download Mojang runtime
        crate::info!(
            "No local Java {} found; fetching Mojang Java runtime...",
            required
        );
        if let Some(j) = java::download_mojang_java(&self.game_dir, component, self.threads) {
            self.java_path = Some(j.clone());
            return Ok(j);
        }

        // Fallback to system Java
        crate::warn_msg!(
            "Could not get Java {}; falling back to system Java. Modded launches may crash.",
            required
        );
        self.java()
    }

    /// Do not silently turn a requested or previously installed modded instance into vanilla.
    fn load_loader_profile(
        &self,
        mc_version: &str,
        loader: Option<&str>,
    ) -> AppResult<(Option<String>, Option<serde_json::Value>)> {
        util::validate_component(mc_version)?;
        let candidates = loader
            .map(|name| vec![name])
            .unwrap_or_else(|| vec!["fabric", "neoforge", "forge"]);
        for name in candidates {
            if !["fabric", "forge", "neoforge"].contains(&name) {
                return Err("Unsupported loader".into());
            }
            let path = self
                .game_dir
                .join("libraries")
                .join(name)
                .join(format!("{name}-profile-{mc_version}.json"));
            if !path.exists() {
                continue;
            }
            let profile: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
            if !profile["mainClass"].is_string()
                || !profile["libraries"].is_array()
                || (name != "fabric" && profile["profile_format"].as_u64() != Some(1))
            {
                return Err(AppError::Loader(format!("Invalid or legacy {name} profile. Reinstall with: mc-launcher install-{name} -v {mc_version}")));
            }
            return Ok((Some(name.into()), Some(profile)));
        }
        if let Some(name) = loader {
            return Err(AppError::Loader(format!("No {name} profile for {mc_version}. Run: mc-launcher install-{name} -v {mc_version}")));
        }
        Ok((None, None))
    }

    /// Ensure the session is valid (refresh if needed).
    fn ensure_session(
        &mut self,
        account_data: &mut serde_json::Value,
    ) -> AppResult<(String, String, String, String)> {
        let acc_type = account_data["type"]
            .as_str()
            .unwrap_or("offline")
            .to_string();
        let mut username = account_data["username"]
            .as_str()
            .unwrap_or("Player")
            .to_string();
        let mut user_uuid = account_data["uuid"]
            .as_str()
            .unwrap_or("00000000-0000-0000-0000-000000000000")
            .to_string();

        let access_token = if acc_type == "msa" {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            if now + 60.0 >= account_data["expires_at"].as_f64().unwrap_or(0.0)
                || account_data["access_token"]
                    .as_str()
                    .unwrap_or("")
                    .is_empty()
            {
                crate::info!("Session expired. Attempting silent token refresh...");
                let mut auth = MicrosoftAuth::new();
                let refresh = account_data["refresh_token"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();

                if !auth.try_refresh(&refresh) {
                    return Err(AppError::Generic(
                        "Token refresh failed. Run 'login' again to re-authenticate.".into(),
                    ));
                }

                // Update account_data in memory with new token
                account_data["access_token"] = serde_json::json!(auth.mc_token);
                account_data["refresh_token"] = serde_json::json!(auth.refresh_token);
                account_data["expires_at"] = serde_json::json!(auth.expires_at);

                let uid = util::format_uuid(&auth.uuid);
                self.accounts.set_msa(
                    &auth.username,
                    &uid,
                    &auth.mc_token,
                    &auth.refresh_token,
                    auth.expires_at,
                )?;
                username = auth.username.clone();
                user_uuid = uid;
                crate::success!("Token refreshed -- {}", auth.username);
                auth.mc_token
            } else {
                account_data["access_token"]
                    .as_str()
                    .unwrap_or("0")
                    .to_string()
            }
        } else {
            "0".to_string()
        };

        Ok((acc_type, username, user_uuid, access_token))
    }

    /// Launch Minecraft.
    pub fn launch(
        &mut self,
        version_id: Option<&str>,
        account_data: Option<serde_json::Value>,
        ram_mb: u32,
        loader: Option<&str>,
        width: Option<u32>,
        height: Option<u32>,
    ) -> AppResult<i32> {
        let mut account_data = account_data.unwrap_or_else(|| {
            self.accounts.get_default().unwrap_or_else(|| {
                crate::die!("No account. Run 'login' or 'offline <name>' first.");
            })
        });

        let (acc_type, username, user_uuid, access_token) =
            self.ensure_session(&mut account_data)?;

        let (version_id_str, version_data) = self.versions.get_version_info(version_id)?;
        let mc_version = &version_id_str;

        let version_game_dir = self.game_dir.join("versions").join(mc_version);
        std::fs::create_dir_all(&version_game_dir)?;

        let (loader_name, loader_profile) = self.load_loader_profile(mc_version, loader)?;

        if let Some(ref ln) = loader_name {
            let title = match ln.as_str() {
                "fabric" => "Fabric",
                "forge" => "Forge",
                "neoforge" => "NeoForge",
                _ => ln,
            };
            crate::log::header(&format!(
                "Minecraft {} ({}) | {} ({})",
                mc_version, title, username, acc_type
            ));
        } else {
            crate::log::header(&format!(
                "Minecraft {} | {} ({})",
                mc_version, username, acc_type
            ));
        }
        crate::info!("Game dir: {}\n", version_game_dir.display());

        crate::log::step(1, 4, "Downloading client jar...");
        let client_jar = self
            .versions
            .download_client_jar(mc_version, &version_data)?;

        let natives_dir = self.game_dir.join("natives").join(mc_version);
        std::fs::create_dir_all(&natives_dir)?;

        crate::log::step(2, 4, "Downloading libraries...");
        let mut library_data = version_data.clone();
        if let Some(profile) = &loader_profile {
            library_data["libraries"] =
                merged_libraries(&version_data["libraries"], &profile["libraries"]);
        }
        let lib_jars =
            self.versions
                .download_libraries(&library_data, &natives_dir, self.threads)?;

        crate::log::step(3, 4, "Downloading assets...");
        let assets_index = self.versions.download_assets(&version_data, self.threads)?;

        crate::log::step(4, 4, "Launching game...");

        let sep = if cfg!(windows) { ";" } else { ":" };
        if loader_profile.is_some() {
            // Every loader discovers mods from gameDir/mods. Do not inject them into the JVM classpath.
            // Mods for Fabric (Forge/NeoForge discover mods on their own)
            let mm = ModManager::new(&self.game_dir);
            let mods_dir = mm.mods_dir(mc_version);
            let mod_jars: Vec<PathBuf> = std::fs::read_dir(&mods_dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    name.ends_with(".jar")
                        && !name.ends_with(".disabled")
                        && !name.ends_with("-sources.jar")
                        && !name.ends_with("-javadoc.jar")
                })
                .map(|e| e.path())
                .collect();

            if !mod_jars.is_empty() {
                crate::info!("Mods: {} jar(s)", mod_jars.len());
            }
        }

        // Build classpath
        let mut all_cp: Vec<PathBuf> = Vec::with_capacity(1 + lib_jars.len());
        all_cp.push(client_jar);
        all_cp.extend(lib_jars);

        // Deduplicate
        let mut seen = std::collections::HashSet::new();
        let mut deduped = Vec::new();
        for p in all_cp {
            let key = p.to_string_lossy().to_string();
            if seen.insert(key) {
                deduped.push(p);
            }
        }
        let classpath = deduped
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(sep);

        // Main class
        let main_class = loader_profile
            .as_ref()
            .and_then(|p| p["mainClass"].as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                version_data["mainClass"]
                    .as_str()
                    .unwrap_or("net.minecraft.client.main.Main")
                    .to_string()
            });

        // --- JVM args ---
        let mut jvm_args: Vec<String> = Vec::new();

        if let Some(args) = version_data["arguments"]["jvm"].as_array() {
            for arg in args {
                match arg {
                    serde_json::Value::String(s) => jvm_args.push(s.clone()),
                    serde_json::Value::Object(obj) => {
                        if let Some(rules) = obj.get("rules") {
                            if !VersionManager::rules_allow_with_features(
                                rules,
                                false,
                                &[("has_custom_resolution", width.is_some() || height.is_some())],
                            ) {
                                continue;
                            }
                        }
                        if let Some(val) = obj.get("value") {
                            match val {
                                serde_json::Value::String(s) => {
                                    jvm_args.push(s.clone());
                                }
                                serde_json::Value::Array(arr) => {
                                    jvm_args.extend(
                                        arr.iter().filter_map(|v| v.as_str()).map(String::from),
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else {
            jvm_args.extend_from_slice(&[
                "-Djava.library.path=${natives_directory}".to_string(),
                "-cp".to_string(),
                "${classpath}".to_string(),
            ]);
        }

        // RAM
        let has_xmx = jvm_args.iter().any(|a| a.starts_with("-Xmx"));
        let has_xms = jvm_args.iter().any(|a| a.starts_with("-Xms"));
        if !has_xmx {
            jvm_args.insert(0, format!("-Xmx{}M", ram_mb));
        }
        if !has_xms {
            jvm_args.insert(0, format!("-Xms{}M", (1024u32).min(ram_mb / 2)));
        }

        // Native path
        if !jvm_args
            .iter()
            .any(|a| a.to_lowercase().contains("natives"))
        {
            jvm_args.push(format!("-Djava.library.path={}", natives_dir.display()));
        }

        // Loader JVM args
        if let Some(ref profile) = loader_profile {
            if let Some(args) = profile["arguments"]["jvm"].as_array() {
                for arg in args {
                    match arg {
                        serde_json::Value::String(s) => {
                            jvm_args.push(s.clone());
                        }
                        serde_json::Value::Object(obj) => {
                            if let Some(rules) = obj.get("rules") {
                                if !VersionManager::rules_allow_with_features(
                                    rules,
                                    false,
                                    &[(
                                        "has_custom_resolution",
                                        width.is_some() || height.is_some(),
                                    )],
                                ) {
                                    continue;
                                }
                            }
                            if let Some(val) = obj.get("value") {
                                match val {
                                    serde_json::Value::String(s) => {
                                        jvm_args.push(s.clone());
                                    }
                                    serde_json::Value::Array(arr) => {
                                        jvm_args.extend(
                                            arr.iter().filter_map(|v| v.as_str()).map(String::from),
                                        );
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // --- Game args ---
        let mut game_args: Vec<String> = Vec::new();

        if let Some(args) = version_data["arguments"]["game"].as_array() {
            for arg in args {
                match arg {
                    serde_json::Value::String(s) => {
                        game_args.push(s.clone());
                    }
                    serde_json::Value::Object(obj) => {
                        if let Some(rules) = obj.get("rules") {
                            if !VersionManager::rules_allow_with_features(
                                rules,
                                false,
                                &[("has_custom_resolution", width.is_some() || height.is_some())],
                            ) {
                                continue;
                            }
                        }
                        if let Some(val) = obj.get("value") {
                            match val {
                                serde_json::Value::String(s) => {
                                    game_args.push(s.clone());
                                }
                                serde_json::Value::Array(arr) => {
                                    game_args.extend(
                                        arr.iter().filter_map(|v| v.as_str()).map(String::from),
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else if let Some(mca) = version_data["minecraftArguments"].as_str() {
            game_args = mca.split_whitespace().map(String::from).collect();
        }

        // Loader game args
        if let Some(ref profile) = loader_profile {
            if let Some(args) = profile["arguments"]["game"].as_array() {
                for arg in args {
                    match arg {
                        serde_json::Value::String(s) => {
                            game_args.push(s.clone());
                        }
                        serde_json::Value::Object(obj) => {
                            if let Some(rules) = obj.get("rules") {
                                if !VersionManager::rules_allow_with_features(
                                    rules,
                                    false,
                                    &[(
                                        "has_custom_resolution",
                                        width.is_some() || height.is_some(),
                                    )],
                                ) {
                                    continue;
                                }
                            }
                            if let Some(val) = obj.get("value") {
                                match val {
                                    serde_json::Value::String(s) => {
                                        game_args.push(s.clone());
                                    }
                                    serde_json::Value::Array(arr) => {
                                        game_args.extend(
                                            arr.iter().filter_map(|v| v.as_str()).map(String::from),
                                        );
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // --- Token replacement ---
        // Offline mode: do not pass auth session/access_token to Minecraft
        if acc_type == "offline" {
            game_args.retain(|arg| {
                !arg.starts_with("--auth_session") && !arg.starts_with("--auth_access_token")
            });
        }

        let replacements: Vec<(&str, String)> = vec![
            ("${auth_player_name}", username.clone()),
            ("${auth_uuid}", user_uuid.replace('-', "")),
            ("${auth_access_token}", access_token.clone()),
            ("${auth_session}", access_token.clone()),
            ("${clientid}", "0".into()),
            ("${xuid}", "0".into()),
            ("${auth_xuid}", "0".into()),
            (
                "${user_type}",
                if acc_type == "msa" {
                    "msa".into()
                } else {
                    "legacy".into()
                },
            ),
            (
                "${user_properties}",
                account_data
                    .get("user_properties")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "{}".to_string()),
            ),
            ("${version_name}", mc_version.clone()),
            (
                "${version_type}",
                version_data["type"].as_str().unwrap_or("release").into(),
            ),
            (
                "${game_directory}",
                version_game_dir.to_string_lossy().to_string(),
            ),
            (
                "${game_assets}",
                self.versions
                    .assets_dir
                    .join("virtual")
                    .join(&assets_index)
                    .to_string_lossy()
                    .to_string(),
            ),
            (
                "${assets_root}",
                self.versions.assets_dir.to_string_lossy().to_string(),
            ),
            ("${assets_index_name}", assets_index),
            ("${launcher_name}", LAUNCHER_NAME.into()),
            ("${launcher_version}", LAUNCHER_VER.into()),
            ("${classpath_separator}", sep.into()),
            ("${classpath}", classpath.clone()),
            (
                "${natives_directory}",
                natives_dir.to_string_lossy().to_string(),
            ),
            (
                "${library_directory}",
                self.versions.libraries_dir.to_string_lossy().to_string(),
            ),
            (
                "${resolution_width}",
                width.map(|w| w.to_string()).unwrap_or_else(|| "854".into()),
            ),
            (
                "${resolution_height}",
                height
                    .map(|h| h.to_string())
                    .unwrap_or_else(|| "480".into()),
            ),
        ];

        fn replace_tokens(args: &mut [String], replacements: &[(&str, String)]) {
            for arg in args.iter_mut() {
                for (tok, val) in replacements {
                    *arg = arg.replace(tok, val);
                }
            }
        }

        replace_tokens(&mut jvm_args, &replacements);
        replace_tokens(&mut game_args, &replacements);

        for arg in jvm_args.iter().chain(game_args.iter()) {
            if arg.contains("${") {
                crate::debug_msg!("Unresolved token: {}", arg);
            }
        }

        if width.is_some() || height.is_some() {
            for (flag, value) in [
                ("--width", width.unwrap_or(854)),
                ("--height", height.unwrap_or(480)),
            ] {
                if !game_args.iter().any(|arg| arg == flag) {
                    game_args.extend([flag.into(), value.to_string()]);
                }
            }
        }

        // Ensure -cp is present
        if !jvm_args.iter().any(|a| a == "-cp" || a == "-classpath") {
            jvm_args.push("-cp".into());
            jvm_args.push(classpath);
        }

        // Get Java
        let java_path = self.select_java(&version_data)?;
        let java_ver = java::java_major(&java_path);

        let mut cmd_parts: Vec<String> =
            Vec::with_capacity(1 + jvm_args.len() + 1 + game_args.len());
        cmd_parts.push(java_path.clone());
        cmd_parts.extend(jvm_args);
        cmd_parts.push(main_class);
        cmd_parts.extend(game_args);

        crate::info!(
            "Java:    {} {}",
            java_path,
            java_ver
                .map(|v| format!("(Java {})", v))
                .unwrap_or_default()
        );
        crate::info!("Version: {}", mc_version);
        crate::info!("Player:  {}", username);
        crate::info!("RAM:     {} MB", ram_mb);
        crate::success!("Starting Minecraft...\n");

        let mut child = Command::new(&java_path)
            .args(&cmd_parts[1..]) // Skip java path since it's in Command::new
            .current_dir(&version_game_dir)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| AppError::Generic(format!("Failed to start Minecraft: {}", e)))?;

        let pid = child.id();
        crate::success!("Minecraft PID: {}", pid);
        crate::info!("Waiting for game to close... (Ctrl+C to force quit)\n");

        let status = child
            .wait()
            .map_err(|e| AppError::Generic(format!("Error waiting for Minecraft: {e}")))?;
        Ok(status.code().unwrap_or(1))
    }

    /// Download a Minecraft version without launching.
    pub fn download_version(
        &mut self,
        version_id: Option<&str>,
        skip_assets: bool,
    ) -> AppResult<(String, serde_json::Value)> {
        let (version_id, version_data) = self.versions.get_version_info(version_id)?;

        let version_game_dir = self.game_dir.join("versions").join(&version_id);
        std::fs::create_dir_all(&version_game_dir)?;

        println!("\n  Target:  Minecraft {}", version_id);
        println!("  Dir:     {}", self.game_dir.display());
        println!(
            "  Game:    {}  (version-isolated)\n",
            version_game_dir.display()
        );

        println!("[1/3] Client jar...");
        let jar = self
            .versions
            .download_client_jar(&version_id, &version_data)?;
        let jar_mb = jar.metadata().map(|m| m.len()).unwrap_or(0) as f64 / 1_048_576.0;
        println!("       {}  ({:.1} MB)", jar.display(), jar_mb);

        println!("[2/3] Libraries + natives...");
        let natives_dir = self.game_dir.join("natives").join(&version_id);
        if natives_dir.exists() {
            std::fs::remove_dir_all(&natives_dir)?;
        }
        std::fs::create_dir_all(&natives_dir)?;
        let lib_jars =
            self.versions
                .download_libraries(&version_data, &natives_dir, self.threads)?;
        let lib_count = lib_jars.len();
        let lib_mb: f64 = lib_jars
            .iter()
            .filter_map(|p| p.metadata().ok().map(|m| m.len()))
            .sum::<u64>() as f64
            / 1_048_576.0;
        println!("       {} jars  (~{:.1} MB)", lib_count, lib_mb);

        if skip_assets {
            println!("[3/3] Assets skipped (--no-assets).");
        } else {
            println!("[3/3] Assets...");
            self.versions.download_assets(&version_data, self.threads)?;
        }

        let total_mb = jar_mb + lib_mb;
        println!(
            "\n  [OK] Minecraft {} downloaded (~{:.0} MB) -> {}",
            version_id,
            total_mb,
            self.game_dir.display()
        );
        println!(
            "    Game data (saves, mods, etc.) isolated to: {}",
            version_game_dir.display()
        );

        let account = self.accounts.get_default();
        if let Some(acc) = account {
            println!(
                "    Account: {} ({})",
                acc["username"].as_str().unwrap_or("?"),
                acc["type"].as_str().unwrap_or("?")
            );
            println!("    Launch:  mc-launcher play -v {}", version_id);
        } else {
            println!("    No account saved yet. Login first:");
            println!("      mc-launcher login");
            println!("    Then launch:");
            println!("      mc-launcher play -v {}", version_id);
        }

        for loader in &["fabric", "forge", "neoforge"] {
            let p = self
                .game_dir
                .join("libraries")
                .join(loader)
                .join(format!("{}-profile-{}.json", loader, version_id));
            if p.exists() {
                println!(
                    "    {}:  detected -- add --{} to launch command",
                    match *loader {
                        "fabric" => "Fabric",
                        "forge" => "Forge",
                        "neoforge" => "NeoForge",
                        _ => loader,
                    },
                    loader
                );
            }
        }

        Ok((version_id, version_data))
    }

    #[allow(dead_code)]
    pub fn assets_dir(&self) -> &Path {
        &self.versions.assets_dir
    }
}

/// Loader libraries override the same Maven artifact/classifier, not just identical paths.
fn merged_libraries(base: &serde_json::Value, child: &serde_json::Value) -> serde_json::Value {
    let mut merged = Vec::<serde_json::Value>::new();
    let mut positions = std::collections::HashMap::new();
    for lib in base
        .as_array()
        .into_iter()
        .flatten()
        .chain(child.as_array().into_iter().flatten())
    {
        if !VersionManager::rules_allow(&lib["rules"], true) {
            continue;
        }
        let name = lib["name"].as_str().unwrap_or("");
        let (coordinate, extension) = name.split_once('@').unwrap_or((name, "jar"));
        let parts: Vec<_> = coordinate.split(':').collect();
        let key = format!(
            "{}:{}:{}@{}",
            parts.first().unwrap_or(&""),
            parts.get(1).unwrap_or(&""),
            parts.get(3).unwrap_or(&""),
            extension
        );
        if let Some(&position) = positions.get(&key) {
            merged[position] = lib.clone();
        } else {
            positions.insert(key, merged.len());
            merged.push(lib.clone());
        }
    }
    serde_json::Value::Array(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loader_library_versions_override_vanilla_without_dropping_classifiers() {
        let base = serde_json::json!([{"name":"g:a:1"},{"name":"g:a:1:natives-linux"}]);
        let child = serde_json::json!([{"name":"g:a:2"},{"name":"g:b:1"}]);
        let merged = merged_libraries(&base, &child);
        assert_eq!(merged.as_array().unwrap().len(), 3);
        assert_eq!(merged[0]["name"], "g:a:2");
        assert_eq!(merged[1]["name"], "g:a:1:natives-linux");
    }
}
