//! Mod installation and ROM rebuilding, ported from `src/main/modService.js` and the
//! `add-mod` handler in `src/main/ipcHandlers.js`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::metadata::{parse_metadata_xml, ModMetadata};
use crate::store::Mod;

/// The mod subfolders that get copied into the rom, and their (lowercased) destination
/// names — matches `modFolders` in `installMod()` in `src/main/modService.js`.
pub const MOD_FOLDERS: &[&str] = &["Meshes", "Definitions", "Audio", "Graphics", "Data"];

#[derive(Debug, Error)]
pub enum ModError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("zip archive error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("vanilla ROM backup not found at {0}")]
    BackupMissing(PathBuf),
    #[error("game directory is not set")]
    GameDirectoryNotSet,
}

/// The default location for the vanilla ROM backup used by the Rust native app.
///
/// Divergence from Electron: `src/main/modService.js` stores this under
/// `app.getPath('userData')/vanilla_rom_backup` (Electron's per-app-name userData dir).
/// The Rust app has no Electron runtime to ask, so this uses the platform data directory
/// (`dirs::data_dir()`) plus `StormForge/vanilla_rom_backup` instead. This is a
/// deliberate, documented divergence — the two apps do not currently share a backup.
pub fn default_vanilla_backup_dir() -> Option<PathBuf> {
    let data_dir = dirs::data_dir()?;
    Some(data_dir.join("StormForge").join("vanilla_rom_backup"))
}

/// Recursively copy the contents of `src` into `dst`, creating `dst` (and
/// subdirectories) as needed. Overwrites existing files, matching the semantics of
/// `fs-extra`'s `fs.copy(src, dst, { overwrite: true })`.
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            fs::copy(&src_path, &dst_path)?;
        }
        // Symlinks are intentionally not followed/recreated; mod packages and the
        // vanilla rom backup are not expected to contain them.
    }
    Ok(())
}

