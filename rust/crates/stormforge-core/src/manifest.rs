//! The rom manifest: a record of which files in the rom are currently owned by mods.
//!
//! Stored as JSON next to the Rust app's own data (NOT inside the Electron store.json),
//! it maps each mod-owned relative path (e.g. `meshes/foo.mesh`) to a cheap source
//! identity: mod name, mod version, and the source file's size and mtime. Paths absent
//! from the manifest are assumed to be untouched vanilla files — the 15,000-file vanilla
//! rom is never individually recorded or hashed.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Cheap identity of a mod-sourced file: enough to decide "has this changed?" without
/// hashing gigabytes of content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileIdentity {
    pub mod_name: String,
    pub mod_version: String,
    pub size: u64,
    /// Source file modification time, in milliseconds since the Unix epoch.
    pub mtime_ms: i64,
}

/// The manifest proper: relative rom path -> identity of the mod file installed there.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub files: BTreeMap<String, FileIdentity>,
}

/// Errors distinguishing "no manifest yet" from "manifest unreadable/corrupt" — both
/// trigger the full-rebuild fallback, but callers may want to log them differently.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest not found")]
    NotFound,
    #[error("manifest could not be read: {0}")]
    Io(#[from] io::Error),
    #[error("manifest is corrupt: {0}")]
    Corrupt(#[from] serde_json::Error),
}

/// Read the manifest from `path`. A missing file is `Err(NotFound)`; unparsable JSON is
/// `Err(Corrupt)`. Callers fall back to a full rebuild in either case.
pub fn read_manifest(path: &Path) -> Result<Manifest, ManifestError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(ManifestError::NotFound),
        Err(e) => return Err(ManifestError::Io(e)),
    };
    Ok(serde_json::from_str(&contents)?)
}

/// Write the manifest atomically: serialise to a temporary sibling file, then rename it
/// over the target. A crash mid-write therefore never leaves a truncated manifest — the
/// old one (or none) survives, and the fallback path handles both.
pub fn write_manifest_atomic(path: &Path, manifest: &Manifest) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(manifest)?;
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// The default manifest location for the Rust app: platform data dir +
/// `StormForge/rom_manifest.json` (alongside the vanilla backup, deliberately separate
/// from the Electron store.json).
pub fn default_manifest_path() -> Option<PathBuf> {
    let data_dir = dirs::data_dir()?;
    Some(data_dir.join("StormForge").join("rom_manifest.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> Manifest {
        let mut files = BTreeMap::new();
        files.insert(
            "meshes/foo.mesh".to_string(),
            FileIdentity { mod_name: "ModA".into(), mod_version: "1.0".into(), size: 42, mtime_ms: 1700000000000 },
        );
        Manifest { files }
    }

    #[test]
    fn round_trips_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("manifest.json");
        let manifest = sample_manifest();

        write_manifest_atomic(&path, &manifest).unwrap();
        let read_back = read_manifest(&path).unwrap();
        assert_eq!(manifest, read_back);
        // No temp file left behind after a successful write.
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn missing_manifest_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let result = read_manifest(&tmp.path().join("nope.json"));
        assert!(matches!(result, Err(ManifestError::NotFound)));
    }

    #[test]
    fn corrupt_manifest_is_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("manifest.json");
        std::fs::write(&path, "{ this is not json").unwrap();
        let result = read_manifest(&path);
        assert!(matches!(result, Err(ManifestError::Corrupt(_))));
    }
}
