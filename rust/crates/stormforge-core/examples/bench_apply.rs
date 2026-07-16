//! Synthetic benchmark: full rebuild vs differential reapply.
//!
//! Builds a fake vanilla rom (default 5,000 small files) and a mod overriding a subset
//! of them inside a tempdir, then times:
//!   1. first apply (no manifest -> full rebuild)
//!   2. reapply of the identical playlist (differential, expected ~0 work)
//!   3. playlist switch (deactivate the mod -> restore only the touched files)
//!
//! Everything happens inside a temporary directory — the real game rom, vanilla
//! backup and store.json are never touched. Run with:
//!   cargo run --release -p stormforge-core --example bench_apply

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use stormforge_core::apply::differential_apply;
use stormforge_core::store::Mod;

const VANILLA_FILES: usize = 5_000;
const MOD_FILES: usize = 500;

fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn build_fixture(root: &Path) -> (PathBuf, Mod) {
    let backup = root.join("vanilla_backup");
    // Spread files over the real rom's top-level folders and some subdirectories.
    let folders = ["meshes", "definitions", "audio", "graphics", "data"];
    for i in 0..VANILLA_FILES {
        let folder = folders[i % folders.len()];
        let sub = i % 20;
        write_file(&backup.join(folder).join(format!("sub{sub}")).join(format!("file{i}.bin")), &format!("vanilla {i}"));
    }

    // The mod overrides the first MOD_FILES vanilla files (same layout, "Meshes" etc.
    // capitalised to exercise the case normalisation) and adds a few new ones.
    let mod_dir = root.join("mods").join("BenchMod");
    for i in 0..MOD_FILES {
        let folder = folders[i % folders.len()];
        let capitalised = format!("{}{}", folder[..1].to_uppercase(), &folder[1..]);
        let sub = i % 20;
        write_file(&mod_dir.join(capitalised).join(format!("sub{sub}")).join(format!("file{i}.bin")), &format!("modded {i}"));
    }

    let bench_mod = Mod {
        name: "BenchMod".into(),
        path: mod_dir,
        author: "bench".into(),
        version: "1.0".into(),
        active: true,
    };
    (backup, bench_mod)
}

fn main() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    println!("Building synthetic rom fixture ({VANILLA_FILES} vanilla files, {MOD_FILES} mod files)...");
    let (backup, bench_mod) = build_fixture(root);

    let rom = root.join("rom");
    let manifest_path = root.join("app_data").join("rom_manifest.json");
    let active = vec![bench_mod.clone()];

    let t = Instant::now();
    let stats = differential_apply(&rom, &backup, &active, &manifest_path).unwrap();
    let full_rebuild_time = t.elapsed();
    println!(
        "1) First apply (full rebuild):   {:>8.3}s  (copied {}, unchanged {}, full_rebuild={})",
        full_rebuild_time.as_secs_f64(),
        stats.copied,
        stats.unchanged,
        stats.full_rebuild
    );

    let t = Instant::now();
    let stats = differential_apply(&rom, &backup, &active, &manifest_path).unwrap();
    let reapply_time = t.elapsed();
    println!(
        "2) Same-playlist reapply (diff): {:>8.3}s  (copied {}, restored {}, removed {}, unchanged {})",
        reapply_time.as_secs_f64(),
        stats.copied,
        stats.restored,
        stats.removed,
        stats.unchanged
    );
    assert_eq!(stats.copied + stats.restored + stats.removed, 0, "reapply must be zero work");

    let t = Instant::now();
    let stats = differential_apply(&rom, &backup, &[], &manifest_path).unwrap();
    let switch_time = t.elapsed();
    println!(
        "3) Deactivate mod (diff):        {:>8.3}s  (copied {}, restored {}, removed {}, unchanged {})",
        switch_time.as_secs_f64(),
        stats.copied,
        stats.restored,
        stats.removed,
        stats.unchanged
    );

    println!(
        "\nDifferential reapply is {:.0}x faster than the full rebuild on this fixture.",
        full_rebuild_time.as_secs_f64() / reapply_time.as_secs_f64().max(f64::EPSILON)
    );
}
