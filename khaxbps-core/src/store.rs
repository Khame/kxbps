//! Persistent desktop state: the installed database, the transaction queue,
//! the selected mirror and the user preferences.
//!
//! The web app keeps all of it in one `localStorage` snapshot under
//! `qxbps-v1`; this port keeps the same four concerns in four JSON files under
//! `$XDG_DATA_HOME/khaxbps/` (fallback `$HOME/.local/share/khaxbps/`), so a
//! damaged file can only cost one concern and the files stay diffable:
//!
//! | File | Payload |
//! | --- | --- |
//! | `installed.json` | `{ "<name>": Inst }` |
//! | `queue.json` | `[QueueItem]` |
//! | `prefs.json` | `Prefs` |
//! | `mirror.json` | `{ "url": "https://…" }` |
//!
//! Every write goes to `<file>.tmp` in the same directory, is fsynced and then
//! renamed over the target, so a crash mid-write leaves either the previous
//! file or the new one — never a truncated one. A missing or unparsable file
//! falls back to that concern's default and nothing else is touched; a broken
//! `prefs.json` must not cost the installed database.
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

use crate::catalog::Catalog;
use crate::engine::{Plan, apply_installed};
use crate::types::{Inst, InstalledMap, Prefs, QueueAction, QueueItem};

/// Mirror used until the user picks another one (`DEFAULT_MIRROR` in the web
/// app's `mirrors.ts`).
pub const DEFAULT_MIRROR: &str = "https://repo-default.voidlinux.org/";

/// Install timestamp of every package in the seed image, in epoch
/// milliseconds: `Date.UTC(2026, 9, 2, 8, 0, 0)` in `catalog.ts`.
pub const SEED_AT: i64 = 1_790_928_000_000;

/// Directory created below the XDG data home.
const DATA_DIR: &str = "khaxbps";
const INSTALLED_FILE: &str = "installed.json";
const QUEUE_FILE: &str = "queue.json";
const PREFS_FILE: &str = "prefs.json";
const MIRROR_FILE: &str = "mirror.json";

/// Packages installed on purpose in the seed image (`EXPLICIT` in `catalog.ts`).
const EXPLICIT: [&str; 20] = [
    "base-system",
    "linux",
    "linux-headers",
    "linux-firmware",
    "plasma-desktop",
    "firefox",
    "git",
    "neovim",
    "htop",
    "tmux",
    "zsh",
    "sudo",
    "vim",
    "nano",
    "openssh",
    "curl",
    "wget",
    "xmirror",
    "konsole",
    "dolphin",
];

/// Dependency-only packages the seed image carries without needing them
/// (`EXTRA_AUTO` in `catalog.ts`).
const EXTRA_AUTO: [&str; 1] = ["cowsay"];

