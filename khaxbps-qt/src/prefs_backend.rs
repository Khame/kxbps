//! The Settings preferences: three values that survive a restart.
//!
//! The store is the only owner of `prefs.json`, and
//! [`crate::installed_backend`] is the only owner of the store, so this backend
//! holds no persistence logic of its own. `set_owner` reads the block once
//! through the installed backend, and every write goes back through the same
//! two forwarders, which save the store once per change. There is no KConfig
//! here on purpose (Global Constraints): the state is three fields and the
//! store already replaces its files atomically.
//!
//! QML only ever sees the three properties. A write that matches the current
//! value is dropped before it reaches the store, so binding a checkbox to a
//! property cannot cause a save — or a notify — for every binding evaluation.

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QObjectMutPtr, QString};
use khaxbps_core::{Prefs, Store};

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

        #[cxx_name = "khaxbps_installed_prefs"]
        fn installed_prefs(owner: QObjectMutPtr) -> QString;

        #[cxx_name = "khaxbps_installed_set_pref_bool"]
        fn installed_set_pref_bool(owner: QObjectMutPtr, key: &QString, value: bool);

        #[cxx_name = "khaxbps_installed_set_pref_string"]
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
        type PrefsBackend = super::PrefsBackendRust;

        fn set_owner(self: Pin<&mut PrefsBackend>, value: QObjectMutPtr);

        fn set_notify_on_start(self: Pin<&mut PrefsBackend>, value: bool);

        fn set_desktop_notifications(self: Pin<&mut PrefsBackend>, value: bool);

        fn set_dismissed_version(self: Pin<&mut PrefsBackend>, value: QString);
    }
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
}

/// One preference write, decoded from the key QML sent.
///
/// QML reaches the store with a key and a value rather than a whole [`Prefs`]
/// block, so a stale copy of the other two fields can never overwrite a change
/// made elsewhere in the meantime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefWrite {
    NotifyOnStart(bool),
    Desktop(bool),
    Dismissed(String),
}

impl PrefWrite {
    /// The store key of this write.
    pub fn key(&self) -> &'static str {
        match self {
            Self::NotifyOnStart(_) => keys::NOTIFY_ON_START,
            Self::Desktop(_) => keys::DESKTOP,
            Self::Dismissed(_) => keys::DISMISSED,
        }
    }

    /// Whether `prefs` already holds this value.
    pub fn is_noop(&self, prefs: &Prefs) -> bool {
        match self {
            Self::NotifyOnStart(value) => prefs.notify_on_start == *value,
            Self::Desktop(value) => prefs.desktop == *value,
            Self::Dismissed(value) => prefs.dismissed == *value,
        }
    }

    /// Applies the write to a plain block; used by the backend and the tests.
    pub fn apply(&self, prefs: &mut Prefs) {
        match self {
            Self::NotifyOnStart(value) => prefs.notify_on_start = *value,
            Self::Desktop(value) => prefs.desktop = *value,
            Self::Dismissed(value) => prefs.dismissed.clone_from(value),
        }
    }

    /// Applies the write through the store, which marks itself dirty.
    pub fn apply_to_store(&self, store: &mut Store) {
        match self {
            Self::NotifyOnStart(value) => store.set_pref_notify_on_start(*value),
            Self::Desktop(value) => store.set_pref_desktop_notifications(*value),
            Self::Dismissed(value) => store.set_pref_dismissed_version(value),
        }
    }

    /// Sends the write to the store owner; a missing owner is a no-op.
    fn forward(&self, owner: QObjectMutPtr) {
        if owner.as_ptr().is_null() {
            return;
        }
        let key = QString::from(self.key());
        match self {
            Self::NotifyOnStart(value) | Self::Desktop(value) => {
                qobject::installed_set_pref_bool(owner, &key, *value);
            }
            Self::Dismissed(value) => {
                qobject::installed_set_pref_string(owner, &key, &QString::from(value.as_str()));
            }
        }
    }
}

