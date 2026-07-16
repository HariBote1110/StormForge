//! Manifest-based differential rom apply.
//!
//! Instead of the Electron approach (wipe the 2.1GB / ~15,000-file rom and restore
//! everything, then re-copy every active mod), this module computes the *difference*
//! between the rom's current mod-owned files (recorded in the manifest, see
//! `crate::manifest`) and the desired state derived from the active mod list, and only
//! touches the files that differ. Re-applying the same playlist is close to zero work;
//! switching playlists moves only the differing files. All copies are filesystem clones
//! where supported (see `crate::fsops`).
//!
//! Safety net: a missing/corrupt manifest, or any error during the differential apply,
//! falls back to a full rebuild (clear + clone-restore from the vanilla backup — itself
//! fast thanks to clonefile) and writes a fresh manifest.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rayon::prelude::*;

use crate::fsops::{clone_dir_recursive, clone_or_copy_file};
use crate::manifest::{read_manifest, write_manifest_atomic, FileIdentity, Manifest};
use crate::mods::{ModError, MOD_FOLDERS};
use crate::store::Mod;

/// A mod-provided file in the desired state: where to copy it from, and its identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModFileSource {
    pub source_path: PathBuf,
    pub identity: FileIdentity,
}

/// The complete desired state of the rom: which relative paths must come from mods
/// (with their sources), and which relative paths exist in the vanilla backup. Paths in
/// neither set must not exist in the rom.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesiredState {
    /// Relative path (normalised: `/`-separated, lowercase top-level folder) -> source.
    pub mod_files: BTreeMap<String, ModFileSource>,
    /// Every relative path present in the vanilla backup.
    pub vanilla_files: BTreeSet<String>,
}

/// A single planned filesystem operation. The diff is a pure function producing these,
/// so the planning logic is unit-testable without touching the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Clone a file from a mod's directory into the rom.
    CopyFromMod { rel_path: String, source_path: PathBuf, identity: FileIdentity },
    /// Clone a file back from the vanilla backup into the rom.
    RestoreVanilla { rel_path: String },
    /// Remove a mod-added file that has no vanilla counterpart.
    Delete { rel_path: String },
}

/// Counters describing what an apply did — surfaced in the GUI status line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyStats {
    pub copied: usize,
    pub restored: usize,
    pub removed: usize,
    pub unchanged: usize,
    /// True when the safety-net full rebuild ran instead of a differential apply.
    pub full_rebuild: bool,
}

fn mtime_ms(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Recursively collect every file under `root`, returning `/`-separated paths relative
/// to `root`, each paired with its metadata.
fn walk_files(root: &Path) -> io::Result<Vec<(String, fs::Metadata)>> {
    let mut results = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let path = entry.path();
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .expect("walked path must be under root")
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                results.push((rel, entry.metadata()?));
            }
        }
    }
    Ok(results)
}

/// Lowercase the top-level folder component of a relative path, leaving the rest
/// untouched (`Meshes/Sub/File.mesh` -> `meshes/Sub/File.mesh`). The rom's real folders
/// are lowercase; mods on disk may use either case (the Electron code used capitalised
/// names and relied on APFS case-insensitivity — we normalise explicitly instead).
fn normalise_top_level(rel: &str) -> String {
    match rel.split_once('/') {
        Some((top, rest)) => format!("{}/{}", top.to_lowercase(), rest),
        None => rel.to_lowercase(),
    }
}

/// Build the desired rom state from the vanilla backup listing plus the active mods in
/// order (a later mod overwrites an earlier one touching the same path — last writer
/// wins). Only files under `MOD_FOLDERS` are considered, matching the mod-side folder
/// names case-insensitively.
pub fn build_desired_state(vanilla_backup: &Path, active_mods: &[Mod]) -> io::Result<DesiredState> {
    let mut state = DesiredState::default();

    for (rel, _meta) in walk_files(vanilla_backup)? {
        state.vanilla_files.insert(normalise_top_level(&rel));
    }

    let wanted_folders: Vec<String> = MOD_FOLDERS.iter().map(|f| f.to_lowercase()).collect();

    for mod_entry in active_mods {
        if !mod_entry.path.is_dir() {
            continue;
        }
        for dir_entry in fs::read_dir(&mod_entry.path)? {
            let dir_entry = dir_entry?;
            if !dir_entry.file_type()?.is_dir() {
                continue;
            }
            let folder_name = dir_entry.file_name().to_string_lossy().to_lowercase();
            if !wanted_folders.contains(&folder_name) {
                continue;
            }

            let folder_path = dir_entry.path();
            for (inner_rel, metadata) in walk_files(&folder_path)? {
                let rel = format!("{folder_name}/{inner_rel}");
                state.mod_files.insert(
                    rel,
                    ModFileSource {
                        source_path: folder_path.join(inner_rel.split('/').collect::<PathBuf>()),
                        identity: FileIdentity {
                            mod_name: mod_entry.name.clone(),
                            mod_version: mod_entry.version.clone(),
                            size: metadata.len(),
                            mtime_ms: mtime_ms(&metadata),
                        },
                    },
                );
            }
        }
    }

    Ok(state)
}

