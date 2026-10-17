//! Black-box tests: no Microsoft account, keychain or Internet required.
use std::process::{Command, Output};
fn run(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mc-launcher"))
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
    let result = Command::new(env!("CARGO_BIN_EXE_mc-launcher"))
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
