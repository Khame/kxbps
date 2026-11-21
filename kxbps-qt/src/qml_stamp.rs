// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// QML tree fingerprint, shared by `build.rs` and the binary.
//
// clippy and test run build.rs but do not relink, so a debug binary can carry
// QML older than the tree. The hash lets it notice at startup.

use std::path::{Path, PathBuf};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn mix(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Every `.qml` file under `root`, relative and sorted. Only `build.rs` needs
/// the list; the binary needs the hash.
#[allow(dead_code)]
pub fn qml_files(root: &Path) -> Option<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect(root, root, &mut files).ok()?;
    files.sort();
    Some(files)
}

/// FNV-1a over each relative path and file contents. `None` when the tree
/// cannot be read, which is normal for an installed binary.
pub fn qml_tree_hash(root: &Path) -> Option<(u64, usize)> {
    let files = qml_files(root)?;
    let mut hash = FNV_OFFSET;
    for file in &files {
        let contents = std::fs::read(root.join(file)).ok()?;
        hash = mix(hash, file.to_string_lossy().as_bytes());
        hash = mix(hash, &[0]);
        hash = mix(hash, &contents);
        hash = mix(hash, &[0]);
    }
    Some((hash, files.len()))
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(root, &path, out)?;
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "qml") {
            let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            out.push(relative);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("kxbps-qml-stamp-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("temp dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(dir: &Path, relative: &str, body: &str) {
        let path = dir.join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("parent dir");
        fs::write(path, body).expect("write");
    }

    #[test]
    fn an_unchanged_tree_keeps_its_hash() {
        let dir = TempDir::new("same");
        write(dir.path(), "Main.qml", "import QtQuick\n");
        write(dir.path(), "views/Packages.qml", "Item {}\n");

        assert_eq!(
            qml_tree_hash(dir.path()),
            qml_tree_hash(dir.path()),
            "the same tree must hash the same"
        );
    }

    #[test]
    fn the_hash_ignores_the_order_files_appear_in() {
        let first = TempDir::new("order-a");
        write(first.path(), "a.qml", "A\n");
        write(first.path(), "b/c.qml", "C\n");

        let second = TempDir::new("order-b");
        write(second.path(), "b/c.qml", "C\n");
        write(second.path(), "a.qml", "A\n");

        assert_eq!(
            qml_tree_hash(first.path()),
            qml_tree_hash(second.path()),
            "the walk order must not reach the hash"
        );
    }

    #[test]
    fn an_edited_file_changes_the_hash() {
        let dir = TempDir::new("edited");
        write(dir.path(), "Main.qml", "import QtQuick\n");
        let before = qml_tree_hash(dir.path());

        write(dir.path(), "Main.qml", "import QtQuick\nItem {}\n");

        assert_ne!(before, qml_tree_hash(dir.path()));
    }

    #[test]
    fn an_added_file_changes_the_hash() {
        let dir = TempDir::new("added");
        write(dir.path(), "Main.qml", "import QtQuick\n");
        let before = qml_tree_hash(dir.path());

        write(dir.path(), "components/Extra.qml", "Item {}\n");

        assert_ne!(before, qml_tree_hash(dir.path()));
    }

    #[test]
    fn other_extensions_stay_out_of_the_count() {
        let dir = TempDir::new("extensions");
        write(dir.path(), "Main.qml", "import QtQuick\n");
        write(dir.path(), "notes.txt", "not QML\n");
        write(dir.path(), "Main.qml.bak", "backup\n");

        let (_, files) = qml_tree_hash(dir.path()).expect("hash");
        assert_eq!(files, 1);
    }

    #[test]
    fn a_missing_directory_has_no_hash() {
        let dir = TempDir::new("missing");
        let gone = dir.path().join("qml");

        assert_eq!(qml_tree_hash(&gone), None);
    }

    #[test]
    fn the_listing_is_relative_and_sorted() {
        let dir = TempDir::new("listing");
        write(dir.path(), "views/Packages.qml", "Item {}\n");
        write(dir.path(), "Main.qml", "Item {}\n");

        let files = qml_files(dir.path()).expect("list");

        assert_eq!(
            files,
            vec![
                PathBuf::from("Main.qml"),
                PathBuf::from("views/Packages.qml")
            ]
        );
    }
}
