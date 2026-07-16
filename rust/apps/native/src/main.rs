// StormForge native (Slint) GUI — first iteration MVP.
//
// Loads the same store.json the Electron app uses (see
// `stormforge_core::store::electron_store_path`), so both versions of the app observe
// the same mod list/state. Full playlist parity, share strings and Steam
// auto-detection are wired up in stormforge-core already, but not yet surfaced in this
// UI — that is left for the next iteration (see the final report for the full list).

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use stormforge_core::mods::{add_mod_from_path, default_vanilla_backup_dir, rebuild_rom_from_active_mods};
use stormforge_core::rom::get_rom_path;
use stormforge_core::store::{electron_store_path, read_store, write_store, Store};

slint::include_modules!();

/// Messages sent from the "Apply Changes" worker thread back to the UI thread.
enum WorkerMessage {
    Done(Result<(), String>),
}

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

fn playlists_to_model(store: &Store) -> ModelRc<SharedString> {
    let names: Vec<SharedString> = store.playlists.keys().map(|k| SharedString::from(k.clone())).collect();
    ModelRc::new(VecModel::from(names))
}

fn store_path() -> PathBuf {
    // Fall back to a local file if we somehow can't resolve $HOME (headless CI, etc.),
    // matching the "fall back to empty store if missing" requirement.
    electron_store_path().unwrap_or_else(|| PathBuf::from("store.json"))
}

fn main() {
    let window = MainWindow::new().expect("failed to create window");
    let store_path = store_path();

    let initial_store = read_store(&store_path);
    window.set_mods(mods_to_model(&initial_store));
    window.set_playlists(playlists_to_model(&initial_store));
    window.set_status_text(SharedString::from(""));
    window.set_busy(false);

    // Toggle a mod's active state in the store and refresh the model. Kept simple and
    // synchronous since it's just an in-memory + on-disk state flip.
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
                window.set_mods(mods_to_model(&store));
            }
        });
    }

    // Add Mod: native file picker, then extract via stormforge-core.
    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_add_mod(move || {
            let Some(path) = rfd::FileDialog::new()
                .add_filter("StormForge Package", &["slp", "zip"])
                .pick_file()
            else {
                return;
            };

            let Some(window) = window_weak.upgrade() else { return };

            let mod_name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "mod".to_string());

            let mods_dir = store_path
                .parent()
                .map(|p| p.join("mods"))
                .unwrap_or_else(|| PathBuf::from("mods"));

            match add_mod_from_path(&path, &mods_dir, &mod_name) {
                Ok(new_mod) => {
                    let mut store = read_store(&store_path);
                    if let Some(existing) = store.mods.iter_mut().find(|m| m.name == new_mod.name) {
                        *existing = new_mod;
                    } else {
                        store.mods.push(new_mod);
                    }
                    let _ = write_store(&store_path, &store);
                    window.set_mods(mods_to_model(&store));
                    window.set_status_text(SharedString::from("Mod added."));
                }
                Err(err) => {
                    window.set_status_text(SharedString::from(format!("Failed to add mod: {err}")));
                }
            }
        });
    }

    // Apply Changes: rebuild the rom on a worker thread so the UI stays responsive.
    {
        let store_path = store_path.clone();
        let window_weak = window.as_weak();
        window.on_apply_changes(move || {
            let Some(window) = window_weak.upgrade() else { return };
            window.set_busy(true);
            window.set_status_text(SharedString::from("Applying changes..."));

            let store_path_for_thread = store_path.clone();
            let (tx, rx) = mpsc::channel::<WorkerMessage>();

            thread::spawn(move || {
                let result = (|| -> Result<(), String> {
                    let store = read_store(&store_path_for_thread);
                    let game_directory =
                        store.game_directory.clone().ok_or_else(|| "Game directory is not set.".to_string())?;
                    let rom_path = get_rom_path(&game_directory);
                    let backup_path = default_vanilla_backup_dir()
                        .ok_or_else(|| "Could not resolve backup directory.".to_string())?;

                    let active_mods: Vec<_> = store.mods.iter().filter(|m| m.active).cloned().collect();
                    let installed = rebuild_rom_from_active_mods(&rom_path, &backup_path, &active_mods)
                        .map_err(|e| e.to_string())?;

                    let mut store = read_store(&store_path_for_thread);
                    store.installed_files = installed;
                    write_store(&store_path_for_thread, &store).map_err(|e| e.to_string())?;
                    Ok(())
                })();

                let _ = tx.send(WorkerMessage::Done(result));
            });

            // Poll the channel from the UI event loop without blocking it.
            let window_weak_for_timer = window_weak.clone();
            let store_path_for_timer = store_path.clone();
            let timer = slint::Timer::default();
            let timer_rc = Rc::new(timer);
            let timer_rc_for_closure = timer_rc.clone();
            timer_rc.start(
                slint::TimerMode::Repeated,
                std::time::Duration::from_millis(100),
                move || {
                    if let Ok(WorkerMessage::Done(result)) = rx.try_recv() {
                        if let Some(window) = window_weak_for_timer.upgrade() {
                            window.set_busy(false);
                            match result {
                                Ok(()) => {
                                    let store = read_store(&store_path_for_timer);
                                    window.set_mods(mods_to_model(&store));
                                    window.set_status_text(SharedString::from("Changes applied successfully."));
                                }
                                Err(err) => {
                                    window.set_status_text(SharedString::from(format!("Failed: {err}")));
                                }
                            }
                        }
                        timer_rc_for_closure.stop();
                    }
                },
            );
            // Keep the timer alive for the duration of the run by leaking the Rc handle
            // into the closure's own scope is not possible after `start`; instead we
            // intentionally forget it here since Slint timers are otherwise dropped
            // when this closure returns, which would cancel polling before completion.
            std::mem::forget(timer_rc);
        });
    }

    window.run().expect("failed to run window");
}
