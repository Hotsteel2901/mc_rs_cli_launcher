//! Black-box tests: no Microsoft account, keychain or Internet required.
use std::process::{Command, Output};
fn run(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hot-rs-launcher"))
        .args(args)
        .arg("--dir")
        .arg(dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}
#[test]
fn offline_account_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let result = run(dir.path(), &["offline", "Steve"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let accounts = run(dir.path(), &["accounts"]);
    assert!(accounts.status.success());
    assert!(String::from_utf8_lossy(&accounts.stdout).contains("Steve"));
    assert!(run(dir.path(), &["logout"]).status.success());
    let accounts = run(dir.path(), &["accounts"]);
    assert!(String::from_utf8_lossy(&accounts.stdout).contains("No accounts saved"));
}
#[test]
fn invalid_input_does_not_create_game_directory() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("not-created");
    for args in [
        vec!["play", "--threads", "0"],
        vec!["play", "--fabric", "--forge"],
        vec!["download", "-v", "../../outside"],
    ] {
        assert!(!run(&game, &args).status.success());
    }
    assert!(!game.exists());
}
#[test]
fn help_and_listing_work_without_network_or_java() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["--help"],
        vec!["--launcher-version"],
        vec!["list-installed"],
        vec!["accounts"],
        vec!["list-mods", "-v", "1.21.4"],
    ] {
        assert!(run(dir.path(), &args).status.success());
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
#[test]
fn about_lists_author_and_both_repositories() {
    let dir = tempfile::tempdir().unwrap();
    let result = run(dir.path(), &["--about"]);
    assert!(result.status.success());
    let text = String::from_utf8_lossy(&result.stdout);
    assert!(text.contains("Hot-RS-Launcher"), "{text}");
    assert!(text.contains("Hotsteel2901"), "{text}");
    assert!(
        text.contains("github.com/Hotsteel2901/mc_rs_cli_launcher"),
        "{text}"
    );
    assert!(
        text.contains("codeberg.org/hotsteel/mc_rs_cli_launcher"),
        "{text}"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn corrupt_accounts_are_reported_without_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("launcher_accounts.json");
    std::fs::write(&file, "broken json").unwrap();
    assert!(!run(dir.path(), &["offline", "Steve"]).status.success());
    assert_eq!(std::fs::read_to_string(file).unwrap(), "broken json");
}

#[test]
fn tui_refuses_a_pipe_instead_of_hanging() {
    let dir = tempfile::tempdir().unwrap();
    let result = run(dir.path(), &["tui"]);
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("interactive terminal"),
        "unexpected stderr: {stderr}"
    );
    assert!(!dir.path().join("launcher_tui.json").exists());
}

#[cfg(unix)]
#[test]
fn cached_game_launch_uses_absolute_paths_custom_resolution_and_child_exit_code() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let game = root.path().join("game with spaces");
    let version = game.join("versions/test-version");
    std::fs::create_dir_all(&version).unwrap();
    let mut jar =
        zip::ZipWriter::new(std::fs::File::create(version.join("test-version.jar")).unwrap());
    jar.start_file("fixture.txt", zip::write::FileOptions::default())
        .unwrap();
    jar.write_all(b"fixture, not a real Minecraft client")
        .unwrap();
    jar.finish().unwrap();
    let metadata = serde_json::json!({
        "id":"test-version","type":"release","mainClass":"TestMain","libraries":[],
        "downloads":{"client":{"url":"http://127.0.0.1:1/unreachable"}},
        "javaVersion":{"majorVersion":17},
        "arguments":{"game":["--username","${auth_player_name}","--gameDir","${game_directory}",
            {"rules":[{"action":"allow","features":{"has_custom_resolution":true}}],
             "value":["--width","${resolution_width}","--height","${resolution_height}"]}]}
    });
    std::fs::write(version.join("test-version.json"), metadata.to_string()).unwrap();
    assert!(run(&game, &["offline", "Steve"]).status.success());
    let java = root.path().join("fake java");
    std::fs::write(&java, "#!/bin/sh\nif [ \"$1\" = '-version' ]; then echo 'openjdk version \"17.0.1\"' >&2; exit 0; fi\nprintf '%s\\n' \"$@\" > launch-args.txt\npwd > launch-cwd.txt\nexit 7\n").unwrap();
    std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_hot-rs-launcher"))
        .args([
            "play",
            "-v",
            "test-version",
            "--dir",
            "game with spaces",
            "--ram",
            "2G",
            "--width",
            "1024",
            "--height",
            "600",
            "--java",
        ])
        .arg(&java)
        .current_dir(root.path())
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(7),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let args = std::fs::read_to_string(version.join("launch-args.txt")).unwrap();
    assert!(args.contains("-Xmx2048M\n"));
    assert!(args.contains("--username\nSteve\n"));
    assert!(args.contains("--width\n1024\n--height\n600\n"));
    assert!(args.contains(game.canonicalize().unwrap().to_str().unwrap()));
    assert!(!args.contains("${"));
    assert_eq!(
        std::fs::read_to_string(version.join("launch-cwd.txt"))
            .unwrap()
            .trim(),
        version.canonicalize().unwrap().to_str().unwrap()
    );
}

