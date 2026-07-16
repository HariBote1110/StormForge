//! Playlist "share strings", compatible with the Electron implementation of
//! `generate-share-string` / `import-share-string` in `src/main/ipcHandlers.js`.
//!
//! Format: `stormforge-playlist:` followed by base64 of a zlib-deflated JSON payload
//! `{"mods":[{"name":..,"version":..}, ...]}`.
//!
//! The Electron side uses `pako.deflate` / `pako.inflate`, whose defaults produce/consume
//! zlib-wrapped streams (not raw DEFLATE, and not gzip) — so we use `flate2`'s
//! `ZlibEncoder` / `ZlibDecoder` to stay binary-compatible.

use std::io::{Read, Write};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SHARE_STRING_PREFIX: &str = "stormforge-playlist:";

/// A single mod reference inside a share string payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareMod {
    pub name: String,
    pub version: String,
}

/// The decoded payload of a share string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharePlaylist {
    pub mods: Vec<ShareMod>,
}

#[derive(Debug, Error)]
pub enum ShareStringError {
    #[error("share string does not start with the expected prefix")]
    InvalidPrefix,
    #[error("share string is not valid base64")]
    InvalidBase64(#[from] base64::DecodeError),
    #[error("share string payload could not be decompressed")]
    Decompress(#[from] std::io::Error),
    #[error("decompressed payload is not valid JSON")]
    InvalidJson(#[from] serde_json::Error),
}

/// Build a share string from a playlist payload.
pub fn generate_share_string(playlist: &SharePlaylist) -> Result<String, ShareStringError> {
    let json = serde_json::to_string(playlist)?;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(json.as_bytes())?;
    let compressed = encoder.finish()?;

    let encoded = BASE64.encode(compressed);
    Ok(format!("{SHARE_STRING_PREFIX}{encoded}"))
}

/// Parse a share string back into a playlist payload.
pub fn import_share_string(share_string: &str) -> Result<SharePlaylist, ShareStringError> {
    let encoded = share_string
        .strip_prefix(SHARE_STRING_PREFIX)
        .ok_or(ShareStringError::InvalidPrefix)?;

    let compressed = BASE64.decode(encoded)?;

    let mut decoder = ZlibDecoder::new(compressed.as_slice());
    let mut json = String::new();
    decoder.read_to_string(&mut json)?;

    let playlist: SharePlaylist = serde_json::from_str(&json)?;
    Ok(playlist)
}

/// Errors from importing a share string into the store as a playlist.
#[derive(Debug, Error)]
pub enum ImportError {
    #[error(transparent)]
    Share(#[from] ShareStringError),
    /// One or more required mods are not installed; contains their names.
    #[error("missing mods: {0:?}")]
    MissingMods(Vec<String>),
}

/// Import a share string as a new playlist named `playlist_name`, mirroring the
/// Electron import flow: reject with the list of missing mod names when any required
/// mod is not installed; otherwise save a playlist activating exactly the required mods
/// (all other installed mods inactive) and return its states.
pub fn import_share_as_playlist(
    store: &mut crate::store::Store,
    share_string: &str,
    playlist_name: &str,
) -> Result<std::collections::BTreeMap<String, bool>, ImportError> {
    let playlist = import_share_string(share_string)?;

    let required: Vec<&str> = playlist.mods.iter().map(|m| m.name.as_str()).collect();
    let missing: Vec<String> = required
        .iter()
        .filter(|name| !store.mods.iter().any(|m| m.name == **name))
        .map(|name| name.to_string())
        .collect();
    if !missing.is_empty() {
        return Err(ImportError::MissingMods(missing));
    }

    let states: std::collections::BTreeMap<String, bool> = store
        .mods
        .iter()
        .map(|m| (m.name.clone(), required.contains(&m.name.as_str())))
        .collect();
    store.playlists.insert(playlist_name.to_string(), states.clone());
    Ok(states)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_playlist() {
        let playlist = SharePlaylist {
            mods: vec![
                ShareMod { name: "ModA".into(), version: "1.0".into() },
                ShareMod { name: "ModB".into(), version: "2.3.1".into() },
            ],
        };

        let share_string = generate_share_string(&playlist).unwrap();
        assert!(share_string.starts_with(SHARE_STRING_PREFIX));

        let decoded = import_share_string(&share_string).unwrap();
        assert_eq!(decoded, playlist);
    }

    fn store_with_mods(names: &[&str]) -> crate::store::Store {
        let mut store = crate::store::Store::default();
        for name in names {
            store.mods.push(crate::store::Mod {
                name: name.to_string(),
                path: std::path::PathBuf::from(format!("/mods/{name}")),
                author: "A".into(),
                version: "1.0".into(),
                active: false,
            });
        }
        store
    }

    #[test]
    fn import_creates_playlist_activating_required_mods() {
        let mut store = store_with_mods(&["ModA", "ModB", "ModC"]);
        let share = generate_share_string(&SharePlaylist {
            mods: vec![ShareMod { name: "ModA".into(), version: "1.0".into() }],
        })
        .unwrap();

        let states = import_share_as_playlist(&mut store, &share, "Imported-2026-07-16").unwrap();
        assert_eq!(states["ModA"], true);
        assert_eq!(states["ModB"], false);
        assert_eq!(states["ModC"], false);
        assert_eq!(store.playlists["Imported-2026-07-16"], states);
    }

    #[test]
    fn import_reports_missing_mods_and_creates_nothing() {
        let mut store = store_with_mods(&["ModA"]);
        let share = generate_share_string(&SharePlaylist {
            mods: vec![
                ShareMod { name: "ModA".into(), version: "1.0".into() },
                ShareMod { name: "Absent".into(), version: "2.0".into() },
            ],
        })
        .unwrap();

        let result = import_share_as_playlist(&mut store, &share, "X");
        match result {
            Err(ImportError::MissingMods(missing)) => assert_eq!(missing, vec!["Absent".to_string()]),
            other => panic!("expected MissingMods, got {other:?}"),
        }
        assert!(store.playlists.is_empty());
    }

    #[test]
    fn rejects_bad_prefix() {
        let result = import_share_string("not-a-stormforge-string:abcd");
        assert!(matches!(result, Err(ShareStringError::InvalidPrefix)));
    }

    #[test]
    fn rejects_corrupt_base64() {
        let bad = format!("{SHARE_STRING_PREFIX}not valid base64!!");
        let result = import_share_string(&bad);
        assert!(matches!(result, Err(ShareStringError::InvalidBase64(_))));
    }

    #[test]
    fn rejects_corrupt_compressed_payload() {
        // Valid base64, but not valid zlib data.
        let bad = format!("{SHARE_STRING_PREFIX}{}", BASE64.encode(b"definitely not zlib"));
        let result = import_share_string(&bad);
        assert!(result.is_err());
    }
}