/// Extract a `.slp`/`.zip` mod package into `mods_dir/<mod_name>`, reading its
/// `Metadata.xml` for author/version. Mirrors the `add-mod` IPC handler.
///
/// `mod_name` is expected to be the archive's basename without extension, as computed by
/// the caller (matching `path.basename(filePath, path.extname(filePath))` in the JS).
pub fn add_mod_from_path(archive_path: &Path, mods_dir: &Path, mod_name: &str) -> Result<Mod, ModError> {
    let extract_path = mods_dir.join(mod_name);
    let temp_extract_path = mods_dir.join(format!("__temp_{mod_name}"));

    fs::create_dir_all(&temp_extract_path)?;

    {
        let file = fs::File::open(archive_path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        archive.extract(&temp_extract_path)?;
    }

    // If the archive contained a single top-level directory, treat that as the mod
    // root (matching the JS handler's single-root-folder unwrap).
    let mut entries: Vec<PathBuf> = fs::read_dir(&temp_extract_path)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();

    let mod_root_path = if entries.len() == 1 && entries[0].is_dir() {
        entries[0].clone()
    } else {
        temp_extract_path.clone()
    };

    fs::create_dir_all(&extract_path)?;
    copy_dir_recursive(&mod_root_path, &extract_path)?;
    fs::remove_dir_all(&temp_extract_path)?;

    let metadata_path = extract_path.join("Metadata.xml");
    let metadata = if metadata_path.exists() {
        let xml = fs::read_to_string(&metadata_path)?;
        parse_metadata_xml(&xml)
    } else {
        ModMetadata::default()
    };

    Ok(Mod {
        name: mod_name.to_string(),
        path: extract_path,
        author: metadata.author,
        version: metadata.version,
        active: false,
    })
}

/// Install a single mod into the rom: copies each present `MOD_FOLDERS` subdirectory
/// into the lowercased destination folder under `rom_path`, returning every destination
/// file path so it can be recorded in `store.installedFiles`. Mirrors `installMod()`.
pub fn install_mod(mod_entry: &Mod, rom_path: &Path) -> Result<Vec<PathBuf>, ModError> {
    let mut installed_files = Vec::new();

    for folder in MOD_FOLDERS {
        let source_dir = mod_entry.path.join(folder);
        if !source_dir.is_dir() {
            continue;
        }

        let dest_dir = rom_path.join(folder.to_lowercase());
        fs::create_dir_all(&dest_dir)?;
        copy_dir_recursive(&source_dir, &dest_dir)?;

        for entry in fs::read_dir(&source_dir)? {
            let entry = entry?;
            installed_files.push(dest_dir.join(entry.file_name()));
        }
    }

    Ok(installed_files)
}

/// Back up the current rom directory (assumed vanilla/unmodded) to `backup_path`.
/// Mirrors `backupRom()`: clears the backup dir first, then copies the rom into it.
pub fn backup_rom(rom_path: &Path, backup_path: &Path) -> Result<(), ModError> {
    if backup_path.exists() {
        fs::remove_dir_all(backup_path)?;
    }
    fs::create_dir_all(backup_path)?;
    copy_dir_recursive(rom_path, backup_path)?;
    Ok(())
}

/// Restore the rom from the vanilla backup, then install every active mod, returning a
/// map of mod name -> installed file paths (for `store.installedFiles`). Mirrors
/// `rebuildRomFromActiveMods()`'s full-restore path.
///
/// Note (divergence from a planned "Smart Fast Copy" path): at the time of this port,
/// `src/main/modService.js` only implements the full clear-and-restore path described
/// above — there is no incremental/fast-copy branch in the current Electron codebase to
/// port. `Settings::fast_copy` is defined on the Rust side for forward compatibility, but
/// this function always performs a full rebuild for now.
pub fn rebuild_rom_from_active_mods(
    rom_path: &Path,
    vanilla_backup_path: &Path,
    active_mods: &[Mod],
) -> Result<std::collections::BTreeMap<String, Vec<PathBuf>>, ModError> {
    if !vanilla_backup_path.exists() {
        return Err(ModError::BackupMissing(vanilla_backup_path.to_path_buf()));
    }

    if rom_path.exists() {
        fs::remove_dir_all(rom_path)?;
    }
    fs::create_dir_all(rom_path)?;
    copy_dir_recursive(vanilla_backup_path, rom_path)?;

    let mut installed_files = std::collections::BTreeMap::new();
    for mod_entry in active_mods {
        let files = install_mod(mod_entry, rom_path)?;
        installed_files.insert(mod_entry.name.clone(), files);
    }

    Ok(installed_files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = File::create(path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
    }

    #[test]
    fn install_mod_copies_folders_to_lowercased_dests_and_records_files() {
        let tmp = tempfile::tempdir().unwrap();
        let mod_dir = tmp.path().join("MyMod");
        write_file(&mod_dir.join("Data").join("thing.xml"), "<xml/>");
        write_file(&mod_dir.join("Meshes").join("mesh.mesh"), "meshdata");

        let rom_path = tmp.path().join("rom");
        fs::create_dir_all(&rom_path).unwrap();

        let mod_entry = Mod {
            name: "MyMod".to_string(),
            path: mod_dir,
            author: "A".into(),
            version: "1".into(),
            active: true,
        };

        let installed = install_mod(&mod_entry, &rom_path).unwrap();

        assert!(rom_path.join("data").join("thing.xml").exists());
        assert!(rom_path.join("meshes").join("mesh.mesh").exists());
        // Destination folder names must be lowercase (checked via directory listing
        // rather than a negative `Data`-exists check, since case-insensitive
        // filesystems such as default macOS APFS volumes would make that check
        // meaningless).
        let dest_names: Vec<String> = fs::read_dir(&rom_path)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(dest_names.contains(&"data".to_string()));
        assert!(dest_names.contains(&"meshes".to_string()));

        assert_eq!(installed.len(), 2);
        assert!(installed.iter().any(|p| p.ends_with("data/thing.xml") || p.ends_with("data\\thing.xml")));
    }

    #[test]
    fn install_mod_skips_absent_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let mod_dir = tmp.path().join("EmptyMod");
        fs::create_dir_all(&mod_dir).unwrap();
        let rom_path = tmp.path().join("rom");
        fs::create_dir_all(&rom_path).unwrap();

        let mod_entry = Mod {
            name: "EmptyMod".to_string(),
            path: mod_dir,
            author: "A".into(),
            version: "1".into(),
            active: true,
        };

        let installed = install_mod(&mod_entry, &rom_path).unwrap();
        assert!(installed.is_empty());
    }

    #[test]
    fn rebuild_restores_from_backup_then_installs_active_mods() {
        let tmp = tempfile::tempdir().unwrap();

        // Vanilla backup contains one baseline file.
        let backup_path = tmp.path().join("vanilla_backup");
        write_file(&backup_path.join("data").join("vanilla.xml"), "<vanilla/>");

        // Rom starts out already populated with a stale mod file that must be wiped.
        let rom_path = tmp.path().join("rom");
        write_file(&rom_path.join("data").join("stale.xml"), "<stale/>");

        // One active mod, one inactive mod (inactive must not be installed).
        let active_mod_dir = tmp.path().join("mods").join("Active");
        write_file(&active_mod_dir.join("Data").join("active.xml"), "<active/>");
        let active_mod = Mod {
            name: "Active".to_string(),
            path: active_mod_dir,
            author: "A".into(),
            version: "1".into(),
            active: true,
        };

        let installed_files =
            rebuild_rom_from_active_mods(&rom_path, &backup_path, std::slice::from_ref(&active_mod)).unwrap();

        // Vanilla baseline file restored.
        assert!(rom_path.join("data").join("vanilla.xml").exists());
        // Stale leftover file gone.
        assert!(!rom_path.join("data").join("stale.xml").exists());
        // Active mod installed.
        assert!(rom_path.join("data").join("active.xml").exists());

        assert!(installed_files.contains_key("Active"));
        assert_eq!(installed_files["Active"].len(), 1);
    }

    #[test]
    fn rebuild_errors_when_backup_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let rom_path = tmp.path().join("rom");
        let backup_path = tmp.path().join("no_such_backup");

        let result = rebuild_rom_from_active_mods(&rom_path, &backup_path, &[]);
        assert!(matches!(result, Err(ModError::BackupMissing(_))));
    }

    /// Build a tiny zip archive at `zip_path` containing `<root_dir>/Metadata.xml` and
    /// `<root_dir>/Data/thing.xml`, simulating a single-root-folder `.slp` package.
    fn write_fixture_zip(zip_path: &Path, root_dir: &str) {
        let file = File::create(zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<()> = zip::write::FileOptions::default();

        zip.start_file(format!("{root_dir}/Metadata.xml"), options).unwrap();
        zip.write_all(b"<Metadata><Author>Tester</Author><Version>9.9</Version></Metadata>")
            .unwrap();

        zip.start_file(format!("{root_dir}/Data/thing.xml"), options).unwrap();
        zip.write_all(b"<xml/>").unwrap();

        zip.finish().unwrap();
    }

    #[test]
    fn add_mod_from_path_extracts_single_root_folder_and_reads_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("MyMod.slp");
        write_fixture_zip(&zip_path, "MyMod");

        let mods_dir = tmp.path().join("mods");
        fs::create_dir_all(&mods_dir).unwrap();

        let mod_entry = add_mod_from_path(&zip_path, &mods_dir, "MyMod").unwrap();

        assert_eq!(mod_entry.name, "MyMod");
        assert_eq!(mod_entry.author, "Tester");
        assert_eq!(mod_entry.version, "9.9");
        assert!(!mod_entry.active);
        assert!(mod_entry.path.join("Metadata.xml").exists());
        assert!(mod_entry.path.join("Data").join("thing.xml").exists());
        // The single root folder ("MyMod/") must be unwrapped, not preserved as nesting.
        assert!(!mod_entry.path.join("MyMod").exists());
    }

    #[test]
    fn backup_rom_copies_current_rom_into_backup_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let rom_path = tmp.path().join("rom");
        write_file(&rom_path.join("data").join("vanilla.xml"), "<vanilla/>");

        let backup_path = tmp.path().join("backup");
        backup_rom(&rom_path, &backup_path).unwrap();

        assert!(backup_path.join("data").join("vanilla.xml").exists());
    }
}
