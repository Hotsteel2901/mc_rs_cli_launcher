//! Java detection and Mojang runtime download.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rayon::prelude::*;

use crate::http;
use crate::util;

const MOJANG_JAVA_MANIFEST: &str =
    "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// Spawning `java -version` costs 30–200 ms and a launcher asks for the same
/// answer several times per run (candidate scan, selection, logging). Keep the
/// answer in memory, and persist it next to the other verification records so
/// the second run does not spawn anything at all. A Java upgrade changes the
/// binary's size/mtime, which invalidates the record automatically.
const JAVA_PROBE_TTL: Duration = Duration::from_secs(30);
const JAVA_MARKER: &str = "!java:";

type MajorCache = Mutex<HashMap<String, Option<u32>>>;

fn process_cache() -> &'static MajorCache {
    static CACHE: OnceLock<MajorCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Get the Java major version number.
pub fn java_major(java_path: &str) -> Option<u32> {
    if let Ok(cache) = process_cache().lock() {
        if let Some(&value) = cache.get(java_path) {
            return value;
        }
    }
    if let Some(major) = crate::verify::lookup(Path::new(java_path))
        .as_deref()
        .and_then(|record| record.strip_prefix(JAVA_MARKER))
        .and_then(|value| value.parse::<u32>().ok())
    {
        if let Ok(mut cache) = process_cache().lock() {
            cache.insert(java_path.to_string(), Some(major));
        }
        return Some(major);
    }
    let value = probe_java_major(java_path);
    if let Ok(mut cache) = process_cache().lock() {
        cache.insert(java_path.to_string(), value);
    }
    if let Some(major) = value {
        crate::verify::record(Path::new(java_path), &format!("{JAVA_MARKER}{major}"));
    }
    value
}

fn probe_java_major(java_path: &str) -> Option<u32> {
    let output = Command::new(java_path).arg("-version").output().ok()?;

    if !output.status.success() {
        return None;
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    // Match "version "1.8.0_..." or "version "17.0.1"..."
    let re = java_version_regex();
    let cap = re.captures(&text)?;
    cap.get(1)?.as_str().parse().ok()
}

fn java_version_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r#"version "(?:1\.)?(\d+)"#).expect("static regex"))
}

/// Resolve relative executable paths before the game's working directory changes.
pub fn executable_path(path: &str) -> std::io::Result<String> {
    let p = Path::new(path);
    if p.is_relative() && p.components().count() > 1 {
        Ok(util::absolute_path(p)?.to_string_lossy().to_string())
    } else {
        Ok(path.to_string())
    }
}

/// Find all Java candidates on the system. Cached briefly: a picker or a
/// settings change must not rescan the filesystem on every key press, while an
/// installed JDK still shows up within half a minute.
type CandidateCache = Mutex<Option<(Instant, Vec<String>)>>;

pub fn find_java_candidates() -> Vec<String> {
    static CACHE: OnceLock<CandidateCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((at, list)) = guard.as_ref() {
            if at.elapsed() < JAVA_PROBE_TTL {
                return list.clone();
            }
        }
    }
    let list = scan_java_candidates();
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), list.clone()));
    }
    list
}

fn scan_java_candidates() -> Vec<String> {
    let java_bin = if cfg!(windows) { "java.exe" } else { "java" };
    let mut candidates: Vec<String> = Vec::new();

    // JAVA_HOME
    if let Ok(jh) = std::env::var("JAVA_HOME") {
        let je = Path::new(&jh).join("bin").join(java_bin);
        if je.exists() {
            candidates.push(je.to_string_lossy().to_string());
        }
    }

    // PATH
    if let Ok(path) = which::which("java") {
        candidates.push(path.to_string_lossy().to_string());
    }

    // Platform-specific scanning
    let mut scanned = scan_java_installations();
    candidates.append(&mut scanned);

    // Deduplicate by resolved path
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for c in candidates {
        let key = std::fs::canonicalize(&c)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| c.clone());
        if seen.insert(key) {
            result.push(c);
        }
    }
    result
}

