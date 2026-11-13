//! The Settings preferences: three values that survive a restart.
//!
//! The store is the only owner of `prefs.json`, and
//! [`crate::installed_backend`] is the only owner of the store, so this backend
//! holds no persistence logic of its own. `set_owner` reads the block once
//! through the installed backend, and every write goes back through the same
//! shim functions — one read, two writes — which save the store once per
//! change. There is no KConfig here on purpose (Global Constraints): the state
//! is three fields and the store already replaces its files atomically.
//!
//! **Attach order.** `owner` must be assigned before the first preference
//! write. A write on a backend without an owner updates the cache and emits
//! `changed`, but there is no store to persist to; the next `set_owner` adopts
//! the store block and silently overwrites the value. In QML keep
//! `PrefsBackend { owner: installedBackend }` and do not set a preference in the
//! same object literal, or the literal's value is reverted.
//!
//! QML only ever sees the three properties. A write that matches the cached
//! value is confirmed against the store before it is dropped: `reset()` and a
//! finished transaction write `prefs.json` directly, so the cache can be stale
//! while the store has moved.

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QObjectMutPtr, QString};
use kxbps_core::{Prefs, Store};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qobjectmutptr.h");
        type QObjectMutPtr = cxx_qt_lib::QObjectMutPtr;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        // The store lives in the installed backend, so the preferences are read
        // and written through it instead of a second `Store`.
        include!("backend_shim.h");

        #[cxx_name = "kxbps_installed_prefs"]
        fn installed_prefs(owner: QObjectMutPtr) -> QString;

        #[cxx_name = "kxbps_installed_set_pref_bool"]
        fn installed_set_pref_bool(owner: QObjectMutPtr, key: &QString, value: bool);

        #[cxx_name = "kxbps_installed_set_pref_string"]
        fn installed_set_pref_string(owner: QObjectMutPtr, key: &QString, value: &QString);
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QObjectMutPtr, owner, READ, WRITE = set_owner, NOTIFY)]
        #[qproperty(bool, notify_on_start, READ, WRITE = set_notify_on_start, NOTIFY)]
        #[qproperty(
            bool,
            desktop_notifications,
            READ,
            WRITE = set_desktop_notifications,
            NOTIFY
        )]
        #[qproperty(
            QString,
            dismissed_version,
            READ,
            WRITE = set_dismissed_version,
            NOTIFY
        )]
        // Whether the Qt layer keeps an autostart entry for this app. The file
        // is written by `crate::autostart`, not by the store: this property is
        // the persisted decision, and `Main.qml` syncs the file to it.
        #[qproperty(
            bool,
            start_with_session,
            READ,
            WRITE = set_start_with_session,
            NOTIFY
        )]
        type PrefsBackend = super::PrefsBackendRust;

        fn set_owner(self: Pin<&mut PrefsBackend>, value: QObjectMutPtr);

        fn set_notify_on_start(self: Pin<&mut PrefsBackend>, value: bool);

        fn set_desktop_notifications(self: Pin<&mut PrefsBackend>, value: bool);

        fn set_dismissed_version(self: Pin<&mut PrefsBackend>, value: QString);

        fn set_start_with_session(self: Pin<&mut PrefsBackend>, value: bool);

        // Writes or removes the autostart entry so the file matches the
        // preference. Called by QML on every start and after every toggle; the
        // entry is the user's own file, so this needs no privilege.
        #[qinvokable]
        fn sync_autostart(self: &PrefsBackend);
    }

    impl cxx_qt::Constructor<()> for PrefsBackend {}
}

impl cxx_qt::Initialize for qobject::PrefsBackend {
    /// Nothing to initialise at construction: the store owner is attached by
    /// QML afterwards (`owner: installedBackend`).
    fn initialize(self: Pin<&mut Self>) {}
}

/// The keys the preferences cross the store boundary with.
///
/// They are the camelCase field names of [`Prefs`], i.e. exactly what
/// `prefs.json` holds, so a log line or a hand-edited file uses the same names
/// as the API.
pub mod keys {
    /// Announce updates when the window opens.
    pub const NOTIFY_ON_START: &str = "notifyOnStart";
    /// Also announce through the desktop notification service.
    pub const DESKTOP: &str = "desktop";
    /// The update signature the user dismissed.
    pub const DISMISSED: &str = "dismissed";
    /// Start with the session (the autostart entry).
    pub const START_WITH_SESSION: &str = "startWithSession";
}

