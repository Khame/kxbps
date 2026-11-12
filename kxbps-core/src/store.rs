//! Persistent desktop state: the installed database, the transaction queue,
//! the selected mirror and the user preferences.
//!
//! The web app keeps all of it in one `localStorage` snapshot under
//! `qxbps-v1`; this port keeps the queue, the mirror and the preferences in
//! three JSON files under
//! `$XDG_DATA_HOME/kxbps/` (fallback `$HOME/.local/share/kxbps/`), so a
//! damaged file can only cost one concern and the files stay diffable:
//!
//! | File | Payload |
//! | --- | --- |
//! | `queue.json` | `[QueueItem]` |
//! | `prefs.json` | `Prefs` |
//! | `mirror.json` | `{ "url": "https://…" }` |
//!
//! The installed set is not a file: it is whatever the last `xbps-query -l`
//! said, held in memory and read again on every start.
//!
//! Every write goes to `<file>.tmp` in the same directory, is fsynced and then
//! renamed over the target, so a crash mid-write leaves either the previous
//! file or the new one — never a truncated one. A missing or unparsable file
//! falls back to that concern's default and nothing else is touched; a broken
//! `prefs.json` must not cost the queue.
//!
//! Versions before the rename kept the same files under a `khaxbps` directory
//! next to this one; [`Store::load`] moves them out of it on the first start
//! (see [`migrate_legacy`]).
//!
//! `synced_at` is deliberately not persisted: the web app keeps it in memory
//! only, so a restart forgets when the index was last refreshed.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::engine::{Plan, apply_installed};
use crate::types::{Inst, InstalledMap, Prefs, QueueAction, QueueItem};
use crate::xbps::{InstalledPackage, UpdateEntry};

/// Mirror used until the user picks another one.
///
/// Defined once in [`crate::mirrors`] and re-exported here so the store and the
/// mirror list cannot disagree about the default.
pub use crate::mirrors::DEFAULT_MIRROR;

/// Directory created below the XDG data home.
const DATA_DIR: &str = "kxbps";
/// Directory used by versions before the `kxbps` rename; [`migrate_legacy`]
/// moves the state files out of it on the first start afterwards.
const LEGACY_DATA_DIR: &str = "khaxbps";
const QUEUE_FILE: &str = "queue.json";
const PREFS_FILE: &str = "prefs.json";
const MIRROR_FILE: &str = "mirror.json";
/// The state files this module owns; migration moves exactly these and never
/// touches anything else a user may have left in the old directory. The old
/// `installed.json` is deliberately not among them: the installed set is read
/// from xbps now, and a stale file must not come back through the migration.
const STATE_FILES: [&str; 3] = [QUEUE_FILE, PREFS_FILE, MIRROR_FILE];

/// Everything that goes wrong while reading or writing the data directory.
///
/// A file that is missing or fails to parse is *not* an error: it falls back to
/// its default. Only a data directory that cannot be created or written is.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Neither `XDG_DATA_HOME` nor `HOME` is set, so there is nowhere to
    /// persist to.
    #[error("cannot locate the data directory: XDG_DATA_HOME and HOME are unset")]
    NoDataDir,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// On-disk shape of `mirror.json`.
///
/// An object rather than a bare JSON string so the file can grow (region, last
/// probe result) without a format break.
#[derive(Serialize, serde::Deserialize)]
struct MirrorFile {
    url: String,
}

/// The persisted slice of the UI state.
///
/// The installed set is a cache of what xbps last reported, never a source of
/// truth; `dirty` tells the Qt layer whether a [`Store::save`] is still owed.
#[derive(Debug, Clone)]
pub struct Store {
    pub installed: InstalledMap,
    pub queue: Vec<QueueItem>,
    pub mirror: String,
    pub prefs: Prefs,
    /// When the repository index was last refreshed, epoch milliseconds.
    /// Memory-only, like the web app's `syncedAt`.
    pub synced_at: Option<i64>,
    /// The pending updates xbps reported for the system; memory-only, like
    /// `synced_at`, because the next start reads them again.
    updates: Vec<UpdateEntry>,
    dirty: bool,
}

/// Equality over the persisted concerns only.
///
/// `synced_at` is transient and `dirty`/`baseline` are bookkeeping, so a
/// round-trip test can compare a loaded store with the one that was saved.
impl PartialEq for Store {
    fn eq(&self, other: &Self) -> bool {
        self.installed == other.installed
            && self.queue == other.queue
            && self.mirror == other.mirror
            && self.prefs == other.prefs
    }
}