/// Pure diff: compare the manifest (what the rom currently contains beyond vanilla)
/// against the desired state, producing the minimal operation list. No filesystem
/// access — fully unit-testable.
pub fn diff(current: &Manifest, desired: &DesiredState) -> Vec<Op> {
    let mut ops = Vec::new();

    for (rel, current_identity) in &current.files {
        match desired.mod_files.get(rel) {
            Some(src) if src.identity == *current_identity => {} // unchanged: no-op
            Some(src) => ops.push(Op::CopyFromMod {
                rel_path: rel.clone(),
                source_path: src.source_path.clone(),
                identity: src.identity.clone(),
            }),
            None => {
                if desired.vanilla_files.contains(rel) {
                    ops.push(Op::RestoreVanilla { rel_path: rel.clone() });
                } else {
                    ops.push(Op::Delete { rel_path: rel.clone() });
                }
            }
        }
    }

    for (rel, src) in &desired.mod_files {
        if !current.files.contains_key(rel) {
            ops.push(Op::CopyFromMod {
                rel_path: rel.clone(),
                source_path: src.source_path.clone(),
                identity: src.identity.clone(),
            });
        }
    }

    ops
}

/// The manifest the rom will have once `desired` is fully applied.
pub fn manifest_from_desired(desired: &DesiredState) -> Manifest {
    Manifest {
        files: desired.mod_files.iter().map(|(rel, src)| (rel.clone(), src.identity.clone())).collect(),
    }
}

fn rel_to_path(root: &Path, rel: &str) -> PathBuf {
    root.join(rel.split('/').collect::<PathBuf>())
}

/// Execute an op list against the rom, in parallel. Directory creation happens first
/// (sequentially, deduplicated) so the parallel file operations never race on mkdir.
pub fn execute_ops(rom_path: &Path, vanilla_backup: &Path, ops: &[Op]) -> io::Result<()> {
    let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for op in ops {
        let rel = match op {
            Op::CopyFromMod { rel_path, .. } | Op::RestoreVanilla { rel_path } => rel_path,
            Op::Delete { .. } => continue,
        };
        if let Some(parent) = rel_to_path(rom_path, rel).parent() {
            dirs.insert(parent.to_path_buf());
        }
    }
    for dir in &dirs {
        fs::create_dir_all(dir)?;
    }

    ops.par_iter().try_for_each(|op| -> io::Result<()> {
        match op {
            Op::CopyFromMod { rel_path, source_path, .. } => {
                clone_or_copy_file(source_path, &rel_to_path(rom_path, rel_path))
            }
            Op::RestoreVanilla { rel_path } => {
                clone_or_copy_file(&rel_to_path(vanilla_backup, rel_path), &rel_to_path(rom_path, rel_path))
            }
            Op::Delete { rel_path } => match fs::remove_file(rel_to_path(rom_path, rel_path)) {
                Ok(()) => Ok(()),
                // Already gone is fine — the goal state is "absent".
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            },
        }
    })
}

fn stats_from_ops(ops: &[Op], desired: &DesiredState) -> ApplyStats {
    let mut stats = ApplyStats::default();
    for op in ops {
        match op {
            Op::CopyFromMod { .. } => stats.copied += 1,
            Op::RestoreVanilla { .. } => stats.restored += 1,
            Op::Delete { .. } => stats.removed += 1,
        }
    }
    // Everything the rom should contain that we did not have to touch.
    let total_desired: BTreeSet<&String> =
        desired.vanilla_files.iter().chain(desired.mod_files.keys()).collect();
    stats.unchanged = total_desired.len().saturating_sub(stats.copied + stats.restored);
    stats
}

/// Full rebuild: clear the rom, clone-restore the vanilla backup, then clone every
/// desired mod file in parallel. Used as the safety net and for first-time applies.
fn full_rebuild(rom_path: &Path, vanilla_backup: &Path, desired: &DesiredState) -> Result<ApplyStats, ModError> {
    if rom_path.exists() {
        fs::remove_dir_all(rom_path)?;
    }
    fs::create_dir_all(rom_path)?;
    clone_dir_recursive(vanilla_backup, rom_path)?;

    let ops: Vec<Op> = desired
        .mod_files
        .iter()
        .map(|(rel, src)| Op::CopyFromMod {
            rel_path: rel.clone(),
            source_path: src.source_path.clone(),
            identity: src.identity.clone(),
        })
        .collect();
    execute_ops(rom_path, vanilla_backup, &ops)?;

    let mut stats = stats_from_ops(&ops, desired);
    stats.full_rebuild = true;
    Ok(stats)
}

