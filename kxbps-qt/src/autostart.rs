// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The per-user autostart entry: `~/.config/autostart/io.github.khame.kxbps.desktop`.
//!
//! Starting with the session is a file the desktop reads, not a daemon, so this
//! module owns exactly two operations: write the entry when the preference is
//! on, delete it when it is off. The app calls it on every start (`sync`) and
//! on every toggle of the Settings switch, so the file and the preference
//! cannot drift apart.
//!
//! Nothing here is privileged: an XDG autostart entry lives in the user's own
//! config directory. The path is a parameter, not a constant, so the tests
//! write into a private temp directory and the production call resolves it from
//! `XDG_CONFIG_HOME` / `$HOME` through the same rule the desktop uses.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// File name of the entry; the desktop file id equals the application id.
pub const ENTRY_NAME: &str = "io.github.khame.kxbps.desktop";

/// The command a session start runs: hidden window, tray icon up.
pub const ENTRY_EXEC: &str = "kxbps --tray";

/// Writes the entry, or removes it, so the disk matches `enabled`.
///
/// A write that changes nothing does not touch the file, so a start with the
/// entry already in place costs no write and keeps the file's mtime. A failed
/// removal of a file that is not there is a success: the caller asked for "off"
/// and "off" is what it has.
pub fn sync(enabled: bool, path: &Path) -> io::Result<()> {
    if enabled {
        write_entry(path)
    } else {
        remove_entry(path)
    }
}

/// Whether the desktop entry exists, i.e. whether a session start would run it.
///
/// Only existence is checked: the file is ours, and an entry the user edited by
/// hand is still an entry they can see in their autostart settings.
pub fn is_enabled(path: &Path) -> bool {
    path.is_file()
}

/// The rendered desktop entry.
///
/// `Exec` carries `--tray` because the whole point is a background notifier.
/// `X-GNOME-Autostart-enabled` is what GNOME-based sessions read; KDE and other
/// freedesktop sessions use the presence of the file plus `Hidden`.
pub fn entry_contents() -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=KXbps\n\
         GenericName=Package manager\n\
         Comment=Watches for system updates\n\
         Exec={ENTRY_EXEC}\n\
         Icon=io.github.khame.kxbps\n\
         Terminal=false\n\
         NoDisplay=false\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

/// Writes the entry under its own name, replacing an old copy atomically.
///
/// The parent directory is created first: a session that never wrote an
/// autostart entry has no `~/.config/autostart` yet. The temp file plus rename
/// keeps a half-written entry from reaching the session's autostart reader.
fn write_entry(path: &Path) -> io::Result<()> {
    let contents = entry_contents();
    if let Ok(existing) = fs::read_to_string(path)
        && existing == contents
    {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("desktop.tmp");
    fs::write(&tmp, contents.as_bytes())?;
    fs::rename(&tmp, path).inspect_err(|_| {
        // A failed rename leaves the temp file behind; the next sync overwrites
        // it, but removing it now keeps the directory clean either way.
        let _ = fs::remove_file(&tmp);
    })
}

/// Removes the entry; a missing file is already "off".
fn remove_entry(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// `$XDG_CONFIG_HOME/autostart`, or `$HOME/.config/autostart` when the variable
/// is unset, empty or relative (the XDG spec says to ignore relative paths).
///
/// Mirrors the rule the store applies to its data directory so the two never
/// disagree about which home they belong to.
pub fn autostart_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if Path::new(&dir).is_absolute() => PathBuf::from(dir),
        _ => {
            let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
            PathBuf::from(home).join(".config")
        }
    };
    Some(base.join("autostart"))
}