impl Eq for Store {}

/// The web app's default preferences (`store.ts`, initial state).
fn default_prefs() -> Prefs {
    Prefs {
        notify_on_start: true,
        desktop: false,
        dismissed: String::new(),
    }
}

/// `$XDG_DATA_HOME/kxbps`, or `$HOME/.local/share/kxbps` when the variable
/// is unset, empty or relative (the XDG spec says to ignore relative paths).
pub fn data_dir() -> Result<PathBuf, StoreError> {
    Ok(xdg_data_home()?.join(DATA_DIR))
}

/// The XDG data home itself, resolved once for both the current and the
/// legacy directory so the two can never disagree about where to look.
fn xdg_data_home() -> Result<PathBuf, StoreError> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        let dir = PathBuf::from(dir);
        if dir.is_absolute() {
            return Ok(dir);
        }
    }
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .ok_or(StoreError::NoDataDir)?;
    Ok(PathBuf::from(home).join(".local").join("share"))
}

/// The pre-rename `khaxbps` data directory, resolved through the same XDG
/// rules as [`data_dir`] so the migration finds the old files wherever they
/// are; `None` when there is no data home at all.
fn legacy_data_dir() -> Option<PathBuf> {
    xdg_data_home().ok().map(|dir| dir.join(LEGACY_DATA_DIR))
}

impl Store {
    /// An empty store: nothing is installed until xbps says so.
    pub fn fresh() -> Self {
        Store {
            installed: InstalledMap::new(),
            updates: Vec::new(),
            queue: Vec::new(),
            mirror: DEFAULT_MIRROR.to_string(),
            prefs: default_prefs(),
            synced_at: None,
            dirty: false,
        }
    }

    /// Loads the store from the XDG data directory.
    pub fn load() -> Result<Self, StoreError> {
        let dir = data_dir()?;
        // Before `load_at`, which creates `dir` — after it, the "new directory
        // does not exist yet" check inside would always be false.
        migrate_legacy(&dir);
        Self::load_at(&dir)
    }

    /// Loads the store from `dir`, creating it if needed.
    ///
    /// The installed set starts empty: it is not persisted, and the Qt layer
    /// fills it from xbps right after this returns.
    pub fn load_at(dir: &Path) -> Result<Self, StoreError> {
        ensure_dir(dir)?;
        let mut store = Store::fresh();

        if let Some(queue) = read_json(&dir.join(QUEUE_FILE)) {
            store.queue = queue;
        }
        if let Some(prefs) = read_json(&dir.join(PREFS_FILE)) {
            store.prefs = prefs;
        }
        // An empty URL is what `hydrate` treats as "no mirror saved". Written
        // as `filter` rather than a let-chain so the crate keeps its 1.85 MSRV.
        let saved_mirror = read_json::<MirrorFile>(&dir.join(MIRROR_FILE)).map(|file| file.url);
        if let Some(url) = saved_mirror.filter(|url| !url.is_empty()) {
            store.mirror = url;
        }

        store.dirty = false;
        Ok(store)
    }

    /// Writes every concern to the XDG data directory.
    pub fn save(&mut self) -> Result<(), StoreError> {
        self.save_at(&data_dir()?)
    }

    /// Writes every concern to `dir`, creating it if needed.
    ///
    /// All three files are rewritten; each one is replaced atomically, so a
    /// failure halfway leaves the files it already wrote in their new state
    /// and the rest in their old one — never a half-written file.
    pub fn save_at(&mut self, dir: &Path) -> Result<(), StoreError> {
        ensure_dir(dir)?;
        write_json(dir, QUEUE_FILE, &self.queue)?;
        write_json(dir, PREFS_FILE, &self.prefs)?;
        write_json(
            dir,
            MIRROR_FILE,
            &MirrorFile {
                url: self.mirror.clone(),
            },
        )?;
        self.dirty = false;
        Ok(())
    }

    /// Whether a change since the last save is still waiting to be written.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Queues `action` for `name`, or unqueues it when it is already queued
    /// with the same action (`toggleQueue` in `store.ts`).
    pub fn toggle_queue(&mut self, name: &str, action: QueueAction) {
        let already = self
            .queue
            .iter()
            .any(|item| item.name == name && item.action == action);
        self.queue.retain(|item| item.name != name);
        if !already {
            self.queue.push(QueueItem {
                name: name.to_string(),
                action,
            });
        }
        self.dirty = true;
    }