/// Decodes a bool write, `None` for a key that is not a bool preference.
pub fn bool_write(key: &str, value: bool) -> Option<PrefWrite> {
    match key {
        keys::NOTIFY_ON_START => Some(PrefWrite::NotifyOnStart(value)),
        keys::DESKTOP => Some(PrefWrite::Desktop(value)),
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

/// Rust side of [`qobject::PrefsBackend`].
pub struct PrefsBackendRust {
    /// The installed backend, i.e. the owner of the store.
    owner: QObjectMutPtr,
    notify_on_start: bool,
    desktop_notifications: bool,
    dismissed_version: QString,
}

impl Default for PrefsBackendRust {
    /// The store's defaults, so a backend created before an owner is attached
    /// still shows the values the first save will produce. Pinned to the core
    /// block by [`tests::the_default_block_matches_the_store`].
    fn default() -> Self {
        Self {
            owner: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            notify_on_start: true,
            desktop_notifications: false,
            dismissed_version: QString::from(""),
        }
    }
}

impl PrefsBackendRust {
    /// The three fields as the block the store persists.
    fn to_prefs(&self) -> Prefs {
        Prefs {
            notify_on_start: self.notify_on_start,
            desktop: self.desktop_notifications,
            dismissed: self.dismissed_version.to_string(),
        }
    }

    /// Adopts a block read from the store.
    fn set_from_prefs(&mut self, prefs: Prefs) {
        self.notify_on_start = prefs.notify_on_start;
        self.desktop_notifications = prefs.desktop;
        self.dismissed_version = QString::from(prefs.dismissed.as_str());
    }
}

impl qobject::PrefsBackend {
    /// Points the backend at the store owner and adopts its saved block.
    ///
    /// Detaching (a null owner) keeps the last values: the page that owns this
    /// backend may be rebuilt, and a blank checkbox in between would be a lie.
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
                Some(prefs) => {
                    let changed = (
                        data.notify_on_start != prefs.notify_on_start,
                        data.desktop_notifications != prefs.desktop,
                        data.dismissed_version.to_string() != prefs.dismissed,
                    );
                    data.set_from_prefs(prefs);
                    changed
                }
                None => (false, false, false),
            }
        };
        self.as_mut().owner_changed();
        if changed.0 {
            self.as_mut().notify_on_start_changed();
        }
        if changed.1 {
            self.as_mut().desktop_notifications_changed();
        }
        if changed.2 {
            self.as_mut().dismissed_version_changed();
        }
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

    /// Persists one write, then notifies the property that owns it.
    ///
    /// A value the backend already holds is dropped here, before the store
    /// sees it, so re-evaluating a binding cannot write the same file again.
    fn write(mut self: Pin<&mut Self>, write: PrefWrite) {
        if write.is_noop(&self.to_prefs()) {
            return;
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            let mut prefs = data.to_prefs();
            write.apply(&mut prefs);
            data.set_from_prefs(prefs);
        }
        write.forward(self.owner);
        match write {
            PrefWrite::NotifyOnStart(_) => self.as_mut().notify_on_start_changed(),
            PrefWrite::Desktop(_) => self.as_mut().desktop_notifications_changed(),
            PrefWrite::Dismissed(_) => self.as_mut().dismissed_version_changed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use khaxbps_core::Catalog;

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
                "khaxbps-prefs-{tag}-{}-{}",
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
        let store = Store::fresh(&Catalog::default());
        let backend = PrefsBackendRust::default();
        assert_eq!(backend.to_prefs(), store.prefs);
    }

    #[test]
    fn the_store_json_maps_onto_the_three_properties() {
        let prefs = Prefs {
            notify_on_start: false,
            desktop: true,
            dismissed: "sig-42".to_string(),
        };
        let json = serde_json::to_string(&prefs).expect("serialise");
        assert!(json.contains("\"notifyOnStart\""));
        assert!(json.contains("\"desktop\""));
        assert!(json.contains("\"dismissed\""));

        let mut backend = PrefsBackendRust::default();
        backend.set_from_prefs(decode_prefs(&json).expect("decode"));
        assert!(!backend.notify_on_start);
        assert!(backend.desktop_notifications);
        assert_eq!(backend.dismissed_version.to_string(), "sig-42");
        assert_eq!(backend.to_prefs(), prefs);
    }

    #[test]
    fn a_bool_key_dispatches_to_its_own_field() {
        let mut prefs = Prefs {
            notify_on_start: true,
            desktop: false,
            dismissed: String::new(),
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

    #[test]
    fn a_write_survives_a_store_round_trip() {
        let dir = TempDir::new("roundtrip");
        let catalog = Catalog::default();
        let mut store = Store::fresh(&catalog);

        for write in [
            bool_write(keys::NOTIFY_ON_START, false).expect("known key"),
            bool_write(keys::DESKTOP, true).expect("known key"),
            string_write(keys::DISMISSED, "sig-42").expect("known key"),
        ] {
            assert!(!write.is_noop(&store.prefs), "the seed holds another value");
            write.apply_to_store(&mut store);
            assert!(write.is_noop(&store.prefs), "a repeat is a no-op");
        }
        store.save_at(dir.path()).expect("save");

        let loaded = Store::load_at(dir.path(), &catalog).expect("load");
        let json = serde_json::to_string(&loaded.prefs).expect("serialise");
        let prefs = decode_prefs(&json).expect("decode");
        assert!(!prefs.notify_on_start);
        assert!(prefs.desktop);
        assert_eq!(prefs.dismissed, "sig-42");
    }

    #[test]
    fn a_broken_payload_is_not_a_preference_block() {
        assert_eq!(decode_prefs("not json"), None);
        assert_eq!(decode_prefs("{}"), None, "all three fields are required");
        assert_eq!(decode_prefs("[]"), None);
    }
}