/// A minimal installed instance: version json, jar and (optionally) saves/mods.
fn make_instance(game: &std::path::Path, id: &str) {
    let dir = game.join("versions").join(id);
    std::fs::create_dir_all(dir.join("saves").join("world")).unwrap();
    std::fs::write(dir.join("saves").join("world").join("level.dat"), b"level").unwrap();
    std::fs::create_dir_all(dir.join("mods")).unwrap();
    std::fs::write(dir.join("mods").join("example.jar"), b"jar").unwrap();
    let metadata = serde_json::json!({
        "id": id, "type": "release", "mainClass": "TestMain", "libraries": [],
        "downloads": {"client": {"url": "http://127.0.0.1:1/unreachable"}},
        "javaVersion": {"majorVersion": 17}
    });
    std::fs::write(dir.join(format!("{id}.json")), metadata.to_string()).unwrap();
    std::fs::write(dir.join(format!("{id}.jar")), b"jar").unwrap();
}

#[test]
fn instance_lifecycle_commands_work_offline() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    make_instance(&game, "1.21.4");

    assert!(run(
        &game,
        &["rename-version", "-v", "1.21.4", "--name", "my-pack"]
    )
    .status
    .success());
    assert!(game.join("versions/my-pack/my-pack.json").is_file());
    assert!(!game.join("versions/1.21.4").exists());

    assert!(run(
        &game,
        &["copy-version", "-v", "my-pack", "--name", "copy-pack"]
    )
    .status
    .success());
    assert!(game.join("versions/copy-pack/copy-pack.json").is_file());

    let removed = run(
        &game,
        &["remove-version", "-v", "copy-pack", "--keep-saves"],
    );
    assert!(removed.status.success());
    assert!(String::from_utf8_lossy(&removed.stdout).contains("kept"));
    assert!(game
        .join("versions/copy-pack/saves/world/level.dat")
        .is_file());
    assert!(!game.join("versions/copy-pack/copy-pack.json").exists());

    assert!(run(&game, &["remove-version", "-v", "my-pack"])
        .status
        .success());
    assert!(!game.join("versions/my-pack").exists());
    assert!(!run(&game, &["remove-version", "-v", "missing"])
        .status
        .success());
    assert!(!run(&game, &["rename-version", "-v", "copy-pack"])
        .status
        .success());
}

#[test]
fn modpack_and_backup_round_trip_works_offline() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    make_instance(&game, "1.21.4");
    // No network: the exported pack must contain no Modrinth lookups.
    std::fs::remove_file(game.join("versions/1.21.4/mods/example.jar")).unwrap();

    let pack = dir.path().join("pack.mrpack");
    let exported = run(
        &game,
        &[
            "export-modpack",
            "-v",
            "1.21.4",
            "-o",
            pack.to_str().unwrap(),
        ],
    );
    assert!(
        exported.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&exported.stderr)
    );
    assert!(pack.is_file());

    let imported = run(
        &game,
        &[
            "import-modpack",
            pack.to_str().unwrap(),
            "--name",
            "imported",
        ],
    );
    assert!(
        imported.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&imported.stderr)
    );
    assert!(game.join("versions/imported/imported.json").is_file());

    assert!(run(&game, &["backup", "-v", "1.21.4"]).status.success());
    let listed = run(&game, &["list-backups"]);
    assert!(listed.status.success());
    assert!(String::from_utf8_lossy(&listed.stdout).contains("1.21.4"));
    let archive = std::fs::read_dir(game.join("backups"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "zip"))
        .expect("backup archive");
    std::fs::remove_file(game.join("versions/1.21.4/saves/world/level.dat")).unwrap();
    let restored = run(&game, &["restore-backup", archive.to_str().unwrap()]);
    assert!(
        restored.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&restored.stderr)
    );
    assert!(game.join("versions/1.21.4/saves/world/level.dat").is_file());
}