    /// Drops every queued operation.
    pub fn clear_queue(&mut self) {
        self.queue.clear();
        self.dirty = true;
    }

    /// Drops the queued operation for `name`, whichever action it had.
    pub fn remove_queued(&mut self, name: &str) {
        self.queue.retain(|item| item.name != name);
        self.dirty = true;
    }

    /// Selects the mirror the next transaction and probe use.
    pub fn set_mirror(&mut self, url: &str) {
        self.mirror = url.to_string();
        self.dirty = true;
    }

    /// Replaces the whole preference block.
    pub fn set_prefs(&mut self, prefs: Prefs) {
        self.prefs = prefs;
        self.dirty = true;
    }

    /// Whether updates should be announced when the window opens.
    pub fn set_pref_notify_on_start(&mut self, on: bool) {
        self.prefs.notify_on_start = on;
        self.dirty = true;
    }

    /// Whether the announcement should also go through the desktop
    /// notification service.
    pub fn set_pref_desktop_notifications(&mut self, on: bool) {
        self.prefs.desktop = on;
        self.dirty = true;
    }

    /// The update signature the user dismissed, so it is not announced twice.
    pub fn set_pref_dismissed_version(&mut self, signature: &str) {
        self.prefs.dismissed = signature.to_string();
        self.dirty = true;
    }

    /// Pins or unpins an installed package, returning whether it is now held.
    ///
    /// Returns `false` for a package that is not installed; nothing changes.
    pub fn toggle_hold(&mut self, name: &str) -> bool {
        let Some(inst) = self.installed.get_mut(name) else {
            return false;
        };
        inst.held = !inst.held;
        let held = inst.held;
        self.dirty = true;
        held
    }

    /// Applies a finished transaction, as `runJob` does in `store.ts`.
    ///
    /// The installed database is rewritten through [`apply_installed`], the
    /// packages the plan touched leave the queue, and `synced_at` moves to
    /// `now` (epoch milliseconds). A blocked or empty plan changes nothing.
    pub fn apply_plan(&mut self, plan: &Plan, now: i64) {
        if plan.items.is_empty() {
            return;
        }
        self.installed = apply_installed(&self.installed, &plan.items, now);
        let applied: HashSet<&str> = plan.items.iter().map(|item| item.name.as_str()).collect();
        self.queue
            .retain(|item| !applied.contains(item.name.as_str()));
        self.synced_at = Some(now);
        self.dirty = true;
    }

    /// Forgets the user's choices and the cached system state.
    ///
    /// The queue, the mirror and the notification preferences go back to their
    /// defaults, except for the desktop-notification switch, which the web app
    /// keeps across a reset. The installed cache and the pending updates are
    /// dropped too; nothing is installed or restored here, and the Qt layer
    /// re-reads xbps afterwards.
    pub fn reset(&mut self) {
        self.installed.clear();
        self.updates.clear();
        self.queue.clear();
        self.mirror = DEFAULT_MIRROR.to_string();
        self.prefs = Prefs {
            notify_on_start: true,
            desktop: self.prefs.desktop,
            dismissed: String::new(),
        };
        self.synced_at = None;
        self.dirty = true;
    }

    /// Replaces the installed set with what xbps reports for the system.
    ///
    /// `explicit` and `held` are the names the two list modes answered; every
    /// other field comes from the installed records. The pending-update list is
    /// kept as well, so the update counter and the plan do not have to re-read
    /// xbps.
    pub fn apply_snapshot(
        &mut self,
        records: &[InstalledPackage],
        explicit: &[String],
        held: &[String],
        updates: Vec<UpdateEntry>,
    ) {
        let at = |name: &str| self.installed.get(name).map_or(0, |inst| inst.at);
        self.installed = installed_map(records, explicit, held, &at);
        self.updates = updates;
        self.dirty = true;
    }

    /// The updates xbps reported for the system.
    pub fn pending_updates(&self) -> &[UpdateEntry] {
        &self.updates
    }

    /// The pending update of `name`, if there is one.
    pub fn update_of(&self, name: &str) -> Option<&UpdateEntry> {
        self.updates.iter().find(|entry| entry.name == name)
    }
}