#[cfg(windows)]
fn scan_java_installations() -> Vec<String> {
    let mut scanned = Vec::new();
    let java_exe = "java.exe";

    // Windows registry
    let hklm = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE);
    for key_path in &[
        r"SOFTWARE\Eclipse Adoptium\JDK",
        r"SOFTWARE\Eclipse Foundation\JDK",
        r"SOFTWARE\JavaSoft\JDK",
    ] {
        if let Ok(jdk_key) = hklm.open_subkey_with_flags(key_path, winreg::enums::KEY_READ) {
            for name in jdk_key.enum_keys().flatten() {
                if let Ok(sub_key) = jdk_key.open_subkey_with_flags(&name, winreg::enums::KEY_READ)
                {
                    if let Ok(jh) = sub_key.get_value::<String, _>("Path") {
                        let je = Path::new(&jh).join("bin").join(java_exe);
                        if je.exists() {
                            scanned.push(je.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    for key_path in &[r"SOFTWARE\Eclipse Adoptium\JDK", r"SOFTWARE\JavaSoft\JDK"] {
        if let Ok(jdk_key) = hkcu.open_subkey_with_flags(key_path, winreg::enums::KEY_READ) {
            for name in jdk_key.enum_keys().flatten() {
                if let Ok(sub_key) = jdk_key.open_subkey_with_flags(&name, winreg::enums::KEY_READ)
                {
                    if let Ok(jh) = sub_key.get_value::<String, _>("Path") {
                        let je = Path::new(&jh).join("bin").join(java_exe);
                        if je.exists() {
                            scanned.push(je.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    // User home JDK dirs
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let home = Path::new(&userprofile);
        if let Ok(entries) = fs::read_dir(home) {
            let mut jdk_dirs: Vec<_> = entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("jdk-"))
                .collect();
            jdk_dirs.sort_by_key(|e| e.file_name());
            jdk_dirs.reverse();
            for d in jdk_dirs {
                let je = d.path().join("bin").join(java_exe);
                if je.exists() {
                    scanned.push(je.to_string_lossy().to_string());
                }
            }
        }
    }

    // Common install dirs
    for base in &[
        r"C:\Program Files\Java",
        r"C:\Program Files\Eclipse Adoptium",
        r"C:\Program Files\Microsoft",
        r"C:\Program Files\Eclipse Foundation",
        r"C:\Program Files (x86)\Java",
        r"C:\Program Files\Zulu",
        r"C:\Program Files\Amazon Corretto",
        r"C:\Program Files\ojdkbuild",
        r"C:\tools\java",
    ] {
        let bp = Path::new(base);
        if bp.exists() {
            if let Ok(entries) = fs::read_dir(bp) {
                for d in entries.flatten() {
                    if d.path().is_dir() {
                        // Direct java in jdk/bin/
                        let je = d.path().join("bin").join(java_exe);
                        if je.exists() && !scanned.contains(&je.to_string_lossy().to_string()) {
                            scanned.push(je.to_string_lossy().to_string());
                        }
                        // Subdir check (nested JDKs)
                        if let Ok(sub) = fs::read_dir(d.path()) {
                            for sd in sub.flatten() {
                                if sd.path().is_dir() {
                                    let je2 = sd.path().join("bin").join(java_exe);
                                    if je2.exists()
                                        && !scanned.contains(&je2.to_string_lossy().to_string())
                                    {
                                        scanned.push(je2.to_string_lossy().to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    scanned
}

#[cfg(not(windows))]
fn scan_java_installations() -> Vec<String> {
    let mut scanned = Vec::new();
    let java_bin = "java";
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));

    for base in &["/usr/lib/jvm", "/usr/local/opt", "/opt/java"] {
        let bp = Path::new(base);
        if bp.exists() {
            if let Ok(entries) = fs::read_dir(bp) {
                let mut dirs: Vec<_> = entries.flatten().collect();
                dirs.sort_by_key(|e| e.file_name());
                dirs.reverse();
                for d in dirs {
                    let je = d.path().join("bin").join(java_bin);
                    if je.exists() {
                        scanned.push(je.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    // SDKMAN
    let sdkman = home.join(".sdkman/candidates/java");
    if sdkman.exists() {
        if let Ok(entries) = fs::read_dir(&sdkman) {
            let mut dirs: Vec<_> = entries.flatten().collect();
            dirs.sort_by_key(|e| e.file_name());
            dirs.reverse();
            for d in dirs {
                let je = d.path().join("bin").join(java_bin);
                if je.exists() {
                    scanned.push(je.to_string_lossy().to_string());
                }
            }
        }
    }

    // .jdks
    let jdks = home.join(".jdks");
    if jdks.exists() {
        if let Ok(entries) = fs::read_dir(&jdks) {
            let mut dirs: Vec<_> = entries.flatten().collect();
            dirs.sort_by_key(|e| e.file_name());
            dirs.reverse();
            for d in dirs {
                let je = d.path().join("bin").join(java_bin);
                if je.exists() {
                    scanned.push(je.to_string_lossy().to_string());
                }
            }
        }
    }

    // ASDF
    let asdf = home.join(".asdf/installs/java");
    if asdf.exists() {
        if let Ok(entries) = fs::read_dir(&asdf) {
            let mut dirs: Vec<_> = entries.flatten().collect();
            dirs.sort_by_key(|e| e.file_name());
            dirs.reverse();
            for d in dirs {
                let je = d.path().join("bin").join(java_bin);
                if je.exists() {
                    scanned.push(je.to_string_lossy().to_string());
                }
            }
        }
    }

    // Homebrew OpenJDK symlink
    let hb = Path::new("/usr/local/opt/openjdk/bin/java");
    if hb.exists() {
        scanned.push(hb.to_string_lossy().to_string());
    }

    scanned
}

/// Check for a suitable Java installation. Optionally require a specific major version.
/// Returns the path to the java executable.
pub fn check_java(required_major: Option<u32>) -> Option<String> {
    let candidates = find_java_candidates();

    if let Some(req) = required_major {
        for c in &candidates {
            if java_major(c) == Some(req) {
                return Some(c.clone());
            }
        }
        return None;
    }

    if candidates.is_empty() {
        return None;
    }

    let java = &candidates[0];
    let ver = java_major(java);
    if let Some(v) = ver {
        if v < 17 {
            crate::warn_msg!("Java {} detected. Minecraft 1.18+ needs Java 17+.", v);
            crate::info!("Found at: {}", java);
        }
    }
    Some(java.clone())
}

/// Get the platform key for Mojang's Java runtime manifest.
fn mojang_java_platform() -> Option<&'static str> {
    match (util::os_name(), util::os_arch()) {
        ("windows", "arm64") => Some("windows-arm64"),
        ("windows", _) => Some("windows-x64"),
        ("osx", "arm64") => Some("mac-os-arm64"),
        ("osx", _) => Some("mac-os"),
        ("linux", "x86_64") => Some("linux"),
        _ => None,
    }
}

/// Download Mojang's bundled Java runtime. Returns path to java executable.
pub fn download_mojang_java(
    game_dir: &Path,
    component: &str,
    max_workers: usize,
) -> Option<String> {
    util::validate_component(component).ok()?;
    let dest_root = util::absolute_path(&game_dir.join("java").join(component)).ok()?;
    let exe_name = if cfg!(windows) { "java.exe" } else { "java" };

    // Check if already downloaded
    let existing = find_java_in_dir(&dest_root, exe_name);
    if let Some(path) = existing {
        return Some(path);
    }

    let plat = mojang_java_platform()?;

    // Fetch platform manifest
    let (status, body) = http::http_get(MOJANG_JAVA_MANIFEST).ok()?;
    if status != 200 {
        crate::warn_msg!("Mojang Java runtime index fetch failed ({})", status);
        return None;
    }

    let entries: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let platform_entries = entries[plat][component].as_array()?;
    let first = platform_entries.first()?;
    let ver_name = first["version"]["name"].as_str().unwrap_or("?");

    // Fetch component manifest
    let manifest_url = first["manifest"]["url"].as_str()?;
    let (status2, body2) = http::http_get(manifest_url).ok()?;
    if status2 != 200 {
        crate::warn_msg!("Mojang Java runtime manifest fetch failed ({})", status2);
        return None;
    }

    let files_data: serde_json::Value = serde_json::from_slice(&body2).ok()?;
    let files = files_data["files"].as_object()?;

    // Collect downloads
    let mut downloads: Vec<(String, PathBuf, Option<String>)> = Vec::new();
    let mut executables: Vec<PathBuf> = Vec::new();
    let mut links: Vec<(PathBuf, String)> = Vec::new();

    for (rel, info) in files {
        let target = crate::util::safe_join(&dest_root, rel).ok()?;
        match info["type"].as_str() {
            Some("directory") => {
                fs::create_dir_all(&target).ok()?;
            }
            Some("file") => {
                let raw = &info["downloads"]["raw"];
                let url = raw["url"].as_str();
                let sha1 = raw["sha1"].as_str();
                if let Some(url) = url {
                    let need_dl = match (target.exists(), sha1) {
                        (true, Some(expected)) => !crate::util::sha1_file(&target)
                            .is_ok_and(|hash| hash.eq_ignore_ascii_case(expected)),
                        (true, None) => false,
                        (false, _) => true,
                    };
                    if need_dl {
                        downloads.push((url.to_string(), target.clone(), sha1.map(String::from)));
                    }
                }
                if info["executable"].as_bool().unwrap_or(false) {
                    executables.push(target);
                }
            }
            Some("link") => {
                if let Some(link_target) = info["target"].as_str() {
                    links.push((target, link_target.to_string()));
                }
            }
            _ => {}
        }
    }

    crate::info!(
        "Downloading Java runtime {} ({}): {} files [{} threads]...",
        ver_name,
        component,
        downloads.len(),
        max_workers
    );

    let total = downloads.len();
    let completed = std::sync::atomic::AtomicUsize::new(0);
    let fail = std::sync::atomic::AtomicUsize::new(0);
    let workers = max_workers.clamp(1, 64);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .ok()?;

    pool.install(|| {
        downloads.par_iter().for_each(|(url, dest, sha1)| {
            if http::download_file(url, dest, "", sha1.as_deref(), 2, false).is_err() {
                fail.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let n = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            // Redraw every tenth file (and the last one) instead of for every
            // completion, so the progress line does not become the bottleneck.
            if n == total || n.is_multiple_of(10) {
                let pct = n * 100 / total.max(1);
                let filled = pct * 25 / 100;
                let bar: String = (0..25)
                    .map(|i| if i < filled { '\u{2588}' } else { '\u{2591}' })
                    .collect();
                print!("\r  [{:3}%] {} {}/{} files", pct, bar, n, total);
                use std::io::Write;
                std::io::stdout().flush().ok();
            }
        });
    });
    println!();

    let fails = fail.load(std::sync::atomic::Ordering::Relaxed);
    if fails > 0 {
        crate::warn_msg!("{} Java runtime file(s) failed to download.", fails);
        return None;
    }

    // Set executable permissions on non-Windows
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        for t in &executables {
            if let Ok(meta) = fs::metadata(t) {
                let mut perms = meta.permissions();
                perms.set_mode(perms.mode() | 0o755);
                fs::set_permissions(t, perms).ok()?;
            }
        }
        for (target, link_target) in &links {
            let parent = target.parent()?;
            if Path::new(link_target).is_absolute()
                || !util::absolute_path(&parent.join(link_target))
                    .ok()?
                    .starts_with(&dest_root)
            {
                crate::warn_msg!("Java manifest contains a link outside the runtime directory.");
                return None;
            }
            if !target.exists() {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).ok()?;
                }
                std::os::unix::fs::symlink(link_target, target).ok()?;
            }
        }
    }

    let java_bins = find_java_in_dir(&dest_root, exe_name);
    if java_bins.is_none() {
        crate::warn_msg!("Java runtime downloaded but java executable not found.");
        return None;
    }
    crate::success!(
        "Java runtime {} installed -> {}",
        ver_name,
        dest_root.display()
    );
    java_bins
}

fn find_java_in_dir(root: &Path, exe_name: &str) -> Option<String> {
    // Game directories may contain glob metacharacters such as `[` or `*`.
    let pattern = format!(
        "{}/**/bin/{}",
        glob::Pattern::escape(&root.to_string_lossy()),
        exe_name
    );
    let matches = glob::glob(&pattern).ok()?;
    if let Some(entry) = matches.flatten().next() {
        return Some(entry.to_string_lossy().to_string());
    }
    None
}

// ─── Java discovery ────────────────────────────────────────────────

/// Where a discovered runtime came from; shown in pickers and logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JavaOrigin {
    JavaHome,
    Path,
    Scan,
    Managed,
}

impl JavaOrigin {
    pub fn label(self) -> &'static str {
        match self {
            JavaOrigin::JavaHome => "JAVA_HOME",
            JavaOrigin::Path => "PATH",
            JavaOrigin::Scan => "scan",
            JavaOrigin::Managed => "runtime",
        }
    }
}

/// A discovered Java runtime with the metadata a launcher needs to explain
/// "which Java should I use for this version?".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaInfo {
    pub path: String,
    pub major: Option<u32>,
    pub version: String,
    pub vendor: String,
    pub arch: String,
    pub origin: JavaOrigin,
}

impl JavaInfo {
    /// "Java 21 · Temurin" or "Java ? · unknown".
    pub fn label(&self) -> String {
        let major = self
            .major
            .map(|m| m.to_string())
            .unwrap_or_else(|| "?".into());
        format!("Java {major} · {}", self.vendor)
    }

    /// Compact metadata for a picker row.
    pub fn meta(&self) -> String {
        let mut parts = Vec::new();
        if !self.version.is_empty() {
            parts.push(self.version.clone());
        }
        if !self.arch.is_empty() {
            parts.push(self.arch.clone());
        }
        parts.push(self.origin.label().to_string());
        parts.join(" · ")
    }
}

/// Parse an OpenJDK release file (KEY="value" per line). Values may be
/// quoted or bare; comments and blank lines are ignored.
pub fn parse_release_file(text: &str) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        if !key.trim().is_empty() {
            map.insert(key.trim().to_string(), value.to_string());
        }
    }
    map
}

/// "17.0.8" -> 17, "1.8.0_402" -> 8, "21-ea" -> 21.
pub fn major_from_version(version: &str) -> Option<u32> {
    let version = version.trim();
    let stripped = version.strip_prefix("1.").unwrap_or(version);
    let digits: String = stripped
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn java_home_of(exe: &str) -> Option<PathBuf> {
    let path = Path::new(exe);
    let bin = path.parent()?;
    let home = bin.parent()?;
    Some(home.to_path_buf())
}

fn read_release_file(home: &Path) -> Option<std::collections::BTreeMap<String, String>> {
    let text = fs::read_to_string(home.join("release")).ok()?;
    Some(parse_release_file(&text))
}

/// The three fields we can recover from java -version output.
pub struct JavaProbe {
    pub major: Option<u32>,
    pub version: String,
    pub vendor: String,
}

/// Parse java -version output, which writes to stderr on most JDKs:
/// "openjdk version \"17.0.8\" 2023-07-18" / "java version \"1.8.0_402\"".
pub fn parse_java_version_output(text: &str) -> Option<JavaProbe> {
    let re = regex::Regex::new("(?m)^\\s*(\\S+)\\s+version\\s+\"([^\"]+)\"").ok()?;
    let caps = re.captures(text)?;
    let vendor = caps.get(1).map(|m| m.as_str()).unwrap_or("Java");
    let vendor = match vendor {
        "openjdk" => "OpenJDK",
        "java" => "Java",
        other => other,
    };
    let version = caps
        .get(2)
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();
    Some(JavaProbe {
        major: major_from_version(&version),
        version,
        vendor: vendor.to_string(),
    })
}

/// Inspect one executable, preferring the cheap release file in the JDK home
/// and falling back to spawning java -version.
pub fn inspect_java(path: &str, origin: JavaOrigin) -> JavaInfo {
    let mut info = JavaInfo {
        path: path.to_string(),
        major: None,
        version: String::new(),
        vendor: String::new(),
        arch: String::new(),
        origin,
    };
    if let Some(release) = java_home_of(path).as_deref().and_then(read_release_file) {
        info.version = release
            .get("JAVA_VERSION")
            .or_else(|| release.get("JAVA_RUNTIME_VERSION"))
            .cloned()
            .unwrap_or_default();
        info.vendor = release
            .get("IMPLEMENTOR")
            .or_else(|| release.get("JAVA_VENDOR"))
            .cloned()
            .unwrap_or_default();
        info.arch = release.get("OS_ARCH").cloned().unwrap_or_default();
        info.major = major_from_version(&info.version);
    }
    if info.major.is_none() {
        if let Ok(output) = Command::new(path).arg("-version").output() {
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
            if let Some(probe) = parse_java_version_output(&text) {
                info.major = probe.major;
                if info.version.is_empty() {
                    info.version = probe.version;
                }
                if info.vendor.is_empty() {
                    info.vendor = probe.vendor;
                }
            }
        }
    }
    if info.vendor.is_empty() {
        info.vendor = "Java".into();
    }
    info
}

/// Managed runtimes live under <game_dir>/java/<component>/.../bin/java.
pub fn scan_managed_javas(game_dir: &Path) -> Vec<String> {
    let exe_name = if cfg!(windows) { "java.exe" } else { "java" };
    let pattern = format!(
        "{}/java/**/bin/{}",
        glob::Pattern::escape(&game_dir.to_string_lossy()),
        exe_name
    );
    glob::glob(&pattern)
        .map(|entries| {
            entries
                .flatten()
                .filter(|p| p.is_file())
                .map(|p| p.to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// All usable Java runtimes, best first (highest major, managed before system).
pub fn detect_java(game_dir: Option<&Path>) -> Vec<JavaInfo> {
    let mut candidates: Vec<(String, JavaOrigin)> = Vec::new();
    if let Ok(java_home) = std::env::var("JAVA_HOME") {
        let exe = if cfg!(windows) { "java.exe" } else { "java" };
        let path = Path::new(&java_home).join("bin").join(exe);
        if path.is_file() {
            candidates.push((path.to_string_lossy().to_string(), JavaOrigin::JavaHome));
        }
    }
    if let Ok(path) = which::which("java") {
        candidates.push((path.to_string_lossy().to_string(), JavaOrigin::Path));
    }
    for path in scan_java_installations() {
        candidates.push((path, JavaOrigin::Scan));
    }
    if let Some(game_dir) = game_dir {
        for path in scan_managed_javas(game_dir) {
            candidates.push((path, JavaOrigin::Managed));
        }
    }

    let mut seen = std::collections::HashSet::new();
    let mut infos: Vec<JavaInfo> = Vec::new();
    for (path, origin) in candidates {
        let key = fs::canonicalize(&path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.clone());
        if !seen.insert(key) {
            continue;
        }
        infos.push(inspect_java(&path, origin));
    }
    sort_java(&mut infos);
    infos
}

fn sort_java(infos: &mut [JavaInfo]) {
    infos.sort_by(|a, b| {
        b.major
            .unwrap_or(0)
            .cmp(&a.major.unwrap_or(0))
            .then_with(|| (b.origin == JavaOrigin::Managed).cmp(&(a.origin == JavaOrigin::Managed)))
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// Pick the runtime that should run a version needing the given major.
/// Exact major wins; then the lowest newer major; then the highest older one.
pub fn best_java(infos: &[JavaInfo], required: Option<u32>) -> Option<&JavaInfo> {
    let Some(required) = required else {
        return infos.first();
    };
    infos
        .iter()
        .find(|info| info.major == Some(required))
        .or_else(|| {
            infos
                .iter()
                .filter(|info| info.major.is_some_and(|major| major > required))
                .min_by_key(|info| info.major)
        })
        .or_else(|| {
            infos
                .iter()
                .filter(|info| info.major.is_some_and(|major| major < required))
                .max_by_key(|info| info.major)
        })
}

/// The managed runtime of a given major, if it was installed earlier.
pub fn managed_java(game_dir: &Path, major: u32) -> Option<String> {
    scan_managed_javas(game_dir).into_iter().find(|path| {
        java_home_of(path)
            .as_deref()
            .and_then(read_release_file)
            .and_then(|release| {
                release
                    .get("JAVA_VERSION")
                    .and_then(|v| major_from_version(v))
            })
            .or_else(|| java_major(path))
            == Some(major)
    })
}

/// Fetch Mojang's runtime index (all.json).
pub fn fetch_java_manifest() -> Result<serde_json::Value, String> {
    let (status, body) = http::http_get(MOJANG_JAVA_MANIFEST)
        .map_err(|e| format!("Java runtime index is unavailable: {e}"))?;
    if status != 200 {
        return Err(format!("Java runtime index returned HTTP {status}"));
    }
    serde_json::from_slice(&body).map_err(|e| format!("Invalid Java runtime index: {e}"))
}

/// Choose the Mojang component that provides the requested major on this
/// platform. Returns (component, version name).
pub fn component_for_major(
    manifest: &serde_json::Value,
    platform: &str,
    major: u32,
) -> Option<(String, String)> {
    let components = manifest.get(platform)?.as_object()?;
    let mut best: Option<(String, String)> = None;
    for (name, entries) in components {
        for entry in entries.as_array().into_iter().flatten() {
            let Some(version_name) = entry["version"]["name"].as_str() else {
                continue;
            };
            if major_from_version(version_name) != Some(major) {
                continue;
            }
            if entry["manifest"]["url"].as_str().is_none() {
                continue;
            }
            let better = match &best {
                Some((_, current)) => compare_versions(version_name, current).is_gt(),
                None => true,
            };
            if better {
                best = Some((name.clone(), version_name.to_string()));
            }
        }
    }
    best
}

fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |value: &str| {
        value
            .split(|c: char| !c.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<u32>().ok())
            .collect::<Vec<_>>()
    };
    split(a).cmp(&split(b))
}

/// Download (or reuse) Mojang's Java runtime for a major version.
/// This is the install-java path: the component covering the major on this
/// platform is selected from Mojang's index, then installed under
/// <game_dir>/java/<component>.
pub fn download_runtime_for_major(
    game_dir: &Path,
    major: u32,
    max_workers: usize,
) -> Option<String> {
    if let Some(path) = managed_java(game_dir, major) {
        crate::info!("Java {} runtime already installed: {}", major, path);
        return Some(path);
    }
    let platform = mojang_java_platform()?;
    let manifest = fetch_java_manifest().ok()?;
    let (component, version_name) = component_for_major(&manifest, platform, major)?;
    crate::info!(
        "Mojang runtime for Java {}: {} ({}, {})",
        major,
        component,
        version_name,
        platform
    );
    let path = download_mojang_java(game_dir, &component, max_workers)?;
    if let Some(actual) = java_major(&path) {
        if actual != major {
            crate::warn_msg!(
                "Downloaded runtime reports Java {}, expected Java {}.",
                actual,
                major
            );
        }
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_lookup_survives_glob_metacharacters_in_the_game_dir() {
        let dir = tempfile::tempdir().unwrap();
        // `[` and `*` are legal in directory names but special to glob.
        let runtime = dir.path().join("mc[1]*x").join("java").join("gamma");
        let bin = runtime.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let java = bin.join("java");
        std::fs::write(&java, b"fixture").unwrap();
        assert_eq!(
            find_java_in_dir(
                dir.path()
                    .join("mc[1]*x")
                    .join("java")
                    .join("gamma")
                    .as_path(),
                "java"
            )
            .map(|p| Path::new(&p).to_path_buf()),
            Some(java)
        );
        assert!(find_java_in_dir(&dir.path().join("missing[1]"), "java").is_none());
    }

    #[test]
    fn release_files_and_version_strings_are_parsed() {
        let release = parse_release_file(
            "# comment\nIMPLEMENTOR=\"Eclipse Adoptium\"\nJAVA_VERSION=\"17.0.8\"\nOS_ARCH=\"x86_64\"\nBARE=value\n",
        );
        assert_eq!(release.get("IMPLEMENTOR").unwrap(), "Eclipse Adoptium");
        assert_eq!(release.get("JAVA_VERSION").unwrap(), "17.0.8");
        assert_eq!(release.get("BARE").unwrap(), "value");
        assert_eq!(major_from_version("17.0.8"), Some(17));
        assert_eq!(major_from_version("1.8.0_402"), Some(8));
        assert_eq!(major_from_version("21-ea"), Some(21));
        assert_eq!(major_from_version(""), None);
        assert_eq!(major_from_version("ea"), None);
    }

    #[test]
    fn java_version_output_is_parsed() {
        let probe =
            parse_java_version_output("openjdk version \"17.0.8\" 2023-07-18\nOpenJDK Runtime")
                .unwrap();
        assert_eq!(probe.major, Some(17));
        assert_eq!(probe.version, "17.0.8");
        assert_eq!(probe.vendor, "OpenJDK");

        let legacy = parse_java_version_output("java version \"1.8.0_402\"\nJava(TM) SE").unwrap();
        assert_eq!(legacy.major, Some(8));
        assert_eq!(legacy.vendor, "Java");
        assert!(parse_java_version_output("no version here").is_none());
    }

    #[test]
    fn managed_runtime_is_found_without_launching_java() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("java").join("java-runtime-gamma");
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join(if cfg!(windows) { "java.exe" } else { "java" });
        std::fs::write(&exe, b"fixture").unwrap();
        std::fs::write(
            home.join("release"),
            b"IMPLEMENTOR=\"Eclipse Adoptium\"\nJAVA_VERSION=\"17.0.8\"\nOS_ARCH=\"x86_64\"\n",
        )
        .unwrap();
        let found = scan_managed_javas(dir.path());
        assert_eq!(found.len(), 1);
        let info = inspect_java(&found[0], JavaOrigin::Managed);
        assert_eq!(info.major, Some(17));
        assert_eq!(info.vendor, "Eclipse Adoptium");
        assert_eq!(info.arch, "x86_64");
        assert!(info.label().contains("Java 17"));
        assert_eq!(
            managed_java(dir.path(), 17).map(|p| Path::new(&p).to_path_buf()),
            Some(exe)
        );
        assert!(managed_java(dir.path(), 21).is_none());
    }

    #[test]
    fn mojang_component_selection_prefers_the_newest_matching_build() {
        let manifest = serde_json::json!({
            "linux": {
                "jre-legacy": [{"version": {"name": "8u422"}, "manifest": {"url": "u8"}}],
                "java-runtime-alpha": [{"version": {"name": "16.0.1"}, "manifest": {"url": "u16"}}],
                "java-runtime-beta": [{"version": {"name": "17.0.1"}, "manifest": {"url": "u17a"}}],
                "java-runtime-gamma": [{"version": {"name": "17.0.8"}, "manifest": {"url": "u17b"}}],
                "java-runtime-delta": [{"version": {"name": "21.0.3"}, "manifest": {"url": "u21"}}]
            },
            "windows-x64": {
                "java-runtime-gamma": [{"version": {"name": "17.0.8"}, "manifest": {"url": "uw"}}]
            }
        });
        assert_eq!(
            component_for_major(&manifest, "linux", 17),
            Some(("java-runtime-gamma".into(), "17.0.8".into()))
        );
        assert_eq!(
            component_for_major(&manifest, "linux", 8).map(|(c, _)| c),
            Some("jre-legacy".into())
        );
        assert_eq!(
            component_for_major(&manifest, "linux", 21).map(|(c, _)| c),
            Some("java-runtime-delta".into())
        );
        assert!(component_for_major(&manifest, "linux", 11).is_none());
        assert!(component_for_major(&manifest, "solaris", 17).is_none());
    }

    #[test]
    fn best_java_prefers_exact_then_nearest_newer() {
        let info = |major: u32, origin: JavaOrigin| JavaInfo {
            path: format!("/jdk-{major}/bin/java"),
            major: Some(major),
            version: format!("{major}.0.0"),
            vendor: "Test".into(),
            arch: "x86_64".into(),
            origin,
        };
        let infos = vec![
            info(8, JavaOrigin::Scan),
            info(17, JavaOrigin::Path),
            info(21, JavaOrigin::Managed),
        ];
        assert_eq!(best_java(&infos, Some(17)).unwrap().major, Some(17));
        assert_eq!(best_java(&infos, Some(11)).unwrap().major, Some(17));
        assert_eq!(best_java(&infos, Some(25)).unwrap().major, Some(21));
        assert_eq!(best_java(&infos, None).unwrap().major, Some(8));
        assert!(best_java(&[], Some(17)).is_none());
    }
}
