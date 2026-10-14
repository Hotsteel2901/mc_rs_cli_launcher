//! HTTP transport and transactional, verified downloads.
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use crate::util;
use indicatif::ProgressBar;

const USER_AGENT: &str = concat!("mc-launcher/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceMode {
    /// Official first, mirror fallback for public GET requests only.
    Auto = 0,
    Official = 1,
    BmclApi = 2,
}
static SOURCE_MODE: AtomicU8 = AtomicU8::new(0);

pub fn set_source_mode(mode: SourceMode) {
    SOURCE_MODE.store(mode as u8, Ordering::Relaxed);
}
fn get_source_mode() -> SourceMode {
    match SOURCE_MODE.load(Ordering::Relaxed) {
        1 => SourceMode::Official,
        2 => SourceMode::BmclApi,
        _ => SourceMode::Auto,
    }
}

/// No ICMP/geolocation probe: local commands must work without a network.
pub fn resolve_source_mode(manual: Option<SourceMode>) -> SourceMode {
    manual.unwrap_or(SourceMode::Auto)
}

/// Match the parsed origin, never arbitrary substrings or look-alike domains.
fn mirror_url(original: &str) -> String {
    let Ok(mut url) = url::Url::parse(original) else {
        return original.into();
    };
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return original.into();
    }
    let prefix = match url.host_str().unwrap_or("") {
        "launchermeta.mojang.com"
        | "launcher.mojang.com"
        | "piston-meta.mojang.com"
        | "piston-data.mojang.com" => "",
        "resources.download.minecraft.net" => "/assets",
        "libraries.minecraft.net" | "maven.minecraftforge.net" | "maven.fabricmc.net" => "/maven",
        "meta.fabricmc.net" => "/fabric-meta",
        // Unsupported mappings (notably NeoForge) stay on their official origin.
        _ => return original.into(),
    };
    let path = format!("{prefix}{}", url.path());
    if url.set_host(Some("bmclapi2.bangbang93.com")).is_err() {
        return original.into();
    }
    url.set_path(&path);
    url.into()
}

fn source_urls(url: &str, mode: SourceMode) -> Vec<String> {
    let mirrored = mirror_url(url);
    match mode {
        SourceMode::Official => vec![url.into()],
        SourceMode::BmclApi => vec![mirrored],
        SourceMode::Auto if mirrored != url => vec![url.into(), mirrored],
        SourceMode::Auto => vec![url.into()],
    }
}

pub type HttpResult = Result<(u16, Vec<u8>), String>;

pub fn http_request(
    method: &str,
    url: &str,
    body_data: Option<&[u8]>,
    json_data: Option<&serde_json::Value>,
    extra_headers: Option<&[(&str, &str)]>,
    timeout_secs: u64,
    max_retries: u32,
) -> HttpResult {
    // Never forward authenticated traffic or POST bodies to a mirror.
    let public_get =
        method == "GET" && extra_headers.is_none() && body_data.is_none() && json_data.is_none();
    let urls = if public_get {
        source_urls(url, get_source_mode())
    } else {
        vec![url.into()]
    };
    let mut result = Err("No source available".into());
    for target in urls {
        result = http_request_no_mirror(
            method,
            &target,
            body_data,
            json_data,
            extra_headers,
            timeout_secs,
            max_retries,
        );
        if matches!(&result, Ok((200..=299, _))) {
            break;
        }
    }
    result
}