/// Builds the installed map from the snapshot the read layer took.
///
/// `explicit` and `held` are the names `xbps-query -m` and `-H` answered. A
/// name from `xbps-query -O` (orphaned: installed, not explicit, nothing needs
/// it) is not explicit either, which the list already says; its "orphan" state
/// is computed where the package list is filtered, not stored here.
///
/// `at` keeps the install date of a package the store already knew: xbps has
/// no bulk source for it, and the value only grows stale, never wrong.
pub fn installed_map(
    records: &[InstalledPackage],
    explicit: &[String],
    held: &[String],
    at: &dyn Fn(&str) -> i64,
) -> InstalledMap {
    let explicit: HashSet<&str> = explicit.iter().map(String::as_str).collect();
    let held: HashSet<&str> = held.iter().map(String::as_str).collect();
    records
        .iter()
        .map(|package| {
            (
                package.name.clone(),
                Inst {
                    version: package.version.clone(),
                    explicit: explicit.contains(package.name.as_str()),
                    held: held.contains(package.name.as_str()),
                    at: at(&package.name),
                },
            )
        })
        .collect()
}

/// Moves the pre-rename state files into the current data directory.
///
/// Versions before the rename kept their state under a `khaxbps` sibling of
/// the current directory. On the first start afterwards — when the new
/// directory does not exist yet — this moves the queue, the mirror and the
/// preferences over, so they survive the rename. The old `installed.json` is
/// left behind on purpose: the installed set is read from xbps now. Best
/// effort by design: every error is swallowed and the store starts empty,
/// exactly as on a machine that never had the old directory.
fn migrate_legacy(new_dir: &Path) {
    let Some(old_dir) = legacy_data_dir() else {
        return;
    };
    migrate_legacy_at(new_dir, &old_dir);
}

/// [`migrate_legacy`] against an explicit pair of directories, so the tests
/// can drive it with temp paths.
///
/// Copies each state file before removing it from `old_dir`: a directory rename
/// could not cross a filesystem boundary, and a half-moved state must never
/// lose the only complete copy. The legacy directory itself is left in place —
/// it may hold files this store does not own.
fn migrate_legacy_at(new_dir: &Path, old_dir: &Path) {
    // A new directory that already exists wins: a second start, or a fresh
    // store someone created next to the old one, must not be overwritten.
    if !old_dir.is_dir() || new_dir.exists() {
        return;
    }
    if ensure_dir(new_dir).is_err() {
        return;
    }
    for file in STATE_FILES {
        let from = old_dir.join(file);
        let to = new_dir.join(file);
        if from.is_file() && fs::copy(&from, &to).is_ok() {
            let _ = fs::remove_file(&from);
        }
    }
}

