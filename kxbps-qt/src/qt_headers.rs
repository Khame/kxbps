// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// Where the KF6 headers sit, shared by `build.rs` and its tests.
//
// The shims include the KF6 headers by their bare names, so the compiler needs
// `<prefix>/include/KF6/KI18n` and `<prefix>/include/KF6/KStatusNotifierItem`
// on the include path. Qt names its own directory through `qmake -query
// QT_INSTALL_HEADERS`, and in a cross build the qtconf in front of qmake makes
// that answer with the target sysroot. KF6 installs next to Qt under the same
// `include` root, and no qmake query names it.

/// The directory that holds both `qt6/` and `KF6/`, derived from the Qt header
/// directory. A layout without KF6 next to Qt falls back to `/usr/include`.
pub fn include_root(qt_headers: &std::path::Path) -> std::path::PathBuf {
    match qt_headers.parent() {
        Some(root) if root.join("KF6").is_dir() => root.to_path_buf(),
        _ => std::path::PathBuf::from("/usr/include"),
    }
}

#[cfg(test)]
mod tests {
    use super::include_root;
    use std::fs;
    use std::path::{Path, PathBuf};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("kxbps-qt-headers-{name}-{}", std::process::id()));
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

    #[test]
    fn the_parent_of_the_qt_headers_holds_kf6() {
        let dir = TempDir::new("next-to-qt");
        fs::create_dir_all(dir.path().join("include/qt6")).expect("qt headers");
        fs::create_dir_all(dir.path().join("include/KF6")).expect("kf6 headers");

        assert_eq!(
            include_root(&dir.path().join("include/qt6")),
            dir.path().join("include"),
            "KF6 sits next to Qt under the same include root"
        );
    }

    #[test]
    fn a_layout_without_kf6_falls_back_to_usr_include() {
        let dir = TempDir::new("without-kf6");
        fs::create_dir_all(dir.path().join("include/qt6")).expect("qt headers");

        assert_eq!(
            include_root(&dir.path().join("include/qt6")),
            PathBuf::from("/usr/include"),
            "a Debian-style multiarch Qt has no KF6 beside it"
        );
    }
}