/// Versions the seed image pins below the repository versions, so a fresh
/// install has updates to offer (`VERSION_OVERRIDE` in `catalog.ts`).
const VERSION_OVERRIDE: [(&str, &str); 9] = [
    ("linux", "6.12.48_1"),
    ("linux-headers", "6.12.48_1"),
    ("linux-firmware", "20250812_1"),
    ("xbps", "0.59.6_1"),
    ("curl", "8.12.1_1"),
    ("git", "2.48.1_1"),
    ("neovim", "0.10.4_1"),
    ("firefox", "135.0.1_1"),
    ("mesa", "25.0.2_1"),
];

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
/// The five public fields mirror the brief's `Store` shape; `baseline` keeps
/// the seed image around so [`Store::reset`] needs no catalog argument, and
/// `dirty` tells the Qt layer whether a [`Store::save`] is still owed.
#[derive(Debug, Clone)]
pub struct Store {
    pub installed: InstalledMap,
    pub queue: Vec<QueueItem>,
    pub mirror: String,
    pub prefs: Prefs,
    /// When the repository index was last refreshed, epoch milliseconds.
    /// Memory-only, like the web app's `syncedAt`.
    pub synced_at: Option<i64>,
    /// The seed image this store was created from; restored by [`Store::reset`].
    baseline: InstalledMap,
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

/// Builds the seed image exactly like the web app's `freshInstall()`.
///
/// Every name in [`EXPLICIT`] is installed as explicit, its catalog
/// dependencies as implicit, then [`EXTRA_AUTO`] as implicit. Versions come
/// from [`VERSION_OVERRIDE`] when present and from the catalog otherwise, and
/// every entry is stamped [`SEED_AT`].
///
/// Names the catalog does not know are skipped instead of aborting the whole
/// image: the catalog is generated and validated, and a single stale name must
/// not leave the user without a system. A dependency cycle is broken by the
/// `visiting` guard (the web app would recurse forever).
pub fn fresh_install(catalog: &Catalog) -> InstalledMap {
    let mut installed = InstalledMap::new();
    let mut visiting = HashSet::new();
    for name in EXPLICIT {
        add_seed(catalog, &mut installed, &mut visiting, name, true);
    }
    for name in EXTRA_AUTO {
        add_seed(catalog, &mut installed, &mut visiting, name, false);
    }
    installed
}

fn add_seed(
    catalog: &Catalog,
    installed: &mut InstalledMap,
    visiting: &mut HashSet<String>,
    name: &str,
    explicit: bool,
) {
    if let Some(existing) = installed.get_mut(name) {
        // A package reached twice is explicit if either path wanted it.
        if explicit {
            existing.explicit = true;
        }
        return;
    }
    if !visiting.insert(name.to_string()) {
        return;
    }
    let Some(pkg) = catalog.by_name(name) else {
        visiting.remove(name);
        return;
    };
    let version = seed_version(name, &pkg.version);
    let depends = pkg.depends.clone();
    for dep in depends {
        add_seed(catalog, installed, visiting, &dep, false);
    }
    installed.insert(
        name.to_string(),
        Inst {
            version,
            explicit,
            held: false,
            at: SEED_AT,
        },
    );
    visiting.remove(name);
}

fn seed_version(name: &str, catalog_version: &str) -> String {
    VERSION_OVERRIDE
        .iter()
        .find(|(overridden, _)| *overridden == name)
        .map_or_else(
            || catalog_version.to_string(),
            |(_, version)| (*version).to_string(),
        )
}

/// `$XDG_DATA_HOME/khaxbps`, or `$HOME/.local/share/khaxbps` when the variable
/// is unset, empty or relative (the XDG spec says to ignore relative paths).
pub fn data_dir() -> Result<PathBuf, StoreError> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        let dir = PathBuf::from(dir);
        if dir.is_absolute() {
            return Ok(dir.join(DATA_DIR));
        }
    }
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .ok_or(StoreError::NoDataDir)?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join(DATA_DIR))
}

impl Store {
    /// A store holding the seed image and the default preferences.
    pub fn fresh(catalog: &Catalog) -> Self {
        let baseline = fresh_install(catalog);
        Store {
            installed: baseline.clone(),
            queue: Vec::new(),
            mirror: DEFAULT_MIRROR.to_string(),
            prefs: default_prefs(),
            synced_at: None,
            baseline,
            dirty: false,
        }
    }

    /// Loads the store from the XDG data directory.
    pub fn load(catalog: &Catalog) -> Result<Self, StoreError> {
        Self::load_at(&data_dir()?, catalog)
    }

