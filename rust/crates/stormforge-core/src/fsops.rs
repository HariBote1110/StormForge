//! Low-level filesystem primitives optimised for copy-on-write filesystems.
//!
//! On APFS (macOS) and ReFS (Windows Dev Drive), cloning a file is a constant-time
//! metadata operation rather than a data copy — measured ~11x faster than a byte copy
//! for a 104MB mod on APFS. `clone_or_copy_file` uses `reflink-copy` to clone where the
//! filesystem supports it and silently falls back to a normal copy elsewhere, so all
//! callers can treat it as "the fast copy".

use std::fs;
use std::io;
use std::path::Path;

/// Copy a single file, preferring a filesystem clone (reflink/clonefile) and falling
/// back to a regular copy. Overwrites an existing destination file (clonefile refuses
/// to overwrite, so any existing destination is removed first).
pub fn clone_or_copy_file(src: &Path, dst: &Path) -> io::Result<()> {
    if dst.exists() {
        fs::remove_file(dst)?;
    }
    // `reflink_or_copy` returns Ok(None) when a clone succeeded, Ok(Some(bytes)) when
    // it had to fall back to a byte copy — both are success for our purposes.
    reflink_copy::reflink_or_copy(src, dst).map(|_| ())
}

/// Recursively copy the contents of `src` into `dst` using `clone_or_copy_file` for
/// every file. Creates `dst` (and subdirectories) as needed and overwrites existing
/// files, matching `fs-extra`'s `fs.copy(src, dst, { overwrite: true })` semantics.
pub fn clone_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            clone_dir_recursive(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            clone_or_copy_file(&src_path, &dst_path)?;
        }
        // Symlinks are intentionally not followed/recreated; mod packages and the
        // vanilla rom backup are not expected to contain them.
    }
    Ok(())
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

    #[test]
    fn clones_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.txt");
        let dst = tmp.path().join("dst.txt");
        write_file(&src, "hello");

        clone_or_copy_file(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "hello");
    }

    #[test]
    fn overwrites_existing_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.txt");
        let dst = tmp.path().join("dst.txt");
        write_file(&src, "new contents");
        write_file(&dst, "old contents");

        clone_or_copy_file(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "new contents");
    }

    #[test]
    fn clones_a_directory_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        write_file(&src.join("a.txt"), "a");
        write_file(&src.join("sub").join("b.txt"), "b");

        let dst = tmp.path().join("dst");
        clone_dir_recursive(&src, &dst).unwrap();

        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "a");
        assert_eq!(fs::read_to_string(dst.join("sub").join("b.txt")).unwrap(), "b");
    }
}