/// One of the persisted preferences, i.e. one `prefs.json` key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    NotifyOnStart,
    Desktop,
    Dismissed,
    StartWithSession,
}

impl Field {
    /// The key the field is persisted under.
    pub fn key(self) -> &'static str {
        match self {
            Self::NotifyOnStart => keys::NOTIFY_ON_START,
            Self::Desktop => keys::DESKTOP,
            Self::Dismissed => keys::DISMISSED,
            Self::StartWithSession => keys::START_WITH_SESSION,
        }
    }

    /// The write that would reproduce the value `prefs` holds for this field.
    fn current(self, prefs: &Prefs) -> PrefWrite {
        match self {
            Self::NotifyOnStart => PrefWrite::NotifyOnStart(prefs.notify_on_start),
            Self::Desktop => PrefWrite::Desktop(prefs.desktop),
            Self::Dismissed => PrefWrite::Dismissed(prefs.dismissed.clone()),
            Self::StartWithSession => PrefWrite::StartWithSession(prefs.start_with_session),
        }
    }
}

/// One preference write, decoded from the key QML sent.
///
/// QML reaches the store with a key and a value rather than a whole [`Prefs`]
/// block, so a stale copy of the other fields can never overwrite a change made
/// elsewhere in the meantime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefWrite {
    NotifyOnStart(bool),
    Desktop(bool),
    Dismissed(String),
    StartWithSession(bool),
}

impl PrefWrite {
    /// The field this write targets.
    pub fn field(&self) -> Field {
        match self {
            Self::NotifyOnStart(_) => Field::NotifyOnStart,
            Self::Desktop(_) => Field::Desktop,
            Self::Dismissed(_) => Field::Dismissed,
            Self::StartWithSession(_) => Field::StartWithSession,
        }
    }

    /// The store key of this write.
    pub fn key(&self) -> &'static str {
        self.field().key()
    }

    /// Whether `prefs` already holds this value.
    pub fn is_noop(&self, prefs: &Prefs) -> bool {
        self.field().current(prefs) == *self
    }

    /// Applies the write to a plain block; used by the backend and the tests.
    pub fn apply(&self, prefs: &mut Prefs) {
        match self {
            Self::NotifyOnStart(value) => prefs.notify_on_start = *value,
            Self::Desktop(value) => prefs.desktop = *value,
            Self::Dismissed(value) => prefs.dismissed.clone_from(value),
            Self::StartWithSession(value) => prefs.start_with_session = *value,
        }
    }

    /// Applies the write through the store, which marks itself dirty.
    pub fn apply_to_store(&self, store: &mut Store) {
        match self {
            Self::NotifyOnStart(value) => store.set_pref_notify_on_start(*value),
            Self::Desktop(value) => store.set_pref_desktop_notifications(*value),
            Self::Dismissed(value) => store.set_pref_dismissed_version(value),
            Self::StartWithSession(value) => store.set_pref_start_with_session(*value),
        }
    }

    /// Sends the write to the store owner; a missing owner is a no-op.
    fn forward(&self, owner: QObjectMutPtr) {
        if owner.as_ptr().is_null() {
            return;
        }
        let key = QString::from(self.key());
        match self {
            Self::NotifyOnStart(value) | Self::Desktop(value) | Self::StartWithSession(value) => {
                qobject::installed_set_pref_bool(owner, &key, *value);
            }
            Self::Dismissed(value) => {
                qobject::installed_set_pref_string(owner, &key, &QString::from(value.as_str()));
            }
        }
    }
}

/// What one preference write should do, after comparing cache and store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// The store does not hold the value: apply it and save.
    Apply,
    /// The store already holds the value but the cache does not match the
    /// store: adopt the store block and notify, without saving.
    Adopt,
    /// The cache and the store agree: nothing to do.
    Skip,
}

/// Decides how a write relates to the cache and the store.
///
/// The store is authoritative. A value it does not hold must be written even
/// when the cache already shows it — `reset()` and a finished transaction write
/// the store directly, so the cache can be stale. When the store already holds
/// the value the write is not repeated; the cache is only corrected when it
/// disagrees with the store (the `Adopt` case), which never saves.
pub fn decide(write: &PrefWrite, cache: &Prefs, store: &Prefs) -> Decision {
    if !write.is_noop(store) {
        Decision::Apply
    } else if cache != store {
        Decision::Adopt
    } else {
        Decision::Skip
    }
}

