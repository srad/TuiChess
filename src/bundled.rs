//! The Stockfish binary embedded at build time (see `build.rs`), written out to the per-user cache
//! directory so it can be started as a UCI engine.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

static STOCKFISH: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/stockfish.bin"));
const TAG: &str = env!("TUICHESS_STOCKFISH_TAG");
const PREFIX: &str = "stockfish-sf_";

/// Path of the extracted engine, extracting it first if needed.
pub fn path() -> io::Result<PathBuf> {
    let cache = dirs::cache_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no user cache directory"))?;
    extract_to(&cache.join("TuiChess"))
}

pub(crate) fn extract_to(dir: &Path) -> io::Result<PathBuf> {
    let name = format!("stockfish-{TAG}{}", std::env::consts::EXE_SUFFIX);
    let target = dir.join(&name);
    if !matches_embedded(&target) {
        fs::create_dir_all(dir)?;
        // A per-process temp file plus rename: concurrent starts or a crash mid-write never
        // leave a half-written executable at `target`.
        let tmp = dir.join(format!("{name}.{}.tmp", std::process::id()));
        fs::write(&tmp, STOCKFISH)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
        }
        if let Err(e) = fs::rename(&tmp, &target) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    remove_old_versions(dir, &name);
    Ok(target)
}

fn matches_embedded(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    if file.metadata().map(|m| m.len()).ok() != Some(STOCKFISH.len() as u64) {
        return false;
    }
    let mut contents = Vec::with_capacity(STOCKFISH.len());
    file.read_to_end(&mut contents).is_ok() && contents == STOCKFISH
}

/// Best effort: deletes engines extracted by other versions. Temp files are left alone, since
/// they may be another instance's write in progress.
fn remove_old_versions(dir: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if name.starts_with(PREFIX) && name != keep && !name.ends_with(".tmp") {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fresh directory under the system temp dir, unique to this test.
    pub(crate) fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tuichess-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn extracts_once_and_cleans_old_versions() {
        let dir = temp_dir("extract");
        fs::create_dir_all(&dir).unwrap();
        let old = dir.join("stockfish-sf_1.exe");
        let pending = dir.join("stockfish-sf_2.exe.999.tmp");
        fs::write(&old, b"old").unwrap();
        fs::write(&pending, b"in progress").unwrap();

        let first = extract_to(&dir).unwrap();
        assert!(matches_embedded(&first));
        let modified = fs::metadata(&first).unwrap().modified().unwrap();
        let second = extract_to(&dir).unwrap();
        assert_eq!(first, second);
        assert_eq!(fs::metadata(&second).unwrap().modified().unwrap(), modified);

        assert!(!old.exists(), "old version removed");
        assert!(pending.exists(), "temp files of other instances kept");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&first).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_replaced() {
        let dir = temp_dir("corrupt");
        let path = extract_to(&dir).unwrap();
        fs::write(&path, b"garbage").unwrap();
        assert_eq!(extract_to(&dir).unwrap(), path);
        assert!(matches_embedded(&path));
        let _ = fs::remove_dir_all(&dir);
    }
}
