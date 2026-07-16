//! Core, UI-agnostic logic for StormForge: persisted store handling, ROM path
//! resolution, mod installation/rebuilding, share-string encoding, Metadata.xml parsing
//! and Steam library detection. Ported from the Electron app's `src/main/*.js` files so
//! it can be shared by multiple native frontends (Slint now, Tauri later).

pub mod metadata;
pub mod mods;
pub mod rom;
pub mod share;
pub mod steam;
pub mod store;