#[test]
fn diagnose_reports_crash_reports_offline() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    make_instance(&game, "1.21.4");
    let reports = game.join("versions/1.21.4/crash-reports");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(
        reports.join("crash-test.txt"),
        "Description: Test crash\njava.lang.OutOfMemoryError: Java heap space\n",
    )
    .unwrap();
    let result = run(&game, &["diagnose", "-v", "1.21.4"]);
    assert!(result.status.success());
    let output = String::from_utf8_lossy(&result.stdout);
    assert!(output.contains("内存不足"), "{output}");
    let empty = run(&game, &["diagnose", "-v", "my-pack"]);
    assert!(empty.status.success());
    assert!(String::from_utf8_lossy(&empty.stdout).contains("No crash report"));

    let json = run(&game, &["diagnose", "-v", "1.21.4", "--json"]);
    assert!(json.status.success());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert!(value["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["title"].as_str().unwrap_or("").contains("内存")));
}

#[test]
fn content_commands_manage_worlds_and_resource_packs_offline() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    make_instance(&game, "1.21.4");
    let version = game.join("versions/1.21.4");
    let packs = version.join("resourcepacks");
    std::fs::create_dir_all(&packs).unwrap();
    {
        use std::io::Write;
        let file = std::fs::File::create(packs.join("Pack.zip")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("pack.mcmeta", zip::write::FileOptions::default())
            .unwrap();
        zip.write_all(b"{\"pack\":{\"pack_format\":15,\"description\":\"Test pack\"}}")
            .unwrap();
        zip.finish().unwrap();
    }
    std::fs::write(
        version.join("options.txt"),
        b"renderDistance:12\nresourcePacks:[\"file/Pack.zip\",\"vanilla\"]\n",
    )
    .unwrap();

    let saves = run(&game, &["list-saves", "-v", "1.21.4", "--json"]);
    assert!(
        saves.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&saves.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&saves.stdout).unwrap();
    assert_eq!(value["saves"][0]["folder"], "world");

    // Without -v the auto-detected version must not pollute JSON output.
    let auto = run(&game, &["list-saves", "--json"]);
    assert!(auto.status.success());
    let value: serde_json::Value = serde_json::from_slice(&auto.stdout).unwrap();
    assert_eq!(value["instance"], "1.21.4");
    assert_eq!(value["saves"][0]["name"], "world");

    let listed = run(&game, &["list-resourcepacks", "-v", "1.21.4", "--json"]);
    assert!(listed.status.success());
    let value: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(value["resourcepacks"][0]["file"], "Pack.zip");
    assert_eq!(value["resourcepacks"][0]["enabled"], true);
    assert_eq!(value["resourcepacks"][0]["pack_format"], 15);

    let source = dir.path().join("imported-pack.zip");
    std::fs::copy(packs.join("Pack.zip"), &source).unwrap();
    let imported_pack = run(
        &game,
        &[
            "import-resourcepack",
            source.to_str().unwrap(),
            "-v",
            "1.21.4",
        ],
    );
    assert!(
        imported_pack.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&imported_pack.stderr)
    );
    assert!(packs.join("imported-pack.zip").is_file());

    let archive = dir.path().join("world.zip");
    let backup = run(
        &game,
        &[
            "backup-save",
            "world",
            "-o",
            archive.to_str().unwrap(),
            "-v",
            "1.21.4",
        ],
    );
    assert!(
        backup.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&backup.stderr)
    );
    assert!(archive.is_file());

    assert!(!run(&game, &["delete-save", "world", "-v", "1.21.4"])
        .status
        .success());
    let deleted = run(&game, &["delete-save", "world", "-v", "1.21.4", "--yes"]);
    assert!(deleted.status.success());
    assert!(!version.join("saves/world").exists());

    let imported = run(
        &game,
        &[
            "import-save",
            archive.to_str().unwrap(),
            "--name",
            "Restored",
            "-v",
            "1.21.4",
        ],
    );
    assert!(
        imported.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&imported.stderr)
    );
    assert!(version.join("saves/Restored/level.dat").is_file());

    let disabled = run(&game, &["disable-resourcepack", "Pack.zip", "-v", "1.21.4"]);
    assert!(disabled.status.success());
    let options = std::fs::read_to_string(version.join("options.txt")).unwrap();
    assert!(!options.contains("file/Pack.zip"));
    assert!(options.contains("renderDistance:12"));
    let enabled = run(&game, &["enable-resourcepack", "Pack.zip", "-v", "1.21.4"]);
    assert!(enabled.status.success());
    let options = std::fs::read_to_string(version.join("options.txt")).unwrap();
    assert!(options.contains("file/Pack.zip"));

    assert!(
        !run(&game, &["delete-resourcepack", "Pack.zip", "-v", "1.21.4"])
            .status
            .success()
    );
    let removed = run(
        &game,
        &["delete-resourcepack", "Pack.zip", "-v", "1.21.4", "--yes"],
    );
    assert!(removed.status.success());
    assert!(!packs.join("Pack.zip").exists());
}