pub fn http_request_no_mirror(
    method: &str,
    url: &str,
    body_data: Option<&[u8]>,
    json_data: Option<&serde_json::Value>,
    extra_headers: Option<&[(&str, &str)]>,
    timeout_secs: u64,
    max_retries: u32,
) -> HttpResult {
    if max_retries == 0 {
        return Err("Request attempts must be greater than zero".into());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_secs))
        .build();
    let mut last = Err("Request failed".into());
    for attempt in 0..max_retries {
        let mut request = agent.request(method, url).set("User-Agent", USER_AGENT);
        if let Some(headers) = extra_headers {
            for (key, value) in headers {
                request = request.set(key, value);
            }
        }
        let response = if let Some(json) = json_data {
            request.send_json(json.clone())
        } else if let Some(data) = body_data {
            request
                .set("Content-Type", "application/x-www-form-urlencoded")
                .send_bytes(data)
        } else {
            request.call()
        };
        last = match response {
            Ok(response) | Err(ureq::Error::Status(_, response)) => {
                let status = response.status();
                let mut body = Vec::new();
                match response.into_reader().read_to_end(&mut body) {
                    Ok(_) => Ok((status, body)),
                    Err(error) => Err(format!("Response read failed: {error}")),
                }
            }
            Err(error) => Err(error.to_string()),
        };
        let retry = match &last {
            Ok((status, _)) => *status == 429 || *status == 408 || *status >= 500,
            Err(_) => true,
        };
        if !retry {
            return last;
        }
        if attempt + 1 < max_retries {
            std::thread::sleep(Duration::from_secs(1 << attempt.min(4)));
        }
    }
    // Preserve the actual status and body (OAuth callers need error bodies).
    last
}

pub fn http_get(url: &str) -> HttpResult {
    http_request("GET", url, None, None, None, 30, 3)
}
pub fn http_get_hdrs(url: &str, headers: &[(&str, &str)]) -> HttpResult {
    http_request_no_mirror("GET", url, None, None, Some(headers), 30, 3)
}
pub fn http_post_json(url: &str, json: &serde_json::Value) -> HttpResult {
    http_request_no_mirror("POST", url, None, Some(json), None, 30, 3)
}
pub fn http_post_form(url: &str, data: &[u8]) -> HttpResult {
    http_request_no_mirror("POST", url, Some(data), None, None, 30, 3)
}

pub fn cached_file_valid(path: &Path, sha1: Option<&str>) -> bool {
    if let Some(hash) = sha1 {
        return util::sha1_file(path).is_ok_and(|actual| actual.eq_ignore_ascii_case(hash));
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some("jar" | "zip") => util::is_jar_intact(path),
        Some("json") => fs::read(path)
            .ok()
            .is_some_and(|data| serde_json::from_slice::<serde_json::Value>(&data).is_ok()),
        // No hash/format information: do not trust arbitrary pre-existing bytes.
        _ => false,
    }
}

/// A failed download never replaces an existing file; temporary files are removed on drop.
pub fn download_file(
    url: &str,
    dest: &Path,
    label: &str,
    sha1_expected: Option<&str>,
    max_retries: u32,
    show_progress: bool,
) -> Result<(), String> {
    if max_retries == 0 {
        return Err("Download attempts must be greater than zero".into());
    }
    if let Some(hash) = sha1_expected {
        if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid SHA-1 in download metadata".into());
        }
    }
    if cached_file_valid(dest, sha1_expected) {
        return Ok(());
    }
    let parent = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    let mut last_error = String::new();
    for source in source_urls(url, get_source_mode()) {
        for attempt in 0..max_retries {
            match download_once(&source, dest, label, sha1_expected, show_progress) {
                Ok(()) => return Ok(()),
                Err(error) => last_error = error,
            }
            if attempt + 1 < max_retries {
                std::thread::sleep(Duration::from_secs(1 << attempt.min(4)));
            }
        }
    }
    Err(format!(
        "Download failed for {}: {last_error}",
        dest.display()
    ))
}