/// Decodes a bool write, `None` for a key that is not a bool preference.
pub fn bool_write(key: &str, value: bool) -> Option<PrefWrite> {
    match key {
        keys::NOTIFY_ON_START => Some(PrefWrite::NotifyOnStart(value)),
        keys::DESKTOP => Some(PrefWrite::Desktop(value)),
        keys::START_WITH_SESSION => Some(PrefWrite::StartWithSession(value)),
        _ => None,
    }
}

/// Decodes a string write, `None` for a key that is not a string preference.
pub fn string_write(key: &str, value: &str) -> Option<PrefWrite> {
    match key {
        keys::DISMISSED => Some(PrefWrite::Dismissed(value.to_string())),
        _ => None,
    }
}

/// Decodes the `Prefs` JSON the store owner hands over.
///
/// `None` means the payload is not a `Prefs` object (the owner is not an
/// installed backend, or the file came from an older schema). The caller keeps
/// the values it already shows rather than resetting them to a guess.
pub fn decode_prefs(json: &str) -> Option<Prefs> {
    serde_json::from_str(json).ok()
}

/// Applies a write to the store when it changes something.
///
/// Returns whether the store changed, i.e. whether the caller has to save. A
/// value the store already holds is left alone, so a no-op never writes a file;
/// unknown keys never reach here ([`bool_write`]/[`string_write`] return `None`
/// and the caller returns before touching the store).
pub fn apply_to_store_if_changed(write: &PrefWrite, store: &mut Store) -> bool {
    if write.is_noop(&store.prefs) {
        return false;
    }
    write.apply_to_store(store);
    true
}

/// Rust side of [`qobject::PrefsBackend`].
pub struct PrefsBackendRust {
    /// The installed backend, i.e. the owner of the store.
    owner: QObjectMutPtr,
    notify_on_start: bool,
    desktop_notifications: bool,
    dismissed_version: QString,
    start_with_session: bool,
    /// The schema version of the block this backend mirrors, so its cache
    /// block equals the store's block again after an adopt.
    schema: u32,
}

impl Default for PrefsBackendRust {
    /// The store's defaults, so a backend created before an owner is attached
    /// still shows the values the first save will produce. Pinned to the core
    /// block by [`tests::the_default_block_matches_the_store`].
    fn default() -> Self {
        Self {
            owner: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            notify_on_start: true,
            desktop_notifications: true,
            dismissed_version: QString::from(""),
            start_with_session: true,
            schema: 4,
        }
    }
}

impl PrefsBackendRust {
    /// The persisted fields as the block the store persists.
    fn to_prefs(&self) -> Prefs {
        Prefs {
            notify_on_start: self.notify_on_start,
            desktop: self.desktop_notifications,
            dismissed: self.dismissed_version.to_string(),
            start_with_session: self.start_with_session,
            schema: self.schema,
        }
    }

    /// Adopts a block read from the store, returning which fields moved.
    fn adopt(&mut self, prefs: Prefs) -> Moved {
        let changed = Moved {
            notify_on_start: self.notify_on_start != prefs.notify_on_start,
            desktop: self.desktop_notifications != prefs.desktop,
            dismissed: self.dismissed_version.to_string() != prefs.dismissed,
            start_with_session: self.start_with_session != prefs.start_with_session,
        };
        self.set_from_prefs(prefs);
        changed
    }

    /// Adopts a block read from the store.
    fn set_from_prefs(&mut self, prefs: Prefs) {
        self.notify_on_start = prefs.notify_on_start;
        self.desktop_notifications = prefs.desktop;
        self.dismissed_version = QString::from(prefs.dismissed.as_str());
        self.start_with_session = prefs.start_with_session;
        self.schema = prefs.schema;
    }
}

/// Which preference properties moved in one call, so each signal fires at most
/// once and only for a value that changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Moved {
    notify_on_start: bool,
    desktop: bool,
    dismissed: bool,
    start_with_session: bool,
}