/// Apply the active mods to the rom differentially, falling back to a full rebuild when
/// the manifest is missing/corrupt or any operation fails. The fresh manifest is only
/// written (atomically) after a successful apply.
pub fn differential_apply(
    rom_path: &Path,
    vanilla_backup: &Path,
    active_mods: &[Mod],
    manifest_path: &Path,
) -> Result<ApplyStats, ModError> {
    if !vanilla_backup.exists() {
        return Err(ModError::BackupMissing(vanilla_backup.to_path_buf()));
    }

    let desired = build_desired_state(vanilla_backup, active_mods)?;

    let stats = match read_manifest(manifest_path) {
        Ok(current) => {
            let ops = diff(&current, &desired);
            match execute_ops(rom_path, vanilla_backup, &ops) {
                Ok(()) => stats_from_ops(&ops, &desired),
                // Any mid-apply failure leaves the rom in an unknown mixed state;
                // rebuild it wholesale from the vanilla backup.
                Err(_) => full_rebuild(rom_path, vanilla_backup, &desired)?,
            }
        }
        Err(_) => full_rebuild(rom_path, vanilla_backup, &desired)?,
    };

    write_manifest_atomic(manifest_path, &manifest_from_desired(&desired))?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = fs::File::create(path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
    }

    fn make_mod(name: &str, path: PathBuf) -> Mod {
        Mod { name: name.into(), path, author: "A".into(), version: "1.0".into(), active: true }
    }

    /// A standard fixture: vanilla backup with two files, ModA overriding one vanilla
    /// file, ModB adding a brand-new file.
    struct Fixture {
        _tmp: tempfile::TempDir,
        rom: PathBuf,
        backup: PathBuf,
        manifest_path: PathBuf,
        mod_a: Mod,
        mod_b: Mod,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let backup = tmp.path().join("vanilla_backup");
        write_file(&backup.join("data/base.xml"), "<vanilla base/>");
        write_file(&backup.join("meshes/ship.mesh"), "vanilla ship");

        let mod_a_dir = tmp.path().join("mods/ModA");
        write_file(&mod_a_dir.join("Meshes/ship.mesh"), "modded ship A");
        let mod_b_dir = tmp.path().join("mods/ModB");
        write_file(&mod_b_dir.join("data/extra.xml"), "<extra B/>");

        Fixture {
            rom: tmp.path().join("rom"),
            backup,
            manifest_path: tmp.path().join("app_data/rom_manifest.json"),
            mod_a: make_mod("ModA", mod_a_dir),
            mod_b: make_mod("ModB", mod_b_dir),
            _tmp: tmp,
        }
    }

    #[test]
    fn desired_state_normalises_folder_case_and_orders_last_writer_wins() {
        let f = fixture();
        // ModA uses capitalised "Meshes"; the desired path must be lowercase.
        let desired = build_desired_state(&f.backup, &[f.mod_a.clone()]).unwrap();
        assert!(desired.mod_files.contains_key("meshes/ship.mesh"));
        assert!(desired.vanilla_files.contains("meshes/ship.mesh"));
        assert!(desired.vanilla_files.contains("data/base.xml"));

        // Last writer wins: a second mod touching the same path overrides the first.
        let tmp2 = tempfile::tempdir().unwrap();
        let mod_c_dir = tmp2.path().join("ModC");
        write_file(&mod_c_dir.join("meshes/ship.mesh"), "modded ship C");
        let mod_c = make_mod("ModC", mod_c_dir);

        let desired = build_desired_state(&f.backup, &[f.mod_a.clone(), mod_c]).unwrap();
        assert_eq!(desired.mod_files["meshes/ship.mesh"].identity.mod_name, "ModC");
    }

    #[test]
    fn reapplying_same_state_produces_no_ops() {
        let f = fixture();
        let desired = build_desired_state(&f.backup, &[f.mod_a.clone(), f.mod_b.clone()]).unwrap();
        let manifest = manifest_from_desired(&desired);
        let ops = diff(&manifest, &desired);
        assert!(ops.is_empty(), "same-playlist reapply must plan zero operations, got {ops:?}");
    }

    #[test]
    fn deactivating_mods_restores_vanilla_and_deletes_added_files() {
        let f = fixture();
        // Currently both mods applied…
        let with_mods = build_desired_state(&f.backup, &[f.mod_a.clone(), f.mod_b.clone()]).unwrap();
        let manifest = manifest_from_desired(&with_mods);
        // …now nothing active.
        let empty = build_desired_state(&f.backup, &[]).unwrap();
        let ops = diff(&manifest, &empty);

        // ModA overwrote a vanilla file -> restore; ModB added a new file -> delete.
        assert!(ops.contains(&Op::RestoreVanilla { rel_path: "meshes/ship.mesh".into() }));
        assert!(ops.iter().any(|op| matches!(op, Op::Delete { rel_path } if rel_path == "data/extra.xml")));
        assert_eq!(ops.len(), 2);
    }

    #[test]
    fn changing_mod_identity_replans_the_copy() {
        let f = fixture();
        let desired = build_desired_state(&f.backup, &[f.mod_a.clone()]).unwrap();
        let mut manifest = manifest_from_desired(&desired);
        // Simulate the installed file having come from an older version of the mod.
        manifest.files.get_mut("meshes/ship.mesh").unwrap().mod_version = "0.9".into();

        let ops = diff(&manifest, &desired);
        assert_eq!(ops.len(), 1);
        assert!(matches!(&ops[0], Op::CopyFromMod { rel_path, .. } if rel_path == "meshes/ship.mesh"));
    }

    #[test]
    fn differential_apply_end_to_end_switches_playlists_correctly() {
        let f = fixture();

        // First apply: no manifest yet -> full rebuild.
        let stats = differential_apply(&f.rom, &f.backup, &[f.mod_a.clone(), f.mod_b.clone()], &f.manifest_path)
            .unwrap();
        assert!(stats.full_rebuild);
        assert_eq!(fs::read_to_string(f.rom.join("meshes/ship.mesh")).unwrap(), "modded ship A");
        assert_eq!(fs::read_to_string(f.rom.join("data/extra.xml")).unwrap(), "<extra B/>");

        // Second apply, same playlist: differential, zero work.
        let stats = differential_apply(&f.rom, &f.backup, &[f.mod_a.clone(), f.mod_b.clone()], &f.manifest_path)
            .unwrap();
        assert!(!stats.full_rebuild);
        assert_eq!(stats.copied + stats.restored + stats.removed, 0);
        assert_eq!(stats.unchanged, 3); // base.xml + ship.mesh + extra.xml

        // Switch to only ModB: ship.mesh restored to vanilla, extra.xml untouched.
        let stats = differential_apply(&f.rom, &f.backup, &[f.mod_b.clone()], &f.manifest_path).unwrap();
        assert!(!stats.full_rebuild);
        assert_eq!(stats.restored, 1);
        assert_eq!(stats.copied, 0);
        assert_eq!(stats.removed, 0);
        assert_eq!(fs::read_to_string(f.rom.join("meshes/ship.mesh")).unwrap(), "vanilla ship");
        assert_eq!(fs::read_to_string(f.rom.join("data/extra.xml")).unwrap(), "<extra B/>");

        // Deactivate everything: extra.xml removed, rom back to vanilla.
        let stats = differential_apply(&f.rom, &f.backup, &[], &f.manifest_path).unwrap();
        assert_eq!(stats.removed, 1);
        assert!(!f.rom.join("data/extra.xml").exists());
        assert_eq!(fs::read_to_string(f.rom.join("data/base.xml")).unwrap(), "<vanilla base/>");
    }

    #[test]
    fn corrupt_manifest_triggers_full_rebuild_and_rewrites_manifest() {
        let f = fixture();
        // Prime the rom with a first apply, then corrupt the manifest.
        differential_apply(&f.rom, &f.backup, &[f.mod_a.clone()], &f.manifest_path).unwrap();
        fs::write(&f.manifest_path, "{ definitely not json").unwrap();

        let stats = differential_apply(&f.rom, &f.backup, &[f.mod_a.clone()], &f.manifest_path).unwrap();
        assert!(stats.full_rebuild);
        assert_eq!(fs::read_to_string(f.rom.join("meshes/ship.mesh")).unwrap(), "modded ship A");

        // Manifest must be healthy again afterwards.
        let manifest = read_manifest(&f.manifest_path).unwrap();
        assert!(manifest.files.contains_key("meshes/ship.mesh"));
    }

    #[test]
    fn missing_backup_errors_out() {
        let tmp = tempfile::tempdir().unwrap();
        let result = differential_apply(
            &tmp.path().join("rom"),
            &tmp.path().join("no_backup"),
            &[],
            &tmp.path().join("manifest.json"),
        );
        assert!(matches!(result, Err(ModError::BackupMissing(_))));
    }
}
