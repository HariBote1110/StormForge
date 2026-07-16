//! Playlist operations over the shared store, ported from the playlist IPC handlers in
//! `src/main/ipcHandlers.js` (save/load/rename/delete/overwrite plus selected-playlist
//! tracking). All functions are pure mutations of an in-memory `Store`; persisting the
//! result is the caller's job.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::store::Store;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PlaylistError {
    #[error("playlist name cannot be empty")]
    EmptyName,
    #[error("playlist '{0}' not found")]
    NotFound(String),
    #[error("playlist '{0}' already exists")]
    AlreadyExists(String),
}

/// The current active/inactive state of every mod, keyed by mod name — the shape stored
/// inside a playlist.
pub fn active_states(store: &Store) -> BTreeMap<String, bool> {
    store.mods.iter().map(|m| (m.name.clone(), m.active)).collect()
}

/// Save (or replace) a playlist under `name`. Mirrors the Electron `save-playlist`
/// handler, which rejects only an empty name and otherwise overwrites freely.
pub fn save_playlist(
    store: &mut Store,
    name: &str,
    states: BTreeMap<String, bool>,
) -> Result<(), PlaylistError> {
    if name.is_empty() {
        return Err(PlaylistError::EmptyName);
    }
    store.playlists.insert(name.to_string(), states);
    Ok(())
}

/// Load a playlist: set each mod's `active` from the playlist map (mods absent from the
/// playlist default to inactive), and record it as the selected playlist. Mirrors
/// `load-playlist` (minus the rom rebuild, which the caller triggers separately).
pub fn load_playlist(store: &mut Store, name: &str) -> Result<(), PlaylistError> {
    let playlist = store
        .playlists
        .get(name)
        .ok_or_else(|| PlaylistError::NotFound(name.to_string()))?
        .clone();

    for mod_entry in &mut store.mods {
        mod_entry.active = playlist.get(&mod_entry.name).copied().unwrap_or(false);
    }
    store.selected_playlist = Some(name.to_string());
    Ok(())
}

/// Rename a playlist, rejecting the operation when the target name is taken. Keeps
/// `selected_playlist` pointing at the same (renamed) playlist.
pub fn rename_playlist(store: &mut Store, old_name: &str, new_name: &str) -> Result<(), PlaylistError> {
    if new_name.is_empty() {
        return Err(PlaylistError::EmptyName);
    }
    if store.playlists.contains_key(new_name) {
        return Err(PlaylistError::AlreadyExists(new_name.to_string()));
    }
    let states = store
        .playlists
        .remove(old_name)
        .ok_or_else(|| PlaylistError::NotFound(old_name.to_string()))?;
    store.playlists.insert(new_name.to_string(), states);

    if store.selected_playlist.as_deref() == Some(old_name) {
        store.selected_playlist = Some(new_name.to_string());
    }
    Ok(())
}

/// Delete a playlist. Clears `selected_playlist` if it pointed at the deleted one (the
/// Electron handler leaves it dangling; clearing is the safer behaviour).
pub fn delete_playlist(store: &mut Store, name: &str) -> Result<(), PlaylistError> {
    if store.playlists.remove(name).is_none() {
        return Err(PlaylistError::NotFound(name.to_string()));
    }
    if store.selected_playlist.as_deref() == Some(name) {
        store.selected_playlist = None;
    }
    Ok(())
}

/// Overwrite an existing playlist's states. Unlike `save_playlist`, this refuses to
/// create a new playlist — mirroring the Electron `overwrite-playlist` handler.
pub fn overwrite_playlist(
    store: &mut Store,
    name: &str,
    states: BTreeMap<String, bool>,
) -> Result<(), PlaylistError> {
    if !store.playlists.contains_key(name) {
        return Err(PlaylistError::NotFound(name.to_string()));
    }
    store.playlists.insert(name.to_string(), states);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Mod;
    use std::path::PathBuf;

    fn store_with_mods(names_active: &[(&str, bool)]) -> Store {
        let mut store = Store::default();
        for (name, active) in names_active {
            store.mods.push(Mod {
                name: name.to_string(),
                path: PathBuf::from(format!("/mods/{name}")),
                author: "A".into(),
                version: "1".into(),
                active: *active,
            });
        }
        store
    }

    #[test]
    fn save_and_load_round_trip() {
        let mut store = store_with_mods(&[("A", true), ("B", false)]);
        let states = active_states(&store);
        save_playlist(&mut store, "MyList", states).unwrap();

        // Flip everything, then load: states must come back and selection updates.
        store.mods[0].active = false;
        store.mods[1].active = true;
        load_playlist(&mut store, "MyList").unwrap();
        assert!(store.mods[0].active);
        assert!(!store.mods[1].active);
        assert_eq!(store.selected_playlist.as_deref(), Some("MyList"));
    }

    #[test]
    fn load_defaults_missing_mods_to_inactive() {
        let mut store = store_with_mods(&[("A", true), ("NewMod", true)]);
        // Playlist saved before NewMod existed.
        save_playlist(&mut store, "Old", BTreeMap::from([("A".to_string(), true)])).unwrap();
        load_playlist(&mut store, "Old").unwrap();
        assert!(store.mods[0].active);
        assert!(!store.mods[1].active);
    }

    #[test]
    fn save_rejects_empty_name_and_load_rejects_unknown() {
        let mut store = Store::default();
        assert_eq!(save_playlist(&mut store, "", BTreeMap::new()), Err(PlaylistError::EmptyName));
        assert_eq!(load_playlist(&mut store, "nope"), Err(PlaylistError::NotFound("nope".into())));
    }

    #[test]
    fn rename_moves_states_and_selection_but_rejects_collisions() {
        let mut store = Store::default();
        save_playlist(&mut store, "Old", BTreeMap::from([("A".to_string(), true)])).unwrap();
        save_playlist(&mut store, "Taken", BTreeMap::new()).unwrap();
        store.selected_playlist = Some("Old".into());

        assert_eq!(
            rename_playlist(&mut store, "Old", "Taken"),
            Err(PlaylistError::AlreadyExists("Taken".into()))
        );

        rename_playlist(&mut store, "Old", "New").unwrap();
        assert!(!store.playlists.contains_key("Old"));
        assert_eq!(store.playlists["New"]["A"], true);
        assert_eq!(store.selected_playlist.as_deref(), Some("New"));
    }

    #[test]
    fn delete_removes_and_clears_selection() {
        let mut store = Store::default();
        save_playlist(&mut store, "Gone", BTreeMap::new()).unwrap();
        store.selected_playlist = Some("Gone".into());

        delete_playlist(&mut store, "Gone").unwrap();
        assert!(store.playlists.is_empty());
        assert_eq!(store.selected_playlist, None);
        assert_eq!(delete_playlist(&mut store, "Gone"), Err(PlaylistError::NotFound("Gone".into())));
    }

    #[test]
    fn overwrite_requires_existing_playlist() {
        let mut store = Store::default();
        assert_eq!(
            overwrite_playlist(&mut store, "nope", BTreeMap::new()),
            Err(PlaylistError::NotFound("nope".into()))
        );
        save_playlist(&mut store, "List", BTreeMap::from([("A".to_string(), false)])).unwrap();
        overwrite_playlist(&mut store, "List", BTreeMap::from([("A".to_string(), true)])).unwrap();
        assert_eq!(store.playlists["List"]["A"], true);
    }
}