fn download_once(
    url: &str,
    dest: &Path,
    label: &str,
    sha1: Option<&str>,
    progress: bool,
) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(120))
        .build();
    let response = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept-Encoding", "identity")
        .call()
        .map_err(|e| e.to_string())?;
    if response.status() != 200 {
        return Err(format!("Unexpected HTTP {}", response.status()));
    }
    let length = response
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok());
    let parent = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let bar = if progress {
        ProgressBar::new(length.unwrap_or(0))
    } else {
        ProgressBar::hidden()
    };
    bar.set_message(label.to_owned());
    let result = (|| -> Result<(), String> {
        let copied = copy_checked(response.into_reader(), &mut temp, length, &bar)
            .map_err(|e| e.to_string())?;
        if copied == 0 {
            return Err("Empty response body".into());
        }
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        if let Some(expected) = sha1 {
            let actual = util::sha1_file(temp.path()).map_err(|e| e.to_string())?;
            if !actual.eq_ignore_ascii_case(expected) {
                return Err("SHA-1 mismatch".into());
            }
        } else if matches!(
            dest.extension().and_then(|v| v.to_str()),
            Some("jar" | "zip")
        ) {
            if !util::is_jar_intact(temp.path()) {
                return Err("Invalid ZIP/JAR archive".into());
            }
        } else if dest.extension().is_some_and(|e| e == "json") {
            let data = fs::read(temp.path()).map_err(|e| e.to_string())?;
            serde_json::from_slice::<serde_json::Value>(&data)
                .map_err(|e| format!("Invalid JSON: {e}"))?;
        }
        temp.persist(dest).map_err(|e| e.error.to_string())?;
        Ok(())
    })();
    bar.finish_and_clear();
    result
}

fn copy_checked(
    mut reader: impl Read,
    mut writer: impl Write,
    expected: Option<u64>,
    bar: &ProgressBar,
) -> std::io::Result<u64> {
    let mut bytes = 0;
    let mut buf = [0; 65536];
    loop {
        let count = reader.read(&mut buf)?;
        if count == 0 {
            break;
        }
        writer.write_all(&buf[..count])?;
        bytes += count as u64;
        bar.inc(count as u64);
    }
    if expected.is_some_and(|n| n != bytes) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "Response length mismatch",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn server(response: &'static [u8]) -> (String, std::thread::JoinHandle<()>) {
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", socket.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = socket.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            stream.write_all(response).unwrap();
        });
        (url, handle)
    }

    #[test]
    fn sources_respect_overrides_and_origin() {
        let url = "https://piston-data.mojang.com/v1/objects/a/client.jar";
        assert_eq!(source_urls(url, SourceMode::Official), vec![url]);
        assert_eq!(source_urls(url, SourceMode::Auto).len(), 2);
        assert!(source_urls(url, SourceMode::BmclApi)[0]
            .starts_with("https://bmclapi2.bangbang93.com/"));
        for url in [
            "https://libraries.minecraft.net.evil.test/a",
            "https://example.org/?url=https://libraries.minecraft.net",
            "https://login.live.com/oauth20_token.srf",
        ] {
            assert_eq!(mirror_url(url), url);
        }
    }

    #[test]
    fn status_codes_are_not_replaced_with_zero() {
        let (url, handle) = server(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\nConnection: close\r\n\r\noops");
        assert_eq!(
            http_request_no_mirror("GET", &url, None, None, None, 3, 1).unwrap(),
            (503, b"oops".to_vec())
        );
        handle.join().unwrap();
    }

    #[test]
    fn failed_download_preserves_existing_file_and_cleans_temp() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("asset");
        fs::write(&dest, b"old").unwrap();
        let (url, handle) =
            server(b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\nshort");
        assert!(download_file(&url, &dest, "test", None, 1, false).is_err());
        handle.join().unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"old");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn verifies_checksum_and_reuses_verified_cache() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("asset");
        let hash = "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d";
        let (url, handle) =
            server(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
        download_file(&url, &dest, "", Some(hash), 1, false).unwrap();
        handle.join().unwrap();
        download_file("http://127.0.0.1:1", &dest, "", Some(hash), 1, false).unwrap();
    }

    #[test]
    fn rejects_bad_checksum_without_publishing() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("asset");
        let (url, handle) =
            server(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
        assert!(download_file(
            &url,
            &dest,
            "",
            Some("0000000000000000000000000000000000000000"),
            1,
            false
        )
        .is_err());
        handle.join().unwrap();
        assert!(!dest.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn read_and_write_errors_are_propagated() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert!(copy_checked(&b"hello"[..], Broken, None, &ProgressBar::hidden()).is_err());
        assert!(copy_checked(&b"short"[..], Vec::new(), Some(20), &ProgressBar::hidden()).is_err());
        assert!(download_file("invalid", Path::new("unused"), "", None, 0, false).is_err());
    }
}