/// Creates `dir` if it does not exist yet and makes sure it is private (0700).
///
/// The mode is enforced on every load and save, not only when the directory is
/// created: a `~/.local/share/kxbps` restored from a backup, migrated from
/// the legacy `khaxbps` directory or created by hand can be 0755, and the
/// state files written into it would then be world-readable. `create_dir_all`
/// is subject to the umask as well, so the explicit `set_permissions` covers
/// both paths.
fn ensure_dir(dir: &Path) -> Result<(), StoreError> {
    if !dir.is_dir() {
        fs::create_dir_all(dir).map_err(|source| StoreError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    // Failing to tighten a directory we do not own is not worth an error: the
    // save below reports the real problem if the directory is unusable.
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    Ok(())
}

/// Reads one JSON file, returning `None` for a missing, unreadable or
/// unparsable file so the caller keeps its default.
fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_json<T: Serialize>(dir: &Path, file: &str, value: &T) -> Result<(), StoreError> {
    let path = dir.join(file);
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|source| StoreError::Json {
        path: path.clone(),
        source,
    })?;
    bytes.push(b'\n');
    write_atomic(&path, &bytes)
}

/// Writes `bytes` to `path` through `<path>.tmp`, so the target is replaced in
/// one `rename` and never observed half-written.
///
/// The temporary file is removed again on any failure, so a failed save does
/// not leave `.tmp` litter behind for the next load to trip over.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);

    let result = (|| -> std::io::Result<()> {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();

    result.map_err(|source| {
        let _ = fs::remove_file(&tmp);
        StoreError::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PlanAction, PlanItem, PlanReason};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQ: AtomicU32 = AtomicU32::new(0);

    /// A private directory under the system temp dir, removed on drop.
    ///
    /// The crate has no `tempfile` dependency and must not gain one, so the
    /// name carries the process id plus a per-process counter.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "kxbps-store-{tag}-{}-{}",
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

    fn file_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("read dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// One installed entry, as the Qt layer would have cached it after a read.
    fn put(store: &mut Store, name: &str, version: &str, explicit: bool, at: i64) {
        store.installed.insert(
            name.to_string(),
            Inst {
                version: version.to_string(),
                explicit,
                held: false,
                at,
            },
        );
    }

    #[test]
    fn missing_files_yield_the_fresh_state() {
        let dir = TempDir::new("missing");
        let store = Store::load_at(dir.path()).expect("load");
        assert_eq!(store, Store::fresh());
        assert_eq!(store.mirror, DEFAULT_MIRROR);
        assert_eq!(store.prefs, default_prefs());
        assert_eq!(store.synced_at, None);
        assert!(!store.is_dirty());
    }

    #[test]
    fn state_survives_a_save_and_load_round_trip() {
        let dir = TempDir::new("roundtrip");
        let mut store = Store::fresh();
        put(&mut store, "vim", "9.1.1700_1", true, 7);
        store.toggle_queue("vim", QueueAction::Install);
        store.toggle_queue("htop", QueueAction::Remove);
        assert!(store.toggle_hold("vim"));
        store.set_mirror("https://mirror.example/void/");
        store.set_pref_notify_on_start(false);
        store.set_pref_desktop_notifications(true);
        store.set_pref_dismissed_version("42");
        assert!(store.is_dirty());

        store.save_at(dir.path()).expect("save");
        assert!(!store.is_dirty());

        let loaded = Store::load_at(dir.path()).expect("load");
        assert_eq!(loaded.queue, store.queue);
        assert_eq!(loaded.mirror, "https://mirror.example/void/");
        assert_eq!(loaded.prefs.dismissed, "42");
        assert!(
            loaded.installed.is_empty(),
            "the installed set is read from xbps, never from disk"
        );
        assert!(!loaded.is_dirty());
    }

    #[test]
    fn save_writes_all_three_files_and_leaves_no_tmp_behind() {
        let dir = TempDir::new("atomic");
        let mut store = Store::fresh();
        store.save_at(dir.path()).expect("save");
        assert_eq!(
            file_names(dir.path()),
            vec!["mirror.json", "prefs.json", "queue.json"]
        );

        // A second save replaces the files in place, still without litter.
        store.set_mirror("https://mirror.example/void/");
        store.save_at(dir.path()).expect("second save");
        assert_eq!(file_names(dir.path()).len(), 3);
    }

    #[test]
    fn corrupt_prefs_falls_back_to_defaults_and_keeps_the_queue() {
        let dir = TempDir::new("corrupt-prefs");
        let queue = vec![QueueItem {
            name: "vim".to_string(),
            action: QueueAction::Install,
        }];
        write_json(dir.path(), QUEUE_FILE, &queue).expect("seed queue");
        fs::write(dir.path().join(PREFS_FILE), br#"{"notifyOnStart": tru"#).expect("corrupt prefs");

        let mut store = Store::load_at(dir.path()).expect("load");
        assert_eq!(store.prefs, default_prefs(), "broken prefs fall back");
        assert_eq!(store.queue, queue, "the queue survives");

        // Saving the repaired state must not cost the surviving file either.
        store.save_at(dir.path()).expect("save");
        assert_eq!(file_names(dir.path()).len(), 3);
        let reloaded = Store::load_at(dir.path()).expect("reload");
        assert_eq!(reloaded.queue, queue);
        assert_eq!(reloaded.prefs, default_prefs());
    }

    #[test]
    fn leftover_tmp_file_from_a_crash_is_ignored() {
        let dir = TempDir::new("stale-tmp");
        let prefs = Prefs {
            notify_on_start: false,
            desktop: true,
            dismissed: "7".to_string(),
        };
        write_json(dir.path(), PREFS_FILE, &prefs).expect("seed prefs");
        // A writer that died before its rename leaves a truncated temp file.
        let stale = dir.path().join(format!("{PREFS_FILE}.tmp"));
        fs::write(&stale, b"{\"notifyOn").expect("stale tmp");

        let mut store = Store::load_at(dir.path()).expect("load");
        assert_eq!(store.prefs, prefs, "the good file wins over the leftover");

        store.save_at(dir.path()).expect("save");
        assert!(!stale.exists(), "the next save cleans the leftover up");
    }

    #[test]
    fn blank_mirror_falls_back_to_the_default() {
        let dir = TempDir::new("mirror");
        fs::write(dir.path().join(MIRROR_FILE), br#"{"url":""}"#).expect("seed mirror");
        let store = Store::load_at(dir.path()).expect("load");
        assert_eq!(store.mirror, DEFAULT_MIRROR);
    }

    #[test]
    fn save_creates_a_private_data_directory() {
        let dir = TempDir::new("mkdir");
        let target = dir.path().join("nested").join("kxbps");
        let mut store = Store::fresh();
        store.save_at(&target).expect("save");
        assert!(target.is_dir());
        let mode = fs::metadata(&target)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "state directory is private");
    }

    #[cfg(unix)]
    #[test]
    fn pre_existing_permissive_directory_is_tightened_to_0700() {
        let dir = TempDir::new("tighten");
        let target = dir.path().join("kxbps");
        fs::create_dir(&target).expect("pre-create dir");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).expect("loosen");
        let mode = |path: &Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode(&target), 0o755, "precondition: world-readable");

        // Loading is enough: the directory is tightened before anything is read.
        let mut store = Store::load_at(&target).expect("load");
        assert_eq!(mode(&target), 0o700, "load_at tightens an existing dir");

        // Saving tightens it again if something loosened it in the meantime.
        fs::set_permissions(&target, fs::Permissions::from_mode(0o750)).expect("loosen again");
        store.save_at(&target).expect("save");
        assert_eq!(mode(&target), 0o700, "save_at tightens an existing dir");
    }

    #[test]
    fn migrate_legacy_moves_the_state_files_into_a_private_new_directory() {
        let root = TempDir::new("migrate");
        let old_dir = root.path().join("khaxbps");
        fs::create_dir_all(&old_dir).expect("legacy dir");
        let contents: Vec<(&str, String)> = STATE_FILES
            .iter()
            .map(|name| (*name, format!("legacy content of {name}")))
            .collect();
        for (name, bytes) in &contents {
            fs::write(old_dir.join(name), bytes).expect("write state file");
        }
        let new_dir = root.path().join("kxbps");
        assert!(!new_dir.exists(), "precondition: no new directory yet");

        migrate_legacy_at(&new_dir, &old_dir);

        for (name, bytes) in &contents {
            assert_eq!(
                fs::read_to_string(new_dir.join(name)).expect("migrated file"),
                *bytes,
                "{name} moved across"
            );
            assert!(!old_dir.join(name).exists(), "{name} was moved, not copied");
        }
        let mode = fs::metadata(&new_dir)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "the migrated directory is private");
        assert!(
            old_dir.is_dir(),
            "the legacy directory itself is left in place"
        );

        // Idempotent: with the new directory in place, a second run is a no-op.
        migrate_legacy_at(&new_dir, &old_dir);
        for (name, bytes) in &contents {
            assert_eq!(
                fs::read_to_string(new_dir.join(name)).expect("file survives the second run"),
                *bytes,
                "{name} untouched by the second run"
            );
        }
    }

    #[test]
    fn migrate_legacy_leaves_an_existing_new_directory_alone() {
        let root = TempDir::new("migrate-skip");
        let old_dir = root.path().join("khaxbps");
        fs::create_dir_all(&old_dir).expect("legacy dir");
        fs::write(old_dir.join(QUEUE_FILE), "legacy").expect("write legacy file");
        let new_dir = root.path().join("kxbps");
        fs::create_dir_all(&new_dir).expect("new dir");
        fs::write(new_dir.join(QUEUE_FILE), "current").expect("write current file");

        migrate_legacy_at(&new_dir, &old_dir);

        assert_eq!(
            fs::read_to_string(new_dir.join(QUEUE_FILE)).expect("current file"),
            "current",
            "an existing new directory is never overwritten"
        );
        assert!(
            old_dir.join(QUEUE_FILE).is_file(),
            "the legacy file is not consumed"
        );
    }

    /// The old installed database is not part of the migration: the installed
    /// set comes from xbps, and a stale file must not come back with it.
    #[test]
    fn migrate_legacy_leaves_the_old_installed_file_behind() {
        let root = TempDir::new("migrate-installed");
        let old_dir = root.path().join("khaxbps");
        fs::create_dir_all(&old_dir).expect("legacy dir");
        fs::write(old_dir.join("installed.json"), "{}").expect("legacy installed");
        let new_dir = root.path().join("kxbps");

        migrate_legacy_at(&new_dir, &old_dir);

        assert!(!new_dir.join("installed.json").exists());
        assert!(old_dir.join("installed.json").is_file());
    }

    #[test]
    fn migrate_legacy_without_a_legacy_directory_creates_nothing() {
        let root = TempDir::new("migrate-none");
        let old_dir = root.path().join("khaxbps");
        let new_dir = root.path().join("kxbps");

        migrate_legacy_at(&new_dir, &old_dir);

        assert!(!new_dir.exists(), "no legacy state, no new directory");
    }

    #[test]
    fn toggle_queue_matches_the_web_app() {
        let mut store = Store::fresh();
        store.toggle_queue("vim", QueueAction::Install);
        store.toggle_queue("htop", QueueAction::Remove);
        assert_eq!(
            store.queue,
            vec![
                QueueItem {
                    name: "vim".to_string(),
                    action: QueueAction::Install,
                },
                QueueItem {
                    name: "htop".to_string(),
                    action: QueueAction::Remove,
                },
            ]
        );

        // Same action again unqueues the package.
        store.toggle_queue("vim", QueueAction::Install);
        assert_eq!(
            store.queue,
            vec![QueueItem {
                name: "htop".to_string(),
                action: QueueAction::Remove,
            }]
        );

        // A different action replaces the entry and moves it to the end.
        store.toggle_queue("htop", QueueAction::Install);
        assert_eq!(
            store.queue,
            vec![QueueItem {
                name: "htop".to_string(),
                action: QueueAction::Install,
            }]
        );

        store.remove_queued("htop");
        assert!(store.queue.is_empty());
        store.toggle_queue("htop", QueueAction::Install);
        store.clear_queue();
        assert!(store.queue.is_empty());
    }

    #[test]
    fn apply_plan_rewrites_installed_queue_and_synced_at() {
        let mut store = Store::fresh();
        put(&mut store, "vim", "9.1.1700_1", true, 7);
        store.toggle_queue("vim", QueueAction::Install);
        store.toggle_queue("htop", QueueAction::Remove);
        let plan = Plan {
            items: vec![
                PlanItem {
                    name: "vim".to_string(),
                    action: PlanAction::Update,
                    from: Some("9.1.1700_1".to_string()),
                    to: Some("9.2.0_1".to_string()),
                    reason: PlanReason::Requested,
                    download: 10,
                    size: 20,
                },
                PlanItem {
                    name: "newpkg".to_string(),
                    action: PlanAction::Install,
                    from: None,
                    to: Some("1.0_1".to_string()),
                    reason: PlanReason::Dependency,
                    download: 1,
                    size: 2,
                },
            ],
            blocked: Vec::new(),
        };

        store.apply_plan(&plan, 5_000);

        assert_eq!(store.installed["vim"].version, "9.2.0_1");
        assert_eq!(
            store.installed["vim"].at, 7,
            "an update keeps the install timestamp"
        );
        assert_eq!(store.installed["newpkg"].at, 5_000);
        assert!(!store.installed["newpkg"].explicit);
        assert_eq!(
            store.queue,
            vec![QueueItem {
                name: "htop".to_string(),
                action: QueueAction::Remove,
            }],
            "planned packages leave the queue, untouched ones stay"
        );
        assert_eq!(store.synced_at, Some(5_000));
        assert!(store.is_dirty());
    }

    #[test]
    fn apply_plan_ignores_a_blocked_plan() {
        let mut store = Store::fresh();
        let before = store.installed.clone();
        let plan = Plan {
            items: Vec::new(),
            blocked: vec![crate::types::Blocked {
                name: "glibc".to_string(),
                needed_by: vec!["vim".to_string()],
            }],
        };

        store.apply_plan(&plan, 5_000);

        assert_eq!(store.installed, before);
        assert_eq!(store.synced_at, None);
        assert!(!store.is_dirty());
    }

    #[test]
    fn reset_clears_the_users_choices_and_keeps_the_desktop_preference() {
        let mut store = Store::fresh();
        put(&mut store, "vim", "9.1.1700_1", true, 7);
        store.toggle_queue("vim", QueueAction::Install);
        store.set_mirror("https://mirror.example/void/");
        store.set_pref_notify_on_start(false);
        store.set_pref_desktop_notifications(true);
        store.set_pref_dismissed_version("42");
        store.synced_at = Some(9);

        store.reset();

        assert!(
            store.installed.is_empty(),
            "reset restores no packages; the next read fills the cache"
        );
        assert_eq!(store.mirror, DEFAULT_MIRROR);
        assert!(store.queue.is_empty());
        assert_eq!(
            store.prefs,
            Prefs {
                notify_on_start: true,
                desktop: true,
                dismissed: String::new(),
            }
        );
        assert_eq!(store.synced_at, None);
        assert!(store.is_dirty());
    }

    #[test]
    fn toggle_hold_only_touches_installed_packages() {
        let mut store = Store::fresh();
        put(&mut store, "vim", "9.2.1031_1", true, 7);
        assert!(store.toggle_hold("vim"));
        assert!(store.installed["vim"].held);
        assert!(!store.toggle_hold("vim"));
        assert!(!store.installed["vim"].held);
        assert!(!store.toggle_hold("definitely-not-installed"));
    }

    /// One installed record as `xbps-query -l` printed it.
    fn record(pkgver: &str) -> InstalledPackage {
        let (name, version) = crate::xbps::split_pkgver(pkgver).expect("pkgver splits");
        InstalledPackage {
            name: name.to_owned(),
            version: version.to_owned(),
            pkgver: pkgver.to_owned(),
            state: crate::xbps::InstallState::Installed,
            description: String::new(),
        }
    }

    /// One dry-run line of `xbps-install -un`.
    fn update(pkgver: &str) -> UpdateEntry {
        let (name, version) = crate::xbps::split_pkgver(pkgver).expect("pkgver splits");
        UpdateEntry {
            name: name.to_owned(),
            version: version.to_owned(),
            pkgver: pkgver.to_owned(),
            action: crate::xbps::XbpsAction::Update,
            arch: "x86_64".to_owned(),
            repository: "https://repo.example/current".to_owned(),
            installed_size: 10,
            download_size: 20,
        }
    }

    #[test]
    fn apply_snapshot_replaces_the_cached_set_with_the_real_records() {
        let mut store = Store::fresh();
        put(&mut store, "vim", "9.1.1700_1", true, 7);

        store.apply_snapshot(
            &[record("Neovim-custom-0.11.0_1")],
            &["Neovim-custom".to_owned()],
            &[],
            Vec::new(),
        );

        assert_eq!(
            store.installed.keys().collect::<Vec<_>>(),
            ["Neovim-custom"],
            "the previous cache is replaced, not merged with"
        );
        let inst = &store.installed["Neovim-custom"];
        assert_eq!(inst.version, "0.11.0_1");
        assert!(inst.explicit, "`-m` listed it");
        assert!(!inst.held, "`-H` did not list it");
        assert_eq!(inst.at, 0, "xbps gives no install date in bulk");
        assert!(store.is_dirty(), "the new set has to be saved");
    }

    #[test]
    fn apply_snapshot_derives_explicit_and_held_from_the_list_modes() {
        let mut store = Store::fresh();
        store.apply_snapshot(
            &[record("vim-9.2.1031_1"), record("linux-6.18.63_1")],
            &["vim".to_owned()],
            &["linux".to_owned()],
            Vec::new(),
        );
        assert!(store.installed["vim"].explicit);
        assert!(!store.installed["linux"].explicit);
        assert!(store.installed["linux"].held);
        assert!(!store.installed["vim"].held);
    }

    #[test]
    fn apply_snapshot_keeps_the_install_date_the_store_already_knew() {
        let mut store = Store::fresh();
        put(&mut store, "vim", "9.1.1700_1", true, 7);
        store.apply_snapshot(
            &[record("vim-9.2.2000_1"), record("ark-25.08.0_1")],
            &["vim".to_owned()],
            &[],
            Vec::new(),
        );
        assert_eq!(
            store.installed["vim"].at, 7,
            "a re-read must not forget when vim was installed"
        );
        assert_eq!(store.installed["ark"].at, 0);
    }

    #[test]
    fn apply_snapshot_replaces_the_pending_updates() {
        let mut store = Store::fresh();
        assert!(store.pending_updates().is_empty());

        store.apply_snapshot(
            &[record("vim-9.2.1031_1")],
            &["vim".to_owned()],
            &[],
            vec![update("vim-9.2.2000_1")],
        );

        assert_eq!(store.pending_updates().len(), 1);
        assert_eq!(
            store.update_of("vim").map(|entry| entry.version.as_str()),
            Some("9.2.2000_1")
        );
        assert!(store.update_of("ark").is_none());

        store.apply_snapshot(&[record("vim-9.2.2000_1")], &[], &[], Vec::new());
        assert!(
            store.pending_updates().is_empty(),
            "after the transaction xbps reports no update"
        );
    }

    #[test]
    fn reset_drops_the_pending_updates_with_the_cached_set() {
        let mut store = Store::fresh();
        store.apply_snapshot(
            &[record("vim-9.2.1031_1")],
            &[],
            &[],
            vec![update("vim-9.2.2000_1")],
        );

        store.reset();

        assert!(store.pending_updates().is_empty());
        assert!(store.installed.is_empty());
    }
}