#[test]
fn shader_management_commands_work_offline() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    make_instance(&game, "1.21.4");
    let version = game.join("versions/1.21.4");
    let shaders = version.join("shaderpacks");
    std::fs::create_dir_all(&shaders).unwrap();
    {
        use std::io::Write;
        let file = std::fs::File::create(shaders.join("TestShader.zip")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(
            "shaders/shaders.properties",
            zip::write::FileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"profile.MEDIUM=\n").unwrap();
        zip.finish().unwrap();
    }
    std::fs::write(
        version.join("options.txt"),
        b"renderDistance:12\nshaderPack:TestShader.zip\n",
    )
    .unwrap();

    let listed = run(&game, &["list-shaders", "-v", "1.21.4", "--json"]);
    assert!(
        listed.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(value["shaders"][0]["file"], "TestShader.zip");
    assert_eq!(value["shaders"][0]["enabled"], true);
    assert_eq!(value["loaders"]["iris"], false);

    assert!(run(&game, &["disable-shader", "-v", "1.21.4"])
        .status
        .success());
    let options = std::fs::read_to_string(version.join("options.txt")).unwrap();
    assert!(options.contains("shaderPack:OFF"));
    assert!(options.contains("renderDistance:12"));

    assert!(
        run(&game, &["enable-shader", "TestShader.zip", "-v", "1.21.4"])
            .status
            .success()
    );
    let options = std::fs::read_to_string(version.join("options.txt")).unwrap();
    assert!(options.contains("shaderPack:TestShader.zip"));

    let source = dir.path().join("ImportedShader.zip");
    std::fs::copy(shaders.join("TestShader.zip"), &source).unwrap();
    let imported = run(
        &game,
        &["import-shader", source.to_str().unwrap(), "-v", "1.21.4"],
    );
    assert!(
        imported.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&imported.stderr)
    );
    assert!(shaders.join("ImportedShader.zip").is_file());

    assert!(!run(
        &game,
        &["delete-shader", "ImportedShader.zip", "-v", "1.21.4"]
    )
    .status
    .success());
    let removed = run(
        &game,
        &[
            "delete-shader",
            "ImportedShader.zip",
            "-v",
            "1.21.4",
            "--yes",
        ],
    );
    assert!(removed.status.success());
    assert!(!shaders.join("ImportedShader.zip").exists());
}

#[cfg(unix)]
#[test]
fn dry_run_prints_the_command_without_starting_the_game() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let game = root.path().join("game");
    make_instance(&game, "test-version");
    {
        use std::io::Write;
        let jar_path = game.join("versions/test-version/test-version.jar");
        let mut jar = zip::ZipWriter::new(std::fs::File::create(&jar_path).unwrap());
        jar.start_file("fixture.txt", zip::write::FileOptions::default())
            .unwrap();
        jar.write_all(b"fixture").unwrap();
        jar.finish().unwrap();
    }
    let java = root.path().join("fake-java");
    std::fs::write(
        &java,
        "#!/bin/sh\nif [ \"$1\" = '-version' ]; then echo 'openjdk version \"17.0.1\"' >&2; exit 0; fi\nprintf '%s\\n' \"$@\" > launch-args.txt\nexit 7\n",
    )
    .unwrap();
    std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o700)).unwrap();

    let result = Command::new(env!("CARGO_BIN_EXE_hot-rs-launcher"))
        .args([
            "play",
            "-v",
            "test-version",
            "--dir",
            game.to_str().unwrap(),
            "--java",
            java.to_str().unwrap(),
            "--ram",
            "2G",
            "--jvm-args",
            "-Dfoo=bar",
            "--gc",
            "g1",
            "--server",
            "mc.example.com:25565",
            "--fullscreen",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8_lossy(&result.stdout);
    for expected in [
        "-Xmx2048M",
        "-Dfoo=bar",
        "-XX:+UseG1GC",
        "--server",
        "mc.example.com",
        "--port",
        "25565",
        "--fullscreen",
    ] {
        assert!(output.contains(expected), "missing {expected} in {output}");
    }
    assert!(!game.join("versions/test-version/launch-args.txt").exists());
}

fn run_env(dir: &std::path::Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hot-rs-launcher"));
    command
        .args(args)
        .arg("--dir")
        .arg(dir)
        .env("NO_COLOR", "1");
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().unwrap()
}

#[test]
fn doctor_reports_environment_without_network_or_keychain() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_env(
        dir.path(),
        &["doctor", "--json"],
        &[("MC_LAUNCHER_NO_NETWORK", "1")],
    );
    assert!(
        result.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["ok"], true);
    let checks = value["checks"].as_array().unwrap();
    assert!(checks
        .iter()
        .any(|check| check["name"] == "Directory writable" && check["status"] == "ok"));
    // The diagnostic must not create accounts or secrets as a side effect.
    assert!(!dir.path().join("launcher_accounts.json").exists());
    assert!(!dir.path().join("launcher_secrets.key").exists());
}