    /// Loads the store from `dir`, creating it if needed.
    ///
    /// The catalog is only needed to rebuild the seed image: for a missing or
    /// unparsable `installed.json`, and as the target of [`Store::reset`].
    pub fn load_at(dir: &Path, catalog: &Catalog) -> Result<Self, StoreError> {
        ensure_dir(dir)?;
        let mut store = Store::fresh(catalog);

        if let Some(installed) = read_json(&dir.join(INSTALLED_FILE)) {
            store.installed = installed;
        }
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
    /// All four files are rewritten; each one is replaced atomically, so a
    /// failure halfway leaves the files it already wrote in their new state
    /// and the rest in their old one — never a half-written file.
    pub fn save_at(&mut self, dir: &Path) -> Result<(), StoreError> {
        ensure_dir(dir)?;
        write_json(dir, INSTALLED_FILE, &self.installed)?;
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

    /// Restores the seed image (`resetSystem` in `store.ts`).
    ///
    /// The queue and the mirror go back to their defaults and the
    /// notification preferences are reset, except for the desktop-notification
    /// switch, which the web app keeps across a reset.
    pub fn reset(&mut self) {
        self.installed = self.baseline.clone();
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
}

/// Creates `dir` if it does not exist yet and makes sure it is private (0700).
///
/// The mode is enforced on every load and save, not only when the directory is
/// created: a `~/.local/share/khaxbps` left behind by an older run, restored
/// from a backup or created by hand can be 0755, and the state files written
/// into it would then be world-readable. `create_dir_all` is subject to the
/// umask as well, so the explicit `set_permissions` covers both paths.
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

    const CATALOG_JSON: &str = include_str!("../../khaxbps-qt/resources/catalog.json");

    fn catalog() -> Catalog {
        Catalog::load_from_str(CATALOG_JSON).expect("bundled catalog parses")
    }

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
                "khaxbps-store-{tag}-{}-{}",
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

    /// The web app's `freshInstall()` output, dumped from `catalog.ts` with
    /// Node: `[name, explicit, version]`, sorted by name.
    const WEB_SEED: [(&str, bool, &str); 56] = [
        ("NetworkManager", false, "1.52.0_1"),
        ("base-system", true, "0.114_1"),
        ("bash", false, "5.2.37_1"),
        ("breeze", false, "6.4.5_1"),
        ("ca-certificates", false, "20250812_1"),
        ("coreutils", false, "9.6_1"),
        ("cowsay", false, "3.8.4_1"),
        ("curl", true, "8.12.1_1"),
        ("dbus", false, "1.16.2_1"),
        ("dhcpcd", false, "10.2.2_1"),
        ("dolphin", true, "25.08.2_1"),
        ("e2fsprogs", false, "1.47.2_1"),
        ("elogind", false, "255.17_1"),
        ("findutils", false, "4.10.0_1"),
        ("firefox", true, "135.0.1_1"),
        ("gawk", false, "5.3.1_1"),
        ("git", true, "2.48.1_1"),
        ("glibc", false, "2.41_1"),
        ("grep", false, "3.11_2"),
        ("gzip", false, "1.14_1"),
        ("htop", true, "3.4.1_1"),
        ("iproute2", false, "6.14.0_1"),
        ("kate", false, "25.08.2_1"),
        ("kbd", false, "2.6.4_1"),
        ("konsole", true, "25.08.2_1"),
        ("kwin", false, "6.4.5_1"),
        ("linux", true, "6.12.48_1"),
        ("linux-firmware", true, "20250812_1"),
        ("linux-headers", true, "6.12.48_1"),
        ("mesa", false, "25.0.2_1"),
        ("nano", true, "8.4_1"),
        ("ncurses", false, "6.5_1"),
        ("neovim", true, "0.10.4_1"),
        ("openssh", true, "9.9p2_1"),
        ("openssl", false, "3.4.1_1"),
        ("pipewire", false, "1.4.7_1"),
        ("plasma-desktop", true, "6.4.5_1"),
        ("plasma-workspace", false, "6.4.5_1"),
        ("qt6-base", false, "6.9.2_1"),
        ("readline", false, "8.2.13_1"),
        ("runit", false, "2.1.2_15"),
        ("sddm", false, "0.21.0_4"),
        ("sed", false, "4.9_1"),
        ("shadow", false, "4.16.0_1"),
        ("sudo", true, "1.9.17p1_1"),
        ("tar", false, "1.35_1"),
        ("tmux", true, "3.5a_1"),
        ("tzdata", false, "2025b_1"),
        ("util-linux", false, "2.40.4_1"),
        ("vim", true, "9.1.1700_1"),
        ("wget", true, "1.25.0_1"),
        ("xbps", false, "0.59.6_1"),
        ("xmirror", true, "0.5_1"),
        ("xz", false, "5.8.1_1"),
        ("zlib", false, "1.3.1_1"),
        ("zsh", true, "5.9_4"),
    ];

    fn seed_triples(installed: &InstalledMap) -> Vec<(String, bool, String)> {
        let mut triples: Vec<(String, bool, String)> = installed
            .iter()
            .map(|(name, inst)| (name.clone(), inst.explicit, inst.version.clone()))
            .collect();
        triples.sort();
        triples
    }

    #[test]
    fn fresh_state_matches_the_web_app_seed() {
        let installed = fresh_install(&catalog());
        let expected: Vec<(String, bool, String)> = WEB_SEED
            .iter()
            .map(|(name, explicit, version)| {
                ((*name).to_string(), *explicit, (*version).to_string())
            })
            .collect();
        assert_eq!(seed_triples(&installed), expected);
        assert_eq!(installed.len(), 56);
        assert_eq!(
            installed.values().filter(|inst| inst.explicit).count(),
            20,
            "the seed has exactly the web app's 20 explicit packages"
        );
        assert!(
            installed
                .values()
                .all(|inst| inst.at == SEED_AT && !inst.held)
        );
    }

    #[test]
    fn missing_files_yield_the_fresh_state() {
        let dir = TempDir::new("missing");
        let store = Store::load_at(dir.path(), &catalog()).expect("load");
        assert_eq!(store, Store::fresh(&catalog()));
        assert_eq!(store.mirror, DEFAULT_MIRROR);
        assert_eq!(store.prefs, default_prefs());
        assert_eq!(store.synced_at, None);
        assert!(!store.is_dirty());
    }

    #[test]
    fn state_survives_a_save_and_load_round_trip() {
        let dir = TempDir::new("roundtrip");
        let catalog = catalog();
        let mut store = Store::fresh(&catalog);
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

        let loaded = Store::load_at(dir.path(), &catalog).expect("load");
        assert_eq!(loaded, store);
        assert!(loaded.installed["vim"].held);
        assert_eq!(loaded.mirror, "https://mirror.example/void/");
        assert_eq!(loaded.prefs.dismissed, "42");
        assert!(!loaded.is_dirty());
    }

    #[test]
    fn save_writes_all_four_files_and_leaves_no_tmp_behind() {
        let dir = TempDir::new("atomic");
        let mut store = Store::fresh(&catalog());
        store.save_at(dir.path()).expect("save");
        assert_eq!(
            file_names(dir.path()),
            vec!["installed.json", "mirror.json", "prefs.json", "queue.json"]
        );

        // A second save replaces the files in place, still without litter.
        store.set_mirror("https://mirror.example/void/");
        store.save_at(dir.path()).expect("second save");
        assert_eq!(file_names(dir.path()).len(), 4);
    }

    #[test]
    fn corrupt_prefs_falls_back_to_defaults_and_keeps_installed() {
        let dir = TempDir::new("corrupt-prefs");
        let catalog = catalog();
        let mut installed = InstalledMap::new();
        installed.insert(
            "vim".to_string(),
            Inst {
                version: "9.1.1700_1".to_string(),
                explicit: true,
                held: false,
                at: 7,
            },
        );
        write_json(dir.path(), INSTALLED_FILE, &installed).expect("seed installed");
        fs::write(dir.path().join(PREFS_FILE), br#"{"notifyOnStart": tru"#).expect("corrupt prefs");

        let mut store = Store::load_at(dir.path(), &catalog).expect("load");
        assert_eq!(store.prefs, default_prefs(), "broken prefs fall back");
        assert_eq!(store.installed, installed, "installed database survives");

        // Saving the repaired state must not cost the surviving file either.
        store.save_at(dir.path()).expect("save");
        assert_eq!(file_names(dir.path()).len(), 4);
        let reloaded = Store::load_at(dir.path(), &catalog).expect("reload");
        assert_eq!(reloaded.installed, installed);
        assert_eq!(reloaded.prefs, default_prefs());
    }

    #[test]
    fn corrupt_installed_falls_back_without_touching_the_queue() {
        let dir = TempDir::new("corrupt-installed");
        let queue = vec![QueueItem {
            name: "vim".to_string(),
            action: QueueAction::Install,
        }];
        write_json(dir.path(), QUEUE_FILE, &queue).expect("seed queue");
        // Valid JSON, wrong type: the map deserialisation must fail, not panic.
        fs::write(dir.path().join(INSTALLED_FILE), b"[1, 2]").expect("corrupt installed");

        let store = Store::load_at(dir.path(), &catalog()).expect("load");
        assert_eq!(store.installed, fresh_install(&catalog()));
        assert_eq!(store.queue, queue);
    }

    #[test]
    fn leftover_tmp_file_from_a_crash_is_ignored() {
        let dir = TempDir::new("stale-tmp");
        let catalog = catalog();
        let prefs = Prefs {
            notify_on_start: false,
            desktop: true,
            dismissed: "7".to_string(),
        };
        write_json(dir.path(), PREFS_FILE, &prefs).expect("seed prefs");
        // A writer that died before its rename leaves a truncated temp file.
        let stale = dir.path().join(format!("{PREFS_FILE}.tmp"));
        fs::write(&stale, b"{\"notifyOn").expect("stale tmp");

        let mut store = Store::load_at(dir.path(), &catalog).expect("load");
        assert_eq!(store.prefs, prefs, "the good file wins over the leftover");

        store.save_at(dir.path()).expect("save");
        assert!(!stale.exists(), "the next save cleans the leftover up");
    }

    #[test]
    fn blank_mirror_falls_back_to_the_default() {
        let dir = TempDir::new("mirror");
        fs::write(dir.path().join(MIRROR_FILE), br#"{"url":""}"#).expect("seed mirror");
        let store = Store::load_at(dir.path(), &catalog()).expect("load");
        assert_eq!(store.mirror, DEFAULT_MIRROR);
    }

    #[test]
    fn save_creates_a_private_data_directory() {
        let dir = TempDir::new("mkdir");
        let target = dir.path().join("nested").join("khaxbps");
        let mut store = Store::fresh(&catalog());
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
        let target = dir.path().join("khaxbps");
        fs::create_dir(&target).expect("pre-create dir");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).expect("loosen");
        let mode = |path: &Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode(&target), 0o755, "precondition: world-readable");

        // Loading is enough: the directory is tightened before anything is read.
        let mut store = Store::load_at(&target, &catalog()).expect("load");
        assert_eq!(mode(&target), 0o700, "load_at tightens an existing dir");

        // Saving tightens it again if something loosened it in the meantime.
        fs::set_permissions(&target, fs::Permissions::from_mode(0o750)).expect("loosen again");
        store.save_at(&target).expect("save");
        assert_eq!(mode(&target), 0o700, "save_at tightens an existing dir");
    }

    #[test]
    fn toggle_queue_matches_the_web_app() {
        let mut store = Store::fresh(&catalog());
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
        let mut store = Store::fresh(&catalog());
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
            store.installed["vim"].at, SEED_AT,
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
        let mut store = Store::fresh(&catalog());
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
    fn reset_restores_the_seed_image_and_keeps_the_desktop_preference() {
        let catalog = catalog();
        let mut store = Store::fresh(&catalog);
        store.toggle_queue("vim", QueueAction::Install);
        store.set_mirror("https://mirror.example/void/");
        store.set_pref_notify_on_start(false);
        store.set_pref_desktop_notifications(true);
        store.set_pref_dismissed_version("42");
        store.installed.remove("vim");
        store.synced_at = Some(9);

        store.reset();

        assert_eq!(store.installed, Store::fresh(&catalog).installed);
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
        let mut store = Store::fresh(&catalog());
        assert!(store.toggle_hold("vim"));
        assert!(store.installed["vim"].held);
        assert!(!store.toggle_hold("vim"));
        assert!(!store.installed["vim"].held);
        assert!(!store.toggle_hold("definitely-not-installed"));
    }
}
