//! Persistent application state, compatible with the Electron store.json format.
//!
//! The Electron app (see `src/main/store.js`) simply reads/writes a JSON blob at
//! `<userData>/store.json` with no fixed schema (plain `fs-extra` readJson/writeJson).
//! Here we give that blob a concrete shape so the Rust side can work with it safely,
//! while keeping every field optional/defaulted so an existing Electron store.json can
//! be read back unchanged (and re-written without losing data other than a stable
//! re-serialisation).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A single installed mod entry, mirroring the shape produced by `add-mod` in
/// `src/main/ipcHandlers.js`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mod {
    pub name: String,
    pub path: PathBuf,
    #[serde(default = "default_unknown")]
    pub author: String,
    #[serde(default = "default_unknown")]
    pub version: String,
    #[serde(default)]
    pub active: bool,
}

fn default_unknown() -> String {
    "Unknown".to_string()
}

/// User-configurable settings. `fast_copy` is a Rust-side addition (not yet present in
/// the Electron store) that will control whether `rebuild_rom_from_active_mods` takes a
/// full-restore path or a smart/incremental copy path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub fast_copy: bool,
}

/// Root persisted state. All fields default so that partial/legacy JSON (as written by
/// the Electron app, which never had `settings`/`playlists`/etc. until first used)
/// deserialises cleanly.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    #[serde(default)]
    pub game_directory: Option<PathBuf>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub mods: Vec<Mod>,
    #[serde(default)]
    pub playlists: BTreeMap<String, BTreeMap<String, bool>>,
    #[serde(default)]
    pub selected_playlist: Option<String>,
    #[serde(default)]
    pub installed_files: BTreeMap<String, Vec<PathBuf>>,
}

/// Read a store from the given path. Returns a default (empty) `Store` if the file does
/// not exist, mirroring `readStore()` in `src/main/store.js`, which swallows read errors
/// and returns `{}`.
pub fn read_store(path: &Path) -> Store {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Store::default(),
    }
}

/// Write a store to the given path as pretty-printed (2-space indent) JSON, matching
/// `writeStore()` in `src/main/store.js` (`fs.writeJsonSync(path, data, { spaces: 2 })`).
pub fn write_store(path: &Path, store: &Store) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(store)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, json)
}

/// The path to the store.json file used by the *Electron* app, so the Rust native app
/// can read/write the same file and both versions stay in sync.
///
/// Electron's `app.getPath('userData')` for an app named `stormforge` resolves (on
/// macOS) to `~/Library/Application Support/stormforge`; `store.js` then joins
/// `store.json` onto that. This deliberately targets that exact path rather than a
/// platform-appropriate Rust convention, since the goal is data-sharing with the
/// existing Electron install, not idiomatic placement.
pub fn electron_store_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join("Library/Application Support/stormforge/store.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_electron_shaped_json() {
        // This is representative of what the Electron app actually writes: camelCase
        // keys, no `settings.fastCopy` (a Rust-only addition), and a mix of populated
        // and empty collections.
        let electron_json = r#"{
  "gameDirectory": "/Applications/Stormworks.app",
  "settings": {
    "language": "en"
  },
  "mods": [
    {
      "name": "MyMod",
      "path": "/Users/test/Library/Application Support/stormforge/mods/MyMod",
      "author": "Someone",
      "version": "1.0",
      "active": true
    }
  ],
  "playlists": {
    "Default": {
      "MyMod": true
    }
  },
  "selectedPlaylist": "Default",
  "installedFiles": {
    "MyMod": [
      "/Applications/Stormworks.app/Contents/Resources/rom/data/foo.xml"
    ]
  }
}"#;

        let store: Store = serde_json::from_str(electron_json).expect("should deserialise");
        assert_eq!(
            store.game_directory,
            Some(PathBuf::from("/Applications/Stormworks.app"))
        );
        assert_eq!(store.settings.language.as_deref(), Some("en"));
        assert!(!store.settings.fast_copy);
        assert_eq!(store.mods.len(), 1);
        assert_eq!(store.mods[0].name, "MyMod");
        assert_eq!(store.selected_playlist.as_deref(), Some("Default"));
        assert_eq!(store.playlists["Default"]["MyMod"], true);
        assert_eq!(store.installed_files["MyMod"].len(), 1);

        // Re-serialise and re-parse: values must survive the round trip.
        let reserialised = serde_json::to_string_pretty(&store).unwrap();
        let store2: Store = serde_json::from_str(&reserialised).unwrap();
        assert_eq!(store, store2);
    }

    #[test]
    fn missing_fields_default_to_empty() {
        let minimal = r#"{}"#;
        let store: Store = serde_json::from_str(minimal).unwrap();
        assert_eq!(store, Store::default());
    }

    #[test]
    fn read_store_returns_default_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does_not_exist.json");
        let store = read_store(&path);
        assert_eq!(store, Store::default());
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");

        let mut store = Store::default();
        store.game_directory = Some(PathBuf::from("/tmp/game"));
        store.mods.push(Mod {
            name: "Foo".into(),
            path: PathBuf::from("/tmp/mods/Foo"),
            author: "Bar".into(),
            version: "2.0".into(),
            active: false,
        });

        write_store(&path, &store).unwrap();
        let read_back = read_store(&path);
        assert_eq!(store, read_back);

        // Confirm the on-disk format is 2-space indented, as required.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("  \"gameDirectory\""));
    }
}