#[test]
fn prune_deletes_only_unreferenced_library_files() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    make_instance(&game, "1.21.4");
    let metadata = serde_json::json!({
        "id": "1.21.4", "type": "release", "mainClass": "TestMain",
        "libraries": [{"name": "g:a:1"}],
        "downloads": {"client": {"url": "http://127.0.0.1:1/unreachable"}},
        "javaVersion": {"majorVersion": 17}
    });
    std::fs::write(
        game.join("versions/1.21.4/1.21.4.json"),
        metadata.to_string(),
    )
    .unwrap();
    let keep = game.join("libraries/g/a/1/a-1.jar");
    let junk = game.join("libraries/junk/x/1/x-1.jar");
    std::fs::create_dir_all(keep.parent().unwrap()).unwrap();
    std::fs::create_dir_all(junk.parent().unwrap()).unwrap();
    std::fs::write(&keep, b"keep").unwrap();
    std::fs::write(&junk, b"junk").unwrap();
    std::fs::create_dir_all(game.join("libraries/fabric")).unwrap();
    std::fs::write(
        game.join("libraries/fabric/fabric-profile-1.21.4.json"),
        b"{}",
    )
    .unwrap();

    let plan = run(&game, &["prune", "--json"]);
    assert!(plan.status.success());
    let value: serde_json::Value = serde_json::from_slice(&plan.stdout).unwrap();
    let libraries = value["libraries"].as_array().unwrap();
    assert!(libraries.iter().any(|path| path.as_str() == junk.to_str()));
    assert!(!libraries.iter().any(|path| path.as_str() == keep.to_str()));
    assert!(junk.is_file(), "dry run must not delete anything");

    let applied = run(&game, &["prune", "--yes"]);
    assert!(applied.status.success());
    assert!(!junk.exists());
    assert!(keep.is_file());
    assert!(game
        .join("libraries/fabric/fabric-profile-1.21.4.json")
        .is_file());
}

#[test]
fn json_listings_and_completions_are_scriptable() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    make_instance(&game, "1.21.4");

    let installed = run(&game, &["list-installed", "--json"]);
    assert!(installed.status.success());
    let value: serde_json::Value = serde_json::from_slice(&installed.stdout).unwrap();
    assert_eq!(value["instances"][0]["id"], "1.21.4");
    assert_eq!(value["account"], serde_json::Value::Null);

    let mods = run(&game, &["list-mods", "-v", "1.21.4", "--json"]);
    assert!(mods.status.success());
    let value: serde_json::Value = serde_json::from_slice(&mods.stdout).unwrap();
    assert_eq!(value["mods"][0]["file"], "example.jar");

    let backups = run(&game, &["list-backups", "--json"]);
    assert!(backups.status.success());
    let value: serde_json::Value = serde_json::from_slice(&backups.stdout).unwrap();
    assert_eq!(value["total"], 0);

    let completions = run(&dir.path().join("no-game"), &["completions", "fish"]);
    assert!(completions.status.success());
    assert!(String::from_utf8_lossy(&completions.stdout).contains("complete -c hot-rs-launcher"));
    assert!(!dir.path().join("no-game").exists());
}

#[test]
fn removing_a_base_version_keeps_shared_loader_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    make_instance(&game, "1.21.4");
    make_instance(&game, "pack");
    std::fs::write(
        game.join("versions/pack/instance.json"),
        br#"{"base":"1.21.4","loader":"fabric"}"#,
    )
    .unwrap();
    let profile = game.join("libraries/fabric/fabric-profile-1.21.4.json");
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(
        &profile,
        br#"{"mainClass":"x","libraries":[],"profile_format":1}"#,
    )
    .unwrap();

    let removed = run(&game, &["remove-version", "-v", "1.21.4"]);
    assert!(
        removed.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(profile.is_file(), "shared loader profile must survive");
    assert!(String::from_utf8_lossy(&removed.stdout).contains("still use it"));
}
