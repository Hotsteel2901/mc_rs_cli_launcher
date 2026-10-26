//! Cross-process verification cache for large, content-addressed caches.
//!
//! Re-hashing every cached library and asset on each launch is pure disk work:
//! a full 1.21 asset set is thousands of files and several gigabytes. Mojang's
//! own launcher only checks presence. We keep something stronger but still
//! cheap by recording `(path, size, mtime) -> verified digest` after a
//! successful check, and trusting that record only while both size and mtime
//! are unchanged.
//!
//! The cache is advisory: a hit skips hashing, a miss (or any I/O problem)
//! falls back to the real checksum. It never weakens the guarantee that a file
//! we *use* matches its metadata, because downloads still verify before
//! publishing and any file we did not record is hashed as before.
//!
//! One bucket is kept per game directory, so a process that serves several
//! directories (tests, embedded frontends) cannot mix records or drop another
//! directory's pending writes.
//!
//! Disable with `MC_LAUNCHER_NO_VERIFY_CACHE=1`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::UNIX_EPOCH;

const CACHE_FILE: &str = ".launcher_verify.json";
const MAX_ENTRIES: usize = 200_000;
/// Marker stored for archives that passed structural validation but have no
/// authoritative digest (loaders, installer jars).
pub const ZIP_MARKER: &str = "!zip";
/// Marker for JSON documents that parsed successfully.
pub const JSON_MARKER: &str = "!json";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Entry {
    size: u64,
    mtime: u64,
    hash: String,
}

struct Bucket {
    dir: PathBuf,
    path: PathBuf,
    entries: HashMap<String, Entry>,
    dirty: bool,
}

static BUCKETS: Mutex<Vec<Bucket>> = Mutex::new(Vec::new());

fn state() -> MutexGuard<'static, Vec<Bucket>> {
    BUCKETS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn disabled() -> bool {
    matches!(
        std::env::var("MC_LAUNCHER_NO_VERIFY_CACHE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes") | Ok("on")
    )
}

/// Load the cache for one game directory. Safe to call repeatedly.
pub fn configure(game_dir: &Path) {
    if disabled() {
        return;
    }
    let dir = game_dir.to_path_buf();
    let mut buckets = state();
    if buckets.iter().any(|bucket| bucket.dir == dir) {
        return;
    }
    let path = dir.join(CACHE_FILE);
    let entries = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<HashMap<String, Entry>>(&bytes).ok())
        .unwrap_or_default();
    buckets.push(Bucket {
        dir,
        path,
        entries,
        dirty: false,
    });
}

fn best_index(buckets: &[Bucket], file: &Path) -> Option<usize> {
    buckets
        .iter()
        .enumerate()
        .filter(|(_, bucket)| file.starts_with(&bucket.dir))
        .max_by_key(|(_, bucket)| bucket.dir.as_os_str().len())
        .map(|(index, _)| index)
}

fn metadata_key(path: &Path) -> Option<(String, u64, u64)> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|age| age.as_nanos().min(u128::from(u64::MAX)) as u64)?;
    Some((path.to_string_lossy().to_string(), metadata.len(), mtime))
}

/// Return the marker/digest recorded for the current state of `path`, if any.
pub fn lookup(path: &Path) -> Option<String> {
    let (key, size, mtime) = metadata_key(path)?;
    let buckets = state();
    let index = best_index(&buckets, path)?;
    buckets[index]
        .entries
        .get(&key)
        .filter(|entry| entry.size == size && entry.mtime == mtime)
        .map(|entry| entry.hash.clone())
}

/// True when this exact file state was already verified against `hash`.
pub fn is_verified(path: &Path, hash: &str) -> bool {
    lookup(path).is_some_and(|recorded| recorded.eq_ignore_ascii_case(hash))
}

/// Remember that `path` matched `hash` in its current state.
pub fn record(path: &Path, hash: &str) {
    if hash.is_empty() {
        return;
    }
    let Some((key, size, mtime)) = metadata_key(path) else {
        return;
    };
    let mut buckets = state();
    let Some(index) = best_index(&buckets, path) else {
        return;
    };
    let bucket = &mut buckets[index];
    if bucket.entries.len() >= MAX_ENTRIES && !bucket.entries.contains_key(&key) {
        // Bounded memory: drop an arbitrary tenth rather than growing without
        // limit on pathological directories. Dropped entries are re-derived.
        let mut drop_keys: Vec<String> = bucket
            .entries
            .keys()
            .take(MAX_ENTRIES / 10)
            .cloned()
            .collect();
        drop_keys.sort();
        for missing in drop_keys {
            bucket.entries.remove(&missing);
        }
    }
    bucket.entries.insert(
        key,
        Entry {
            size,
            mtime,
            hash: hash.to_string(),
        },
    );
    bucket.dirty = true;
}

pub fn forget(path: &Path) {
    let key = path.to_string_lossy().to_string();
    let mut buckets = state();
    let Some(index) = best_index(&buckets, path) else {
        return;
    };
    if buckets[index].entries.remove(&key).is_some() {
        buckets[index].dirty = true;
    }
}

/// Persist pending records. Failures are ignored: the cache is an optimisation,
/// never a correctness dependency.
pub fn flush() {
    let mut buckets = state();
    for bucket in buckets.iter_mut().filter(|bucket| bucket.dirty) {
        let Ok(payload) = serde_json::to_vec(&bucket.entries) else {
            continue;
        };
        if crate::util::atomic_write(&bucket.path, &payload).is_ok() {
            bucket.dirty = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cache is process-global, so these tests must not run concurrently.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn unique_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hrs-verify-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cache_hits_only_while_size_and_mtime_are_unchanged() {
        let _guard = TEST_LOCK.lock().unwrap();
        let dir = unique_dir("hit");
        let file = dir.join("asset");
        fs::write(&file, b"hello").unwrap();
        configure(&dir);
        assert!(!is_verified(
            &file,
            "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"
        ));
        record(&file, "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d");
        assert!(is_verified(
            &file,
            "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"
        ));
        assert!(!is_verified(&file, "0000"));
        fs::write(&file, b"hello world").unwrap();
        assert!(!is_verified(
            &file,
            "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"
        ));
        forget(&file);
        assert!(!is_verified(
            &file,
            "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_writes_a_reloadable_cache() {
        let _guard = TEST_LOCK.lock().unwrap();
        let dir = unique_dir("flush");
        let file = dir.join("lib.jar");
        fs::write(&file, b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
        configure(&dir);
        record(&file, ZIP_MARKER);
        flush();
        let raw = fs::read(dir.join(CACHE_FILE)).unwrap();
        let parsed: HashMap<String, Entry> = serde_json::from_slice(&raw).unwrap();
        assert!(parsed.values().any(|entry| entry.hash == ZIP_MARKER));
        let _ = fs::remove_dir_all(&dir);
    }
}