/// The production entry path, under the resolved config home.
pub fn entry_path() -> Option<PathBuf> {
    autostart_dir().map(|dir| dir.join(ENTRY_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    /// A private directory under the system temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "kxbps-autostart-{tag}-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("temp dir");
            TempDir(path)
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

    /// The path the production helper would use under `dir` as its config home.
    fn entry_in(dir: &Path) -> PathBuf {
        dir.join("autostart").join(ENTRY_NAME)
    }

    #[test]
    fn enabling_writes_the_entry_with_the_tray_flag() {
        let dir = TempDir::new("enable");
        let path = entry_in(dir.path());
        assert!(!is_enabled(&path), "nothing is there before the first sync");

        sync(true, &path).expect("write");

        assert!(is_enabled(&path));
        let text = fs::read_to_string(&path).expect("read");
        assert!(text.starts_with("[Desktop Entry]\n"), "a desktop entry");
        assert!(text.contains(&format!("Exec={ENTRY_EXEC}\n")));
        assert_eq!(ENTRY_EXEC, "kxbps --tray");
        assert!(text.contains("X-GNOME-Autostart-enabled=true\n"));
        assert!(text.contains("Type=Application\n"));
        assert!(
            text.contains("Icon=io.github.khame.kxbps\n"),
            "the tray and the autostart entry show the same icon"
        );
    }

    #[test]
    fn the_entry_has_no_trailing_space_before_the_newline() {
        // A desktop-file key whose value ends in a space is silently different
        // from what a reader expects; the continued string literal must not
        // leave one behind.
        let text = entry_contents();
        for line in text.lines() {
            assert_eq!(line.trim_end(), line, "line {line:?} ends in whitespace");
        }
    }

    #[test]
    fn disabling_removes_the_entry() {
        let dir = TempDir::new("disable");
        let path = entry_in(dir.path());
        sync(true, &path).expect("write");

        sync(false, &path).expect("remove");

        assert!(!is_enabled(&path));
        assert!(
            dir.path().join("autostart").is_dir(),
            "the directory is the session's, not ours to delete"
        );
    }

    #[test]
    fn disabling_what_is_already_off_is_not_an_error() {
        let dir = TempDir::new("absent");
        let path = entry_in(dir.path());
        sync(false, &path).expect("a missing entry already means off");
        assert!(!is_enabled(&path));
    }

    #[test]
    fn a_sync_that_changes_nothing_leaves_the_file_alone() {
        let dir = TempDir::new("noop");
        let path = entry_in(dir.path());
        sync(true, &path).expect("write");
        let first = fs::metadata(&path).and_then(|meta| meta.modified());
        let bytes = fs::read(&path).expect("read");

        sync(true, &path).expect("second sync");

        assert_eq!(fs::read(&path).expect("read"), bytes, "same contents");
        assert_eq!(
            fs::metadata(&path).and_then(|meta| meta.modified()).ok(),
            first.ok(),
            "no rewrite, so no new mtime"
        );
    }

    #[test]
    fn a_stale_temp_file_does_not_block_the_write() {
        // A previous run died between `write` and `rename`.
        let dir = TempDir::new("stale-tmp");
        let path = entry_in(dir.path());
        fs::create_dir_all(path.parent().expect("parent")).expect("autostart dir");
        let tmp = path.with_extension("desktop.tmp");
        fs::write(&tmp, b"half").expect("stale temp");

        sync(true, &path).expect("write");

        assert!(is_enabled(&path));
        assert!(!tmp.exists(), "the rename consumed the temp file");
    }

    #[test]
    fn the_switch_round_trips_on_disk() {
        let dir = TempDir::new("roundtrip");
        let path = entry_in(dir.path());
        for _ in 0..2 {
            sync(true, &path).expect("on");
            assert!(is_enabled(&path));
            sync(false, &path).expect("off");
            assert!(!is_enabled(&path));
        }
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    /// Environment variables are process-wide, so the two tests that read them
    /// take this lock instead of relying on the test harness.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_config_home<T>(value: Option<&std::ffi::OsStr>, body: impl FnOnce() -> T) -> T {
        let _guard = ENV.lock().unwrap_or_else(|poison| poison.into_inner());
        let saved = std::env::var_os("XDG_CONFIG_HOME");
        let saved_home = std::env::var_os("HOME");
        // SAFETY: the lock above is the only access to these two variables.
        unsafe {
            match value {
                Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
        let result = body();
        unsafe {
            match &saved {
                Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
            match &saved_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
        result
    }

    #[test]
    fn an_absolute_config_home_is_used_as_is() {
        let home = Path::new("/home/someone");
        let resolved = with_config_home(Some(home.as_os_str()), autostart_dir);
        assert_eq!(resolved, Some(PathBuf::from("/home/someone/autostart")));
    }

    #[test]
    fn a_relative_config_home_is_ignored_in_favour_of_home() {
        // The XDG spec says a relative path is invalid; falling back keeps the
        // entry out of the process' working directory.
        let resolved = with_config_home(Some(std::ffi::OsStr::new("relative")), || {
            // SAFETY: inside the guard taken by `with_config_home`.
            unsafe { std::env::set_var("HOME", "/home/fallback") };
            autostart_dir()
        });
        assert_eq!(
            resolved,
            Some(PathBuf::from("/home/fallback/.config/autostart"))
        );
    }

    #[test]
    fn no_config_home_and_no_home_means_no_path() {
        let resolved = with_config_home(None, || {
            // SAFETY: inside the guard taken by `with_config_home`.
            unsafe { std::env::remove_var("HOME") };
            autostart_dir()
        });
        assert_eq!(resolved, None);
    }
}
