//! Where Soos keeps its files, and how it writes them. soos-app and soos-cli
//! both use these paths, so the CLI sees the app's rates and converters.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::RawConverter;

/// Where everything is saved: `data/` next to the executable (symlinks
/// followed, so a symlinked `soos-cli` finds the app's folder), which keeps
/// the app to one folder with no installer. On macOS it's Application
/// Support instead, because updating a `.app` replaces the whole bundle.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("Library/Application Support/Soos");
    }
    // Only if the OS can't say where the executable is.
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("data")))
        .unwrap_or_else(|| PathBuf::from("data"))
}

/// The app's converters, exported for soos-cli.
pub fn converters_path() -> PathBuf {
    data_dir().join("converters.json")
}

/// The exported converters; none if the file is missing or unreadable.
pub fn load_converters(path: &Path) -> Vec<RawConverter> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Writes the converters export atomically (see [`write_atomically`]).
pub fn save_converters(path: &Path, converters: &[RawConverter]) -> io::Result<()> {
    let json = serde_json::to_string_pretty(converters).map_err(io::Error::other)?;
    write_atomically(path, json.as_bytes())
}

/// Replace `path` with `contents` so a crash or a concurrent reader sees the
/// old file or the new one, never half of one: write a temp file in the
/// same directory, flush it, rename it over `path`. `create_new` refuses a
/// symlink planted at the temp path, and the rename replaces a symlink at
/// `path` rather than writing through it.
pub(crate) fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // The pid keeps soos-app and soos-cli from sharing a temp name.
    let temp_path = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(contents)?;
        file.sync_all()
    })()
    .and_then(|()| fs::rename(&temp_path, path));
    if written.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("soos-storage-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn converters_round_trip_through_the_export() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("converters.json");
        let converters = vec![RawConverter {
            unit: "teu".to_string(),
            aliases: vec!["TEU".to_string()],
            base: "cbm".to_string(),
            factor: "33.2".to_string(),
        }];
        save_converters(&path, &converters).unwrap();
        assert_eq!(load_converters(&path), converters);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_or_corrupt_export_means_no_converters() {
        let dir = temp_dir("missing");
        assert!(load_converters(&dir.join("converters.json")).is_empty());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("converters.json"), "not json").unwrap();
        assert!(load_converters(&dir.join("converters.json")).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_file() {
        let dir = temp_dir("atomic");
        let path = dir.join("file.json");
        write_atomically(&path, b"one").unwrap();
        write_atomically(&path, b"two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let leftovers = fs::read_dir(&dir).unwrap().count();
        assert_eq!(leftovers, 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
