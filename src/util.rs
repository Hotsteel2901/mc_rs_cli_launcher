//! Platform utilities, hashing, file operations.

use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::{env, fs};

use sha1::{Digest, Sha1};

/// Shared RAM bounds for the CLI and the TUI so both accept the same values.
pub const MIN_RAM_MB: u32 = 128;
pub const MAX_RAM_MB: u32 = 65_536;

/// Detect OS name (matches Mojang naming).
pub fn os_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

/// Detect architecture.
pub fn os_arch() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "unknown"
    }
}

/// OS release string for Mojang's regex rules, obtained once per process.
pub fn os_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        let output = if cfg!(windows) {
            std::process::Command::new("cmd")
                .args(["/C", "ver"])
                .output()
        } else {
            std::process::Command::new("uname").arg("-r").output()
        };
        let text = output
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        if cfg!(windows) {
            regex::Regex::new(r"\d+\.\d+(?:\.\d+)*")
                .ok()
                .and_then(|re| re.find(&text).map(|m| m.as_str().to_string()))
                .unwrap_or_default()
        } else {
            text
        }
    })
}

/// Default game directory: <exe_dir>/minecraft
pub fn default_game_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("minecraft")
}

/// Compute SHA-1 hash of a file.
pub fn sha1_file(path: &Path) -> std::io::Result<String> {
    let mut hasher = Sha1::new();
    let mut file = File::open(path)?;
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// A ZIP must have a readable central directory, not just a four-byte header.
pub fn is_jar_intact(path: &Path) -> bool {
    File::open(path)
        .ok()
        .and_then(|file| zip::ZipArchive::new(file).ok())
        .is_some_and(|archive| !archive.is_empty())
}

/// Validate an untrusted single path component on every supported platform.
pub fn validate_component(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.ends_with(['.', ' '])
        || value
            .chars()
            .any(|c| c.is_control() || r#"/\:<>"|?*"#.contains(c))
    {
        return Err(format!("Unsafe path component: {value:?}"));
    }
    let stem = value.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix)
                .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        })
    {
        return Err(format!("Reserved path component: {value:?}"));
    }
    Ok(())
}

pub fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, String> {
    for part in relative.split('/') {
        validate_component(part)?;
    }
    Ok(root.join(relative))
}

pub fn absolute_path(path: &Path) -> std::io::Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            _ => result.push(component.as_os_str()),
        }
    }
    Ok(result)
}

/// Replace only after a complete write; temporary files are private (0600 on Unix).
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Maven coordinates: group:artifact:version[:classifier][@extension].
pub fn maven_rel_path(name: &str) -> Result<String, String> {
    let (coordinate, extension) = name.split_once('@').unwrap_or((name, "jar"));
    validate_component(extension)?;
    let parts: Vec<_> = coordinate.split(':').collect();
    if !(3..=4).contains(&parts.len()) {
        return Err(format!("Invalid Maven coordinate: {name}"));
    }
    for part in &parts {
        validate_component(part)?;
    }
    let group = parts[0].split('.').collect::<Vec<_>>();
    for part in &group {
        validate_component(part)?;
    }
    let classifier = parts.get(3).map(|c| format!("-{c}")).unwrap_or_default();
    Ok(format!(
        "{}/{}/{}/{}-{}{}.{}",
        group.join("/"),
        parts[1],
        parts[2],
        parts[1],
        parts[2],
        classifier,
        extension
    ))
}

/// Generate an offline-mode UUID from a username (Mojang algorithm).
pub fn offline_uuid(username: &str) -> String {
    let name = format!("OfflinePlayer:{}", username);
    let digest = md5::Md5::digest(name.as_bytes());
    let mut b = <[u8; 16]>::from(digest);
    b[6] = (b[6] & 0x0F) | 0x30;
    b[8] = (b[8] & 0x3F) | 0x80;
    let h = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Find a free TCP port.
#[allow(dead_code)]
pub fn find_free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .ok()
        .and_then(|s| s.local_addr().ok())
        .map(|a| a.port())
        .unwrap_or(25565)
}

