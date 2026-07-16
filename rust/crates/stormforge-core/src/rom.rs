//! Resolves the game's `rom` directory from a configured game directory.
//!
//! Ported from `getRomPath()` in `src/main/modService.js`.

use std::path::{Path, PathBuf};

/// Resolve the `rom` directory for a given game directory.
///
/// - If `game_directory`'s basename is already `rom`, it is returned unchanged (guards
///   against being handed an already-resolved path, matching the JS implementation).
/// - On macOS, the game directory is expected to be the `Stormworks.app` bundle, so the
///   rom lives at `<app>/Contents/Resources/rom`.
/// - On all other platforms, it is `<game_directory>/rom`.
pub fn get_rom_path(game_directory: &Path) -> PathBuf {
    if game_directory.file_name().map(|n| n == "rom").unwrap_or(false) {
        return game_directory.to_path_buf();
    }

    if cfg!(target_os = "macos") {
        game_directory.join("Contents").join("Resources").join("rom")
    } else {
        game_directory.join("rom")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_unchanged_when_already_rom() {
        let path = Path::new("/some/game/rom");
        assert_eq!(get_rom_path(path), path);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_resolves_inside_app_bundle() {
        let path = Path::new("/Applications/Stormworks.app");
        assert_eq!(
            get_rom_path(path),
            PathBuf::from("/Applications/Stormworks.app/Contents/Resources/rom")
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_resolves_directly_under_game_directory() {
        let path = Path::new("/games/Stormworks");
        assert_eq!(get_rom_path(path), PathBuf::from("/games/Stormworks/rom"));
    }
}
