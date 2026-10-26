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

/// Limits applied while unpacking an archive that came from the network or a
/// user-supplied file. A few hundred kilobytes of ZIP can otherwise expand into
/// terabytes (zip bomb) or hundreds of thousands of inodes.
#[derive(Clone, Copy, Debug)]
pub struct ZipLimits {
    pub max_entries: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for ZipLimits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            // A single Minecraft asset or mod is never this large; the client
            // jar is ~25 MB and the biggest modpacks stay well below 512 MB.
            max_file_bytes: 512 * 1024 * 1024,
            max_total_bytes: 16 * 1024 * 1024 * 1024,
        }
    }
}

/// Tracks how much one archive has already expanded to.
pub struct ZipBudget {
    limits: ZipLimits,
    entries: usize,
    total: u64,
}

impl ZipBudget {
    pub fn new(limits: ZipLimits) -> Self {
        Self {
            limits,
            entries: 0,
            total: 0,
        }
    }

    /// Reject an entry using its declared size before any bytes are written.
    pub fn admit(&mut self, entry: &zip::read::ZipFile<'_>) -> Result<(), String> {
        if entry.is_dir() {
            return Ok(());
        }
        self.entries += 1;
        if self.entries > self.limits.max_entries {
            return Err(format!(
                "Archive has more than {} entries; refusing to extract",
                self.limits.max_entries
            ));
        }
        if entry.size() > self.limits.max_file_bytes {
            return Err(format!(
                "Archive entry '{}' declares {} bytes, above the {} byte limit",
                entry.name(),
                entry.size(),
                self.limits.max_file_bytes
            ));
        }
        if self.total.saturating_add(entry.size()) > self.limits.max_total_bytes {
            return Err(format!(
                "Archive expands beyond the {} byte limit; refusing to extract",
                self.limits.max_total_bytes
            ));
        }
        Ok(())
    }

    /// Account for bytes actually written, which may exceed the declared size.
    pub fn charge(&mut self, bytes: u64) -> Result<(), String> {
        if bytes > self.limits.max_file_bytes {
            return Err(format!(
                "Archive entry exceeded the {} byte per-file limit",
                self.limits.max_file_bytes
            ));
        }
        self.total = self.total.saturating_add(bytes);
        if self.total > self.limits.max_total_bytes {
            return Err(format!(
                "Archive expands beyond the {} byte limit; refusing to extract",
                self.limits.max_total_bytes
            ));
        }
        Ok(())
    }

    fn per_file_cap(&self) -> u64 {
        self.limits.max_file_bytes
    }
}

/// ZIP entries use `/`; some Windows tools emit `\`. Normalise before joining
/// so valid archives still work, while `..`, absolute paths and drive letters
/// remain rejected by [`safe_join`].
pub fn zip_relative_path(name: &str) -> Result<String, String> {
    let normalised = name.replace('\\', "/");
    let normalised = normalised.strip_prefix("./").unwrap_or(&normalised);
    if normalised.is_empty() {
        return Err("Empty archive path".into());
    }
    if normalised.starts_with('/') {
        return Err(format!("Absolute path in archive: {name:?}"));
    }
    let bytes = normalised.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(format!("Drive-qualified path in archive: {name:?}"));
    }
    Ok(normalised.to_string())
}

pub fn zip_entry_is_symlink(entry: &zip::read::ZipFile<'_>) -> bool {
    entry
        .unix_mode()
        .is_some_and(|mode| mode & 0o170000 == 0o120000)
}

/// Stream one entry to `dest` through a private temporary file, enforcing the
/// per-file cap even when the archive lies about the declared size.
pub fn extract_zip_entry(
    reader: &mut impl Read,
    dest: &Path,
    budget: &mut ZipBudget,
) -> Result<u64, String> {
    let parent = dest
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
    let cap = budget.per_file_cap();
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "Cannot create a temporary file in {}: {error}",
            parent.display()
        )
    })?;
    let copied = std::io::copy(&mut reader.take(cap.saturating_add(1)), &mut temp)
        .map_err(|error| format!("Cannot extract {}: {error}", dest.display()))?;
    budget.charge(copied)?;
    temp.persist(dest)
        .map_err(|error| format!("Cannot publish {}: {}", dest.display(), error.error))?;
    Ok(copied)
}

/// Extract every entry selected by `map` under `root`. `map` receives the raw
/// archive name and returns the destination-relative path, or `None` to skip.
pub fn extract_zip_entries<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    root: &Path,
    limits: ZipLimits,
    mut map: impl FnMut(&str) -> Option<String>,
) -> Result<usize, String> {
    let mut budget = ZipBudget::new(limits);
    let mut written = 0;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Corrupt archive entry #{index}: {error}"))?;
        if entry.is_dir() {
            continue;
        }
        if zip_entry_is_symlink(&entry) {
            return Err(format!(
                "Archive contains a symlink ({}); refusing to extract",
                entry.name()
            ));
        }
        let Some(relative) = map(entry.name()) else {
            continue;
        };
        budget.admit(&entry)?;
        let relative = zip_relative_path(&relative)?;
        let dest = safe_join(root, &relative)?;
        extract_zip_entry(&mut entry, &dest, &mut budget)?;
        written += 1;
    }
    Ok(written)
}

/// Append `-2`, `-3`, … before the extension until the path is free. Used for
/// archives whose default name only has second precision.
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or(Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("backup");
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 2..10_000u32 {
        let name = match extension {
            Some(ext) => format!("{stem}-{index}.{ext}"),
            None => format!("{stem}-{index}"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
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

    #[test]
    fn zip_names_are_normalised_and_confined() {
        assert_eq!(zip_relative_path("a\\b\\c.txt").unwrap(), "a/b/c.txt");
        assert_eq!(zip_relative_path("./a/b").unwrap(), "a/b");
        for bad in ["/etc/passwd", "C:/Windows/x", "C:x", ""] {
            assert!(zip_relative_path(bad).is_err(), "{bad}");
        }
        // Normalisation must not launder traversal: safe_join still rejects it.
        assert!(safe_join(Path::new("/tmp/out"), &zip_relative_path("../x").unwrap()).is_err());
    }

    #[test]
    fn archive_extraction_rejects_traversal_and_oversized_entries() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let write_zip = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(format!("{}.zip", name.replace('/', "_")));
            let mut writer = zip::ZipWriter::new(fs::File::create(&path).unwrap());
            writer
                .start_file(name, zip::write::FileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
            writer.finish().unwrap();
            path
        };
        let out = dir.path().join("out");

        let archive = write_zip("../escape", b"x");
        let mut handle = zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
        let error = extract_zip_entries(&mut handle, &out, ZipLimits::default(), |name| {
            Some(name.to_string())
        })
        .unwrap_err();
        assert!(error.contains("Unsafe"), "{error}");
        assert!(!dir.path().join("escape").exists());

        let archive = write_zip("big", b"12345");
        let mut handle = zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
        let limits = ZipLimits {
            max_entries: 10,
            max_file_bytes: 4,
            max_total_bytes: 100,
        };
        let error = extract_zip_entries(&mut handle, &out, limits, |name| Some(name.to_string()))
            .unwrap_err();
        assert!(error.contains("limit"), "{error}");
        assert!(!out.join("big").exists());
    }

    #[test]
    fn unique_path_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("save.zip");
        assert_eq!(unique_path(&first), first);
        fs::write(&first, b"1").unwrap();
        let second = unique_path(&first);
        assert_eq!(second, dir.path().join("save-2.zip"));
        fs::write(&second, b"2").unwrap();
        assert_eq!(unique_path(&first), dir.path().join("save-3.zip"));
    }
}
