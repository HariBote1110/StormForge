// StormForge native (Slint) GUI — iteration 3: playlist parity, mod deletion, Steam
// auto-detection, share strings, i18n and a repair action.
//
// Shares the Electron app's store.json (see `stormforge_core::store::electron_store_path`)
// so both versions observe the same data. Long-running rom work always happens on a
// worker thread with a Slint timer polling for completion, keeping the UI responsive.

mod i18n;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use i18n::I18n;
use stormforge_core::apply::{differential_apply_recording, ApplyStats};
use stormforge_core::manifest::{default_manifest_path, invalidate_manifest};
use stormforge_core::mods::{add_mod_from_path, backup_rom, default_vanilla_backup_dir, delete_mod};
use stormforge_core::playlists::{
    active_states, delete_playlist, load_playlist, overwrite_playlist, rename_playlist, save_playlist,
};
use stormforge_core::rom::get_rom_path;
use stormforge_core::share::{generate_share_string, import_share_as_playlist, ImportError, ShareMod, SharePlaylist};
use stormforge_core::steam::detect_game_path;
use stormforge_core::store::{electron_store_path, read_store, write_store, Store};

slint::include_modules!();

fn mods_to_model(store: &Store) -> ModelRc<ModItem> {
    let items: Vec<ModItem> = store
        .mods
        .iter()
        .map(|m| ModItem {
            name: SharedString::from(m.name.clone()),
            author: SharedString::from(m.author.clone()),
            version: SharedString::from(m.version.clone()),
            active: m.active,
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

fn playlists_to_model(store: &Store) -> ModelRc<PlaylistItem> {
    let items: Vec<PlaylistItem> = store
        .playlists
        .keys()
        .map(|name| PlaylistItem {
            name: SharedString::from(name.clone()),
            loaded: store.selected_playlist.as_deref() == Some(name.as_str()),
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// Refresh both list models from the store on disk.
fn refresh(window: &MainWindow, store_path: &std::path::Path) {
    let store = read_store(store_path);
    window.set_mods(mods_to_model(&store));
    window.set_playlists(playlists_to_model(&store));
}

/// Push all translated labels into the window.
fn apply_language(window: &MainWindow, tr: &I18n) {
    window.set_language_index(if tr.lang() == "ja" { 1 } else { 0 });
    window.set_l_mods(tr.t("INSTALLED_MODS").into());
    window.set_l_add_mod(tr.t("ADD_MOD").into());
    window.set_l_apply(tr.t("APPLY_CHANGES").into());
    window.set_l_delete(tr.t("DELETE_PLAYLIST").into()); // plain "Delete" in both locales
    window.set_l_playlists(tr.t("PLAYLISTS").into());
    window.set_l_save_as(tr.t("SAVE_AS_NEW_PLAYLIST").into());
    window.set_l_new_playlist_name(tr.t("NEW_PLAYLIST_NAME").into());
    window.set_l_load(tr.t("LOAD_PLAYLIST").into());
    window.set_l_overwrite(tr.t("OVERWRITE_PLAYLIST").into());
    window.set_l_rename(tr.t("RENAME_PLAYLIST").into());
    window.set_l_loaded(tr.t("LOADED_BADGE").into());
    window.set_l_share(tr.t("SHARE_CONFIG").into());
    window.set_l_export(tr.t("EXPORT").into());
    window.set_l_import(tr.t("IMPORT").into());
    window.set_l_copy(tr.t("COPY").into());
    window.set_l_paste_here(tr.t("PASTE_CONFIG_HERE").into());
    window.set_l_settings(tr.t("SETTINGS").into());
    window.set_l_language(tr.t("LANGUAGE").into());
    window.set_l_auto_detect(tr.t("AUTO_DETECT").into());
    window.set_l_update_backup(tr.t("UPDATE_VANILLA_ROM").into());
    window.set_l_repair(tr.t("REPAIR_FULL_REBUILD").into());
}

/// Format apply statistics for the status line, with thousands separators for the
/// unchanged count (which can be ~15,000 on a real rom).
fn format_stats(stats: &ApplyStats, elapsed: std::time::Duration) -> String {
    fn thousands(n: usize) -> String {
        let digits = n.to_string();
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        out
    }
    let mode = if stats.full_rebuild { " (full rebuild)" } else { "" };
    format!(
        "Applied{}: restored {}, copied {}, removed {}, unchanged {} in {:.2}s",
        mode,
        thousands(stats.restored),
        thousands(stats.copied),
        thousands(stats.removed),
        thousands(stats.unchanged),
        elapsed.as_secs_f64(),
    )
}

/// Run `job` on a worker thread while the window shows a busy state, then report its
/// status string and refresh the models. A Slint timer polls for completion so the UI
/// event loop never blocks.
fn run_worker(
    window: &MainWindow,
    store_path: PathBuf,
    busy_text: &str,
    job: impl FnOnce() -> Result<String, String> + Send + 'static,
) {
    window.set_busy(true);
    window.set_status_text(SharedString::from(busy_text));

    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    thread::spawn(move || {
        let _ = tx.send(job());
    });

    let window_weak = window.as_weak();
    let timer = Rc::new(slint::Timer::default());
    let timer_for_closure = timer.clone();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(100), move || {
        if let Ok(result) = rx.try_recv() {
            if let Some(window) = window_weak.upgrade() {
                window.set_busy(false);
                match result {
                    Ok(msg) => window.set_status_text(SharedString::from(msg)),
                    Err(err) => window.set_status_text(SharedString::from(format!("Failed: {err}"))),
                }
                refresh(&window, &store_path);
            }
            timer_for_closure.stop();
        }
    });
    // Slint timers stop when dropped; keep this one alive until it stops itself.
    std::mem::forget(timer);
}

/// Resolve the rom/backup/manifest paths from the store, shared by every rom job.
fn rom_context(store_path: &std::path::Path) -> Result<(Store, PathBuf, PathBuf, PathBuf), String> {
    let store = read_store(store_path);
    let game_directory = store.game_directory.clone().ok_or_else(|| "Game directory is not set.".to_string())?;
    let rom_path = get_rom_path(&game_directory);
    let backup_path = default_vanilla_backup_dir().ok_or_else(|| "Could not resolve backup directory.".to_string())?;
    let manifest_path = default_manifest_path().ok_or_else(|| "Could not resolve manifest path.".to_string())?;
    Ok((store, rom_path, backup_path, manifest_path))
}

/// The differential-apply job body, shared by Apply Changes, playlist Load and Repair.
fn apply_job(store_path: &std::path::Path) -> Result<String, String> {
    let started = std::time::Instant::now();
    let (store, rom_path, backup_path, manifest_path) = rom_context(store_path)?;
    let active_mods: Vec<_> = store.mods.iter().filter(|m| m.active).cloned().collect();
    let (stats, installed_files) =
        differential_apply_recording(&rom_path, &backup_path, &active_mods, &manifest_path)
            .map_err(|e| e.to_string())?;

    // Mirror installedFiles back into the shared store.json: the Electron app's Smart
    // Fast Copy relies on it to know which rom folders to restore, and would corrupt
    // the rom state on its next apply if we left it stale.
    let mut store = read_store(store_path);
    store.installed_files = installed_files;
    write_store(store_path, &store).map_err(|e| e.to_string())?;

    Ok(format_stats(&stats, started.elapsed()))
}

fn confirm(title: &str, message: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(message)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

fn main() {
    let window = MainWindow::new().expect("failed to create window");
    let store_path = electron_store_path().unwrap_or_else(|| PathBuf::from("store.json"));

    let initial_store = read_store(&store_path);
    let tr = I18n::new(initial_store.settings.language.as_deref().unwrap_or("en"));
    apply_language(&window, &tr);
    refresh(&window, &store_path);
    window.set_busy(false);

    // --- mod list -------------------------------------------------------------
    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_toggle_mod(move |index, active| {
            let mut store = read_store(&store_path);
            if let Some(m) = store.mods.get_mut(index as usize) {
                m.active = active;
            }
            let _ = write_store(&store_path, &store);
            if let Some(window) = window_weak.upgrade() {
                refresh(&window, &store_path);
            }
        });
    }

    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_delete_mod(move |index| {
            let Some(window) = window_weak.upgrade() else { return };
            let mut store = read_store(&store_path);
            let Some(mod_entry) = store.mods.get(index as usize) else { return };
            let name = mod_entry.name.clone();

            if !confirm("Delete Mod", &format!("Really delete '{name}'? This cannot be undone.")) {
                return;
            }
            match delete_mod(&mut store, &name) {
                Ok(()) => {
                    let _ = write_store(&store_path, &store);
                    window.set_status_text(SharedString::from(format!("Deleted mod '{name}'.")));
                }
                Err(err) => window.set_status_text(SharedString::from(format!("Failed to delete: {err}"))),
            }
            refresh(&window, &store_path);
        });
    }

    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_add_mod(move || {
            let Some(path) =
                rfd::FileDialog::new().add_filter("StormForge Package", &["slp", "zip"]).pick_file()
            else {
                return;
            };
            let Some(window) = window_weak.upgrade() else { return };

            let mod_name =
                path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "mod".to_string());
            let mods_dir =
                store_path.parent().map(|p| p.join("mods")).unwrap_or_else(|| PathBuf::from("mods"));

            match add_mod_from_path(&path, &mods_dir, &mod_name) {
                Ok(new_mod) => {
                    let mut store = read_store(&store_path);
                    if let Some(existing) = store.mods.iter_mut().find(|m| m.name == new_mod.name) {
                        *existing = new_mod;
                    } else {
                        store.mods.push(new_mod);
                    }
                    let _ = write_store(&store_path, &store);
                    window.set_status_text(SharedString::from("Mod added."));
                }
                Err(err) => window.set_status_text(SharedString::from(format!("Failed to add mod: {err}"))),
            }
            refresh(&window, &store_path);
        });
    }

    // --- apply ------------------------------------------------------------------
    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_apply_changes(move || {
            let Some(window) = window_weak.upgrade() else { return };
            let job_store_path = store_path.clone();
            run_worker(&window, store_path.clone(), "Applying changes...", move || apply_job(&job_store_path));
        });
    }

    // --- playlists ----------------------------------------------------------------
    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_save_playlist(move |name| {
            let Some(window) = window_weak.upgrade() else { return };
            let mut store = read_store(&store_path);
            let states = active_states(&store);
            match save_playlist(&mut store, &name, states) {
                Ok(()) => {
                    let _ = write_store(&store_path, &store);
                    window.set_status_text(SharedString::from(format!("Playlist '{name}' saved.")));
                }
                Err(err) => window.set_status_text(SharedString::from(format!("Failed: {err}"))),
            }
            refresh(&window, &store_path);
        });
    }

    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_load_playlist(move |name| {
            let Some(window) = window_weak.upgrade() else { return };
            let mut store = read_store(&store_path);
            match load_playlist(&mut store, &name) {
                Ok(()) => {
                    let _ = write_store(&store_path, &store);
                    refresh(&window, &store_path);
                    // Loading a playlist immediately applies it, like the Electron app.
                    let job_store_path = store_path.clone();
                    run_worker(&window, store_path.clone(), "Applying playlist...", move || {
                        apply_job(&job_store_path)
                    });
                }
                Err(err) => window.set_status_text(SharedString::from(format!("Failed: {err}"))),
            }
        });
    }

    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_overwrite_playlist(move |name| {
            let Some(window) = window_weak.upgrade() else { return };
            if !confirm("Overwrite Playlist", &format!("Overwrite '{name}' with the current mod states?")) {
                return;
            }
            let mut store = read_store(&store_path);
            let states = active_states(&store);
            match overwrite_playlist(&mut store, &name, states) {
                Ok(()) => {
                    let _ = write_store(&store_path, &store);
                    window.set_status_text(SharedString::from(format!("Playlist '{name}' overwritten.")));
                }
                Err(err) => window.set_status_text(SharedString::from(format!("Failed: {err}"))),
            }
            refresh(&window, &store_path);
        });
    }

    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_rename_playlist(move |old_name, new_name| {
            let Some(window) = window_weak.upgrade() else { return };
            let mut store = read_store(&store_path);
            match rename_playlist(&mut store, &old_name, &new_name) {
                Ok(()) => {
                    let _ = write_store(&store_path, &store);
                    window.set_status_text(SharedString::from(format!("Renamed '{old_name}' to '{new_name}'.")));
                }
                Err(err) => window.set_status_text(SharedString::from(format!("Failed: {err}"))),
            }
            refresh(&window, &store_path);
        });
    }

    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_delete_playlist(move |name| {
            let Some(window) = window_weak.upgrade() else { return };
            if !confirm("Delete Playlist", &format!("Really delete '{name}'? This cannot be undone.")) {
                return;
            }
            let mut store = read_store(&store_path);
            match delete_playlist(&mut store, &name) {
                Ok(()) => {
                    let _ = write_store(&store_path, &store);
                    window.set_status_text(SharedString::from(format!("Playlist '{name}' deleted.")));
                }
                Err(err) => window.set_status_text(SharedString::from(format!("Failed: {err}"))),
            }
            refresh(&window, &store_path);
        });
    }

    // --- share strings ----------------------------------------------------------
    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_export_share(move || {
            let Some(window) = window_weak.upgrade() else { return };
            let store = read_store(&store_path);
            let playlist = SharePlaylist {
                mods: store
                    .mods
                    .iter()
                    .filter(|m| m.active)
                    .map(|m| ShareMod { name: m.name.clone(), version: m.version.clone() })
                    .collect(),
            };
            match generate_share_string(&playlist) {
                Ok(s) => window.set_share_text(SharedString::from(s)),
                Err(err) => window.set_status_text(SharedString::from(format!("Failed: {err}"))),
            }
        });
    }

    {
        let window_weak = window.as_weak();
        window.on_copy_share(move || {
            let Some(window) = window_weak.upgrade() else { return };
            let text = window.get_share_text().to_string();
            match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
                Ok(()) => window.set_status_text(SharedString::from("Copied to clipboard.")),
                Err(err) => window.set_status_text(SharedString::from(format!("Clipboard error: {err}"))),
            }
        });
    }

    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_import_share(move |text| {
            let Some(window) = window_weak.upgrade() else { return };
            let mut store = read_store(&store_path);
            let playlist_name = format!("Imported-{}", chrono::Local::now().format("%Y-%m-%d"));
            match import_share_as_playlist(&mut store, text.trim(), &playlist_name) {
                Ok(states) => {
                    // Reflect the imported states as pending checkbox changes (the user
                    // still clicks Apply), mirroring the Electron import flow.
                    for mod_entry in &mut store.mods {
                        mod_entry.active = states.get(&mod_entry.name).copied().unwrap_or(false);
                    }
                    let _ = write_store(&store_path, &store);
                    window.set_import_text(SharedString::from(""));
                    window.set_status_text(SharedString::from(format!("Playlist '{playlist_name}' created.")));
                }
                Err(ImportError::MissingMods(missing)) => {
                    window.set_status_text(SharedString::from(format!("Missing mods: {}", missing.join(", "))));
                }
                Err(err) => window.set_status_text(SharedString::from(format!("Failed: {err}"))),
            }
            refresh(&window, &store_path);
        });
    }

    // --- settings ----------------------------------------------------------------
    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_set_language(move |index| {
            let Some(window) = window_weak.upgrade() else { return };
            let lang = if index == 1 { "ja" } else { "en" };
            let mut store = read_store(&store_path);
            store.settings.language = Some(lang.to_string());
            let _ = write_store(&store_path, &store);
            apply_language(&window, &I18n::new(lang));
        });
    }

    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_auto_detect(move || {
            let Some(window) = window_weak.upgrade() else { return };
            let job_store_path = store_path.clone();
            run_worker(&window, store_path.clone(), "Detecting Stormworks installation...", move || {
                let detected = detect_game_path().ok_or("Could not detect Stormworks installation.")?;
                let mut store = read_store(&job_store_path);
                store.game_directory = Some(detected.clone());
                write_store(&job_store_path, &store).map_err(|e| e.to_string())?;

                let rom_path = get_rom_path(&detected);
                if rom_path.is_dir() {
                    let backup_path =
                        default_vanilla_backup_dir().ok_or("Could not resolve backup directory.")?;
                    backup_rom(&rom_path, &backup_path).map_err(|e| e.to_string())?;
                    // The backup changed, so every manifest assumption is stale.
                    let manifest_path = default_manifest_path().ok_or("Could not resolve manifest path.")?;
                    invalidate_manifest(&manifest_path).map_err(|e| e.to_string())?;
                }
                Ok(format!("Detected: {}", detected.display()))
            });
        });
    }

    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_backup_rom(move || {
            let Some(window) = window_weak.upgrade() else { return };
            let job_store_path = store_path.clone();
            run_worker(&window, store_path.clone(), "Backing up vanilla ROM...", move || {
                let (_store, rom_path, backup_path, manifest_path) = rom_context(&job_store_path)?;
                if !rom_path.is_dir() {
                    return Err(format!("ROM directory not found: {}", rom_path.display()));
                }
                backup_rom(&rom_path, &backup_path).map_err(|e| e.to_string())?;
                // The backup changed, so every manifest assumption is stale.
                invalidate_manifest(&manifest_path).map_err(|e| e.to_string())?;
                Ok("Vanilla ROM backup updated.".to_string())
            });
        });
    }

    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_repair(move || {
            let Some(window) = window_weak.upgrade() else { return };
            let job_store_path = store_path.clone();
            run_worker(&window, store_path.clone(), "Repairing (full rebuild)...", move || {
                // Dropping the manifest forces the next apply onto the full-rebuild path.
                let manifest_path = default_manifest_path().ok_or("Could not resolve manifest path.")?;
                invalidate_manifest(&manifest_path).map_err(|e| e.to_string())?;
                apply_job(&job_store_path)
            });
        });
    }

    window.run().expect("failed to run window");
}
