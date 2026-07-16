//! Steam library detection, ported from `src/main/steamUtils.js` (`detectGamePath`).
//!
//! Filesystem/OS-dependent probing (checking `Stormworks.app` exists, querying the
//! Windows registry, etc.) can't be meaningfully unit tested here, so this module keeps
//! the pure, testable part — extracting library paths that contain a given Steam AppID
//! from a `libraryfolders.vdf` file — separate from the path-probing glue.

use std::path::{Path, PathBuf};

/// Stormworks' Steam AppID.
pub const STORMWORKS_APP_ID: &str = "573090";

/// Extract every `"path"` entry from a `libraryfolders.vdf` file whose block also
/// contains the given app ID, in file order.
///
/// This mirrors the simplified parsing in the JS implementation: rather than a full VDF
/// parser, it splits the file on `"path"` and inspects each resulting block for the
/// app ID and a following `"path" "<value>"` pair.
pub fn extract_library_paths_for_app(vdf_contents: &str, app_id: &str) -> Vec<String> {
    let mut results = Vec::new();
    let quoted_app_id = format!("\"{app_id}\"");

    // Splitting on `"path"` mirrors the JS `vdfContent.split('"path"')`; the first
    // fragment is header noise before any `"path"` key and is skipped.
    let fragments: Vec<&str> = vdf_contents.split("\"path\"").collect();
    for fragment in fragments.iter().skip(1) {
        let block = format!("\"path\"{fragment}");
        if !block.contains(&quoted_app_id) {
            continue;
        }

        if let Some(path_value) = extract_first_path_value(&block) {
            results.push(path_value);
        }
    }

    results
}

/// Extract the value of the first `"path" "<value>"` pair in a VDF fragment.
fn extract_first_path_value(block: &str) -> Option<String> {
    let after_key = block.strip_prefix("\"path\"")?;
    let trimmed = after_key.trim_start();
    let inner = trimmed.strip_prefix('"')?;
    let end = inner.find('"')?;
    Some(inner[..end].to_string())
}

/// Given a Steam library path and the Stormworks AppID, build the expected game
/// directory: `<library>/steamapps/common/Stormworks`, with `Stormworks.app` appended
/// on macOS (the game directory is expected to point at the `.app` bundle there).
pub fn stormworks_path_from_library(library_path: &str) -> PathBuf {
    let common = Path::new(library_path).join("steamapps").join("common").join("Stormworks");
    if cfg!(target_os = "macos") {
        common.join("Stormworks.app")
    } else {
        common
    }
}

/// The Steam installation directory for the current platform, if it exists. Mirrors
/// `getSteamPath()` in the JS: registry query (approximated here by the default install
/// path) on Windows, `~/Library/Application Support/Steam` on macOS, none elsewhere.
fn get_steam_path() -> Option<PathBuf> {
    let candidate = if cfg!(target_os = "windows") {
        PathBuf::from("C:\\Program Files (x86)\\Steam")
    } else if cfg!(target_os = "macos") {
        dirs::home_dir()?.join("Library").join("Application Support").join("Steam")
    } else {
        return None;
    };
    candidate.is_dir().then_some(candidate)
}

/// Detect the Stormworks installation by inspecting Steam's `libraryfolders.vdf`,
/// falling back to the default library when the VDF is absent or has no match. Returns
/// the game directory in the shape the store expects (the `.app` bundle on macOS).
/// Ported from `detectGamePath()` in `src/main/steamUtils.js`.
pub fn detect_game_path() -> Option<PathBuf> {
    let steam_path = get_steam_path()?;
    let vdf_path = steam_path.join("steamapps").join("libraryfolders.vdf");

    if let Ok(vdf_contents) = std::fs::read_to_string(&vdf_path) {
        for library in extract_library_paths_for_app(&vdf_contents, STORMWORKS_APP_ID) {
            // VDF escapes backslashes on Windows; undo that before path building.
            let library = library.replace("\\\\", "\\");
            let candidate = stormworks_path_from_library(&library);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // Fall back to the default library inside the Steam directory itself.
    let default_candidate = stormworks_path_from_library(&steam_path.to_string_lossy());
    default_candidate.exists().then_some(default_candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A trimmed-down but structurally faithful fixture of a real libraryfolders.vdf.
    const FIXTURE_VDF: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"apps"
		{
			"123"		"1000"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"label"		""
		"apps"
		{
			"573090"		"5000000"
			"456"		"2000"
		}
	}
}
"#;

    #[test]
    fn finds_library_containing_app_id() {
        let paths = extract_library_paths_for_app(FIXTURE_VDF, STORMWORKS_APP_ID);
        assert_eq!(paths, vec!["D:\\\\SteamLibrary".to_string()]);
    }

    #[test]
    fn returns_empty_when_app_id_absent() {
        let paths = extract_library_paths_for_app(FIXTURE_VDF, "999999");
        assert!(paths.is_empty());
    }

    #[test]
    fn builds_expected_game_directory() {
        let path = stormworks_path_from_library("/Users/test/Library/Application Support/Steam");
        let expected_suffix = if cfg!(target_os = "macos") {
            "steamapps/common/Stormworks/Stormworks.app"
        } else {
            "steamapps/common/Stormworks"
        };
        assert!(path.to_string_lossy().ends_with(expected_suffix));
    }
}