/// Format a UUID string to include dashes if it's 32 hex chars.
pub fn format_uuid(raw: &str) -> String {
    if raw.len() == 32 && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        format!(
            "{}-{}-{}-{}-{}",
            &raw[0..8],
            &raw[8..12],
            &raw[12..16],
            &raw[16..20],
            &raw[20..32]
        )
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offline_uuid_known() {
        // Test that offline_uuid returns a consistent UUID for a known username
        let uuid = offline_uuid("Steve");
        // Should be a valid UUID format
        assert_eq!(uuid.len(), 36);
        assert_eq!(uuid.chars().filter(|&c| c == '-').count(), 4);
        // Deterministic test
        let uuid2 = offline_uuid("Steve");
        assert_eq!(uuid, uuid2);
    }

    #[test]
    fn test_offline_uuid_different_users() {
        let uuid1 = offline_uuid("Steve");
        let uuid2 = offline_uuid("Alex");
        assert_ne!(uuid1, uuid2);
    }

    #[test]
    fn test_maven_rel_path() {
        let path = maven_rel_path("com.example:test-lib:1.0.0").unwrap();
        assert_eq!(path, "com/example/test-lib/1.0.0/test-lib-1.0.0.jar");
    }

    #[test]
    fn test_maven_rel_path_with_classifier() {
        let path = maven_rel_path("org.lwjgl:lwjgl:3.2.1:natives-windows").unwrap();
        assert_eq!(
            path,
            "org/lwjgl/lwjgl/3.2.1/lwjgl-3.2.1-natives-windows.jar"
        );
    }

    #[test]
    fn test_format_uuid_32_chars() {
        let raw = "c1a5e5a5e5a5e5a5e5a5e5a5e5a5e5a5";
        let formatted = format_uuid(raw);
        assert_eq!(formatted.len(), 36);
        assert_eq!(formatted.chars().filter(|&c| c == '-').count(), 4);
    }

    #[test]
    fn test_format_uuid_short() {
        let raw = "short";
        let formatted = format_uuid(raw);
        assert_eq!(formatted, "short");
    }

    #[test]
    fn test_format_uuid_with_dashes() {
        let raw = "c1a5e5a5-e5a5-e5a5-e5a5-e5a5e5a5e5a5";
        let formatted = format_uuid(raw);
        assert_eq!(formatted, raw);
    }

    #[test]
    fn test_is_jar_intact_negative() {
        let path = std::path::Path::new("/nonexistent/file.jar");
        assert!(!is_jar_intact(path));
    }

    #[test]
    fn test_os_name() {
        let name = os_name();
        assert!(name == "windows" || name == "osx" || name == "linux");
    }

    #[test]
    fn test_os_arch() {
        let arch = os_arch();
        assert!(arch == "x86_64" || arch == "arm64" || arch == "unknown");
    }
    #[test]
    fn path_and_maven_validation() {
        for value in [
            "",
            ".",
            "..",
            "../outside",
            "a/b",
            "a\\b",
            "C:evil",
            "NUL",
            "a.",
            "a ",
        ] {
            assert!(validate_component(value).is_err(), "{value}");
        }
        for value in ["broken", "a:b", "a:b:../v", "a..b:c:d", "a:b:c:d:e"] {
            assert!(maven_rel_path(value).is_err());
        }
        assert_eq!(
            maven_rel_path("a.b:c:1:extra@zip").unwrap(),
            "a/b/c/1/c-1-extra.zip"
        );
        assert_eq!(
            offline_uuid("Steve"),
            "5627dd98-e6be-3c21-b8a8-e92344183641"
        );
        assert_eq!(
            format_uuid("中文中文中文中文中文ab"),
            "中文中文中文中文中文ab"
        );
    }
    #[test]
    fn missing_file_hash_and_partial_zip_fail() {
        let dir = tempfile::tempdir().unwrap();
        assert!(sha1_file(&dir.path().join("missing")).is_err());
        let path = dir.path().join("partial.jar");
        fs::write(&path, b"PK\x03\x04 this is not a zip archive").unwrap();
        assert!(!is_jar_intact(&path));
    }
}