impl qobject::PrefsBackend {
    /// Points the backend at the store owner and adopts its saved block.
    ///
    /// Detaching (a null owner) keeps the last values: the page that owns this
    /// backend may be rebuilt, and a blank checkbox in between would be a lie.
    /// Attaching always overwrites the cache, so a preference written before
    /// this call is lost (see the module docs on attach order).
    fn set_owner(mut self: Pin<&mut Self>, value: QObjectMutPtr) {
        let loaded = if value.as_ptr().is_null() {
            None
        } else {
            decode_prefs(&qobject::installed_prefs(value).to_string())
        };
        let changed = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.owner = value;
            match loaded {
                Some(prefs) => data.adopt(prefs),
                None => Moved::default(),
            }
        };
        self.as_mut().owner_changed();
        self.as_mut().notify_changed(changed);
    }

    /// Whether updates are announced when the window opens.
    fn set_notify_on_start(self: Pin<&mut Self>, value: bool) {
        self.write(PrefWrite::NotifyOnStart(value));
    }

    /// Whether the announcement also goes through the desktop service.
    fn set_desktop_notifications(self: Pin<&mut Self>, value: bool) {
        self.write(PrefWrite::Desktop(value));
    }

    /// The update signature the user dismissed.
    fn set_dismissed_version(self: Pin<&mut Self>, value: QString) {
        self.write(PrefWrite::Dismissed(value.to_string()));
    }

    /// Whether the Qt layer keeps an autostart entry for this app.
    fn set_start_with_session(self: Pin<&mut Self>, value: bool) {
        self.write(PrefWrite::StartWithSession(value));
    }

    /// Makes `~/.config/autostart` match the `startWithSession` preference.
    ///
    /// A failure is not fatal: the app still runs, and the next start retries.
    /// It goes to stderr rather than a dialog, because a session without a
    /// writable config home has bigger problems than a missing entry.
    fn sync_autostart(&self) {
        let Some(path) = crate::autostart::entry_path() else {
            return;
        };
        if let Err(err) = crate::autostart::sync(self.start_with_session, &path) {
            eprintln!("kxbps: autostart entry: {err}");
        }
    }

    /// Persists one write, then notifies the property that owns it.
    ///
    /// The cache is checked first: when it already shows a change, the write
    /// and its save happen without a JSON round-trip. When the cache calls the
    /// write a no-op the store is consulted, because the cache can be stale
    /// (`reset()` and a finished transaction write the store directly); a store
    /// that moved turns the write real again, and a store that agrees is left
    /// untouched. Without an owner there is nothing to confirm against: the
    /// write only updates the cache (attach order, module docs).
    fn write(self: Pin<&mut Self>, write: PrefWrite) {
        let cache = self.to_prefs();
        if !write.is_noop(&cache) {
            self.apply_and_save(write);
            return;
        }
        let Some(store) = self.store_prefs() else {
            return;
        };
        match decide(&write, &cache, &store) {
            Decision::Apply => self.apply_and_save(write),
            Decision::Adopt => self.adopt_only(store),
            Decision::Skip => {}
        }
    }

    /// The block the store currently holds; `None` when no owner is attached or
    /// the owner cannot answer with a complete block.
    fn store_prefs(&self) -> Option<Prefs> {
        if self.owner.as_ptr().is_null() {
            return None;
        }
        decode_prefs(&qobject::installed_prefs(self.owner).to_string())
    }

    /// Writes the value into the cache, forwards it to the store and notifies.
    fn apply_and_save(mut self: Pin<&mut Self>, write: PrefWrite) {
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            let mut prefs = data.to_prefs();
            write.apply(&mut prefs);
            data.set_from_prefs(prefs);
        }
        write.forward(self.owner);
        match write.field() {
            Field::NotifyOnStart => self.as_mut().notify_on_start_changed(),
            Field::Desktop => self.as_mut().desktop_notifications_changed(),
            Field::Dismissed => self.as_mut().dismissed_version_changed(),
            Field::StartWithSession => self.as_mut().start_with_session_changed(),
        }
    }

    /// Takes the store block over the cache without writing anything.
    fn adopt_only(mut self: Pin<&mut Self>, store: Prefs) {
        let changed = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.adopt(store)
        };
        self.as_mut().notify_changed(changed);
    }

    /// Emits the property signals for the fields that moved.
    fn notify_changed(mut self: Pin<&mut Self>, changed: Moved) {
        if changed.notify_on_start {
            self.as_mut().notify_on_start_changed();
        }
        if changed.desktop {
            self.as_mut().desktop_notifications_changed();
        }
        if changed.dismissed {
            self.as_mut().dismissed_version_changed();
        }
        if changed.start_with_session {
            self.as_mut().start_with_session_changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    /// A private directory under the system temp dir, removed on drop.
    ///
    /// The crate has no `tempfile` dependency and must not gain one, so the
    /// name carries the process id plus a per-process counter.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "kxbps-prefs-{tag}-{}-{}",
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

    #[test]
    fn the_default_block_matches_the_store() {
        // The backend is created before an owner exists, so its initial values
        // must be the ones the store would write; drift here shows up as a
        // checkbox that flips the moment the page is opened.
        let store = Store::fresh();
        let backend = PrefsBackendRust::default();
        assert_eq!(backend.to_prefs(), store.prefs);
    }

    #[test]
    fn the_store_json_maps_onto_the_four_properties() {
        let prefs = Prefs {
            notify_on_start: false,
            desktop: true,
            dismissed: "sig-42".to_string(),
            start_with_session: false,
            schema: 4,
        };
        let json = serde_json::to_string(&prefs).expect("serialise");
        assert!(json.contains("\"notifyOnStart\""));
        assert!(json.contains("\"desktop\""));
        assert!(json.contains("\"dismissed\""));
        assert!(json.contains("\"startWithSession\""));

        let mut backend = PrefsBackendRust::default();
        backend.set_from_prefs(decode_prefs(&json).expect("decode"));
        assert!(!backend.notify_on_start);
        assert!(backend.desktop_notifications);
        assert_eq!(backend.dismissed_version.to_string(), "sig-42");
        assert!(!backend.start_with_session);
        assert_eq!(backend.to_prefs(), prefs);
    }

    #[test]
    fn a_bool_key_dispatches_to_its_own_field() {
        let mut prefs = Prefs {
            notify_on_start: true,
            desktop: false,
            dismissed: String::new(),
            start_with_session: true,
            schema: 4,
        };
        let write = bool_write(keys::NOTIFY_ON_START, false).expect("known key");
        assert_eq!(write.key(), keys::NOTIFY_ON_START);
        write.apply(&mut prefs);
        assert!(!prefs.notify_on_start);
        assert!(!prefs.desktop, "the sibling bool is untouched");
        assert_eq!(prefs.dismissed, "", "the string is untouched");

        let write = bool_write(keys::DESKTOP, true).expect("known key");
        assert_eq!(write.key(), keys::DESKTOP);
        write.apply(&mut prefs);
        assert!(prefs.desktop);
        assert!(!prefs.notify_on_start);
    }

    #[test]
    fn a_string_key_dispatches_to_its_own_field() {
        let mut prefs = Prefs {
            notify_on_start: true,
            desktop: false,
            dismissed: String::new(),
            start_with_session: true,
            schema: 4,
        };
        let write = string_write(keys::DISMISSED, "sig-7").expect("known key");
        assert_eq!(write.key(), keys::DISMISSED);
        write.apply(&mut prefs);
        assert_eq!(prefs.dismissed, "sig-7");
        assert!(prefs.notify_on_start, "the bools are untouched");
        assert!(!prefs.desktop);
    }

    #[test]
    fn an_unknown_key_is_ignored() {
        // The caller returns before touching the store, so an unknown key
        // cannot cause a save.
        assert_eq!(bool_write("nope", true), None);
        assert_eq!(string_write("nope", "x"), None);
        // The two kinds are not interchangeable.
        assert_eq!(bool_write(keys::DISMISSED, true), None);
        assert_eq!(string_write(keys::NOTIFY_ON_START, "x"), None);
        assert_eq!(string_write(keys::DESKTOP, "x"), None);
    }

    #[test]
    fn writing_the_current_value_is_a_noop() {
        let prefs = Prefs {
            notify_on_start: true,
            desktop: false,
            dismissed: String::new(),
            start_with_session: true,
            schema: 4,
        };
        assert!(
            bool_write(keys::NOTIFY_ON_START, true)
                .unwrap()
                .is_noop(&prefs)
        );
        assert!(bool_write(keys::DESKTOP, false).unwrap().is_noop(&prefs));
        assert!(string_write(keys::DISMISSED, "").unwrap().is_noop(&prefs));
        assert!(
            !bool_write(keys::NOTIFY_ON_START, false)
                .unwrap()
                .is_noop(&prefs)
        );
        assert!(!bool_write(keys::DESKTOP, true).unwrap().is_noop(&prefs));
        assert!(
            !string_write(keys::DISMISSED, "sig")
                .unwrap()
                .is_noop(&prefs)
        );
    }

    /// A preference block, for the decision tests.
    fn block(notify_on_start: bool, desktop: bool, dismissed: &str) -> Prefs {
        Prefs {
            notify_on_start,
            desktop,
            dismissed: dismissed.to_string(),
            start_with_session: true,
            schema: 4,
        }
    }

    #[test]
    fn every_property_maps_to_its_own_store_key() {
        // The key a property writes must decode back to the same field, so a
        // copy-paste between setters cannot silently persist to a neighbour.
        let writes = [
            PrefWrite::NotifyOnStart(false),
            PrefWrite::Desktop(true),
            PrefWrite::Dismissed("sig".to_string()),
            PrefWrite::StartWithSession(false),
        ];
        assert_eq!(writes[0].key(), "notifyOnStart");
        assert_eq!(writes[1].key(), "desktop");
        assert_eq!(writes[2].key(), "dismissed");
        assert_eq!(writes[3].key(), "startWithSession");

        assert_eq!(writes[0].field(), Field::NotifyOnStart);
        assert_eq!(writes[1].field(), Field::Desktop);
        assert_eq!(writes[2].field(), Field::Dismissed);
        assert_eq!(writes[3].field(), Field::StartWithSession);

        assert_eq!(bool_write(writes[0].key(), false), Some(writes[0].clone()));
        assert_eq!(bool_write(writes[1].key(), true), Some(writes[1].clone()));
        assert_eq!(
            string_write(writes[2].key(), "sig"),
            Some(writes[2].clone())
        );
        assert_eq!(bool_write(writes[3].key(), false), Some(writes[3].clone()));
    }

    #[test]
    fn a_store_that_moved_forces_the_write_through() {
        // Reset emptied the store while the cache still shows the dismissed
        // signature: the cache calls the write a no-op, the store must still
        // receive it or the banner returns after a restart.
        let cache = block(true, false, "sig-A");
        let store = block(true, false, "");
        let write = string_write(keys::DISMISSED, "sig-A").expect("known key");
        assert!(write.is_noop(&cache), "the cache calls it a no-op");
        assert_eq!(decide(&write, &cache, &store), Decision::Apply);
    }

    #[test]
    fn a_store_that_already_holds_the_value_is_adopted_without_a_save() {
        // Another writer moved the store to the value QML is about to write,
        // and the cache is stale on another field: adopt, do not save.
        let cache = block(true, false, "");
        let store = block(true, false, "sig-A");
        let write = string_write(keys::DISMISSED, "sig-A").expect("known key");
        assert_eq!(decide(&write, &cache, &store), Decision::Adopt);
    }

    #[test]
    fn a_write_both_sides_agree_on_is_skipped() {
        let prefs = block(true, false, "sig-A");
        let write = string_write(keys::DISMISSED, "sig-A").expect("known key");
        assert_eq!(decide(&write, &prefs, &prefs), Decision::Skip);
    }

    #[test]
    fn a_store_that_already_holds_the_value_is_not_changed() {
        // This is the rule `InstalledBackend::setPrefBool|String` applies: the
        // `false` answers mean "do not save".
        let mut store = Store::fresh();
        let same = bool_write(keys::NOTIFY_ON_START, true).expect("known key");
        assert!(!apply_to_store_if_changed(&same, &mut store), "no save");

        let change = bool_write(keys::NOTIFY_ON_START, false).expect("known key");
        assert!(apply_to_store_if_changed(&change, &mut store), "save");
        assert!(!store.prefs.notify_on_start);
        assert!(
            !apply_to_store_if_changed(&change, &mut store),
            "a repeat is a no-op"
        );
    }

    #[test]
    fn four_writes_survive_a_store_round_trip() {
        let dir = TempDir::new("roundtrip");
        let mut store = Store::fresh();

        for write in [
            bool_write(keys::NOTIFY_ON_START, false).expect("known key"),
            bool_write(keys::DESKTOP, false).expect("known key"),
            string_write(keys::DISMISSED, "sig-42").expect("known key"),
            bool_write(keys::START_WITH_SESSION, false).expect("known key"),
        ] {
            assert!(
                !write.is_noop(&store.prefs),
                "the defaults hold another value"
            );
            write.apply_to_store(&mut store);
            assert!(write.is_noop(&store.prefs), "a repeat is a no-op");
        }
        store.save_at(dir.path()).expect("save");

        let loaded = Store::load_at(dir.path()).expect("load");
        let json = serde_json::to_string(&loaded.prefs).expect("serialise");
        let prefs = decode_prefs(&json).expect("decode");
        assert!(!prefs.notify_on_start);
        assert!(!prefs.desktop);
        assert_eq!(prefs.dismissed, "sig-42");
        assert!(!prefs.start_with_session);
    }

    #[test]
    fn a_broken_payload_is_not_a_preference_block() {
        assert_eq!(decode_prefs("not json"), None);
        assert_eq!(decode_prefs("{}"), None, "the stored fields are required");
        assert_eq!(decode_prefs("[]"), None);
    }
}
