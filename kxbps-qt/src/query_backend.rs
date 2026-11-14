//! Query façade for QML: the raw `xbps-query` takes of the Query page and the
//! facts of one package, both read from xbps through the runner.
//!
//! The package *model* is owned by [`crate::installed_backend::InstalledBackend`],
//! because it needs the store's installed and queue snapshots. This object
//! answers the Query page's `search()` (verbatim output, no rows) and the
//! details pane's `get()`, and it receives the *same* snapshot from that
//! backend (`queryBackend` in QML), so the facts answer `isUpdate`/`held` from
//! the same source as a list row.
//!
//! Neither answer comes from the embedded catalog: the Query page runs one of
//! the five diagnostics in [`QueryMode::ALL`] (`-x`, `-X`, `-f`, `-o`, `-p`),
//! and the facts of one package run the two property takes and the two
//! dependency lists, both on a worker thread through the same injected
//! transport as the snapshot reads. The Available listing has its own path
//! (`read_list_search`), which stays a repository search and is not this one.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cxx_qt::CxxQtType;
use cxx_qt::Threading;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QStringList, QVariant};
use kxbps_core::{InstalledMap, QueryMode, QueueAction, QueueItem, UpdateEntry, split_pkgver};

use crate::package_model::InstalledState;
use crate::xbps_snapshot::{
    PackageFacts, RawOutput, SearchSource, SharedInvoke, SnapshotError, read_package, read_raw,
    read_search,
};

/// The repository answers the package list already holds.
///
/// The cache exists so a request the backend does not want to re-read still
/// gets exactly one answer: the window sets `searchLoading` before it emits
/// `searchRequested` and only an answer clears it, so a silent skip would leave
/// the page loading forever.
///
/// Two slots at most, kept apart on purpose: the whole listing (the empty
/// pattern) and the newest typed pattern. Typing must not evict the listing,
/// because clearing the field is the same page and should be instant. The JSON
/// is stored as `xbps-query -Rs` produced it; the model parses it again on every
/// delivery, which costs a few tens of milliseconds for twenty thousand hits —
/// caching parsed rows here would duplicate the model's own row store.
#[derive(Default)]
struct ListAnswers {
    listing: Option<QString>,
    typed: Option<(String, QString)>,
}

impl ListAnswers {
    /// The stored answer for `pattern`, when the list already asked for it.
    fn get(&self, pattern: &str) -> Option<&QString> {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            self.listing.as_ref()
        } else {
            self.typed
                .as_ref()
                .filter(|(held, _)| held == pattern)
                .map(|(_, json)| json)
        }
    }

    /// Records the answer the list now shows for `pattern`.
    ///
    /// A fresh listing also forgets the typed answer: the index may have moved,
    /// and nothing asks for the old pattern again except the pattern itself,
    /// which arrives as a new read.
    fn remember(&mut self, pattern: &str, json: &QString) {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            self.listing = Some(json.clone());
            self.typed = None;
        } else {
            self.typed = Some((pattern.to_owned(), json.clone()));
        }
    }

    /// Drops the listing; the next empty request reads the repositories again.
    fn forget_listing(&mut self) {
        self.listing = None;
    }
}

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/core/qmap/qmap_QString_QVariant.h");
        type QMap_QString_QVariant = cxx_qt_lib::QMap<cxx_qt_lib::QMapPair_QString_QVariant>;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, search_error, READ, NOTIFY)]
        #[qproperty(QStringList, search_error_args, READ, NOTIFY)]
        /// The mode selector's tokens, in the order it offers them. Command
        /// syntax, never translated; QML renders this list instead of keeping
        /// a second copy that could drift from `QueryMode::ALL`.
        #[qproperty(QStringList, query_modes, READ, CONSTANT)]
        /// Whether the newest query is still being read.
        #[qproperty(bool, searching, READ, NOTIFY)]
        // Every take of this page prints text, never package rows: the output
        // of the newest `read_raw` answer and the command it came from.
        #[qproperty(QString, raw_text, READ, NOTIFY)]
        #[qproperty(QString, raw_command, READ, NOTIFY)]
        #[qproperty(QString, facts_error, READ, NOTIFY)]
        #[qproperty(QStringList, facts_error_args, READ, NOTIFY)]
        #[qproperty(QString, facts_pkgver, READ, NOTIFY)]
        #[qproperty(i32, facts_revision, READ, NOTIFY)]
        type QueryBackend = super::QueryBackendRust;

        #[qinvokable]
        fn version(self: &QueryBackend) -> QString;

        /// Runs one diagnostic take on a worker thread. The output of the
        /// previous answer stays until the new one lands; `rawText` and
        /// `rawCommand` notify when it does.
        #[qinvokable]
        fn search(self: Pin<&mut QueryBackend>, query: &QString, mode: &QString);

        /// The facts of one pkgver, as a role-keyed map. An empty map while the
        /// first read is in flight; QML re-reads through `factsRevision`.
        #[qinvokable]
        fn get(self: &QueryBackend, pkgver: &QString) -> QMap_QString_QVariant;

        /// Reads a package's facts when nothing has asked for it yet.
        #[qinvokable]
        fn request_facts(self: Pin<&mut QueryBackend>, pkgver: &QString);

        /// The version a read reported, or the one the pkgver carries.
        #[qinvokable]
        fn version_of(self: &QueryBackend, pkgver: &QString) -> QString;

        /// Reads the repository hits for the package list's search field and
        /// answers through `listSearchDone` when the read lands.
        ///
        /// The list asks this, not the Query view: the pattern is always
        /// searched against the repositories (`xbps-query -Rs`), whatever
        /// filter the list shows, and the hits arrive ranked with the exact
        /// name first. A pattern already on screen costs no second read.
        #[qinvokable]
        fn read_list_search(self: Pin<&mut QueryBackend>, pattern: &QString);

        /// Whether the list already shows the answer for `pattern`; a
        /// repeated Enter costs nothing.
        #[qinvokable]
        fn list_search_is_current(self: &QueryBackend, pattern: &QString) -> bool;

        /// Drops the cached repository listing.
        ///
        /// Called when the repository index may have changed — the toolbar
        /// Refresh and the end of a job — so the next Available visit reads the
        /// repositories again instead of showing rows an index has moved past.
        /// Only the listing is dropped; a typed answer is transient anyway.
        #[qinvokable]
        fn invalidate_list_search(self: Pin<&mut QueryBackend>);

        /// A repository answer for the package list: the request id, so a
        /// stale answer is dropped, and the JSON of
        /// `{"hits": [...], "errorKey": "…", "errorArgs": [...]}`.
        #[qsignal]
        fn list_search_done(self: Pin<&mut QueryBackend>, request: u64, json: QString);

        // Pushes the installed/queue snapshot; called by the installed backend
        // through `kxbps_query_backend_set_state`. `updates_json` and
        // `orphans_json` are the lists xbps reported, so a query hit answers
        // `isUpdate` from the same source as a list row.
        fn set_state(
            self: Pin<&mut QueryBackend>,
            installed_json: &QString,
            queue_json: &QString,
            updates_json: &QString,
            orphans_json: &QString,
        );
    }

    impl cxx_qt::Constructor<()> for QueryBackend {}
    // A search and a facts read run on worker threads and report back through
    // `qt_thread().queue(...)`; that needs the thread handle CXX-Qt generates.
    impl cxx_qt::Threading for QueryBackend {}
}

/// Rust side of [`qobject::QueryBackend`].
///
/// The read state lives here and every mutation takes `&mut self`: the bridge
/// methods wrap it in `Pin<&mut Self>`, which only unwraps inside `unsafe`, so
/// the tweaks and the guards sit together on the plain struct.
pub struct QueryBackendRust {
    /// The same snapshot the package model holds.
    installed: InstalledMap,
    queue: HashMap<String, QueueAction>,
    /// Derived from `installed`, shared with the model.
    state: InstalledState,
    /// The newest read owns the window: one counter for search and one for
    /// the facts, so a slow answer in one never cancels the other.
    search_generation: Arc<AtomicU64>,
    facts_generation: Arc<AtomicU64>,
    /// The process transport. Tests install a fake here, so the suite never
    /// starts xbps; production installs the closure
    /// [`crate::xbps_bridge::production_invoke`] returns.
    invoke: SharedInvoke,
    /// The newest search answer, tagged with the signature it answers.
    answer: SearchAnswer,
    /// The signature of the newest search, pending or answered.
    signature: SearchSignature,
    /// The mode selector's tokens, in the order it offers them. A property of
    /// its own so QML renders exactly `QueryMode::ALL` instead of keeping a
    /// second list that could drift.
    query_modes: QStringList,
    search_error: QString,
    search_error_args: QStringList,
    searching: bool,
    /// The text the newest take printed, and its command.
    raw_text: QString,
    raw_command: QString,
    /// The facts of one package per pkgver read.
    facts: HashMap<String, PackageFacts>,
    /// The pkgver whose read is in flight, if any.
    facts_pending: Option<String>,
    facts_error: QString,
    facts_error_args: QStringList,
    facts_pkgver: QString,
    facts_revision: i32,
    /// The repository answers the list already shows, so a request that is not
    /// re-read is still answered from here instead of hanging the page.
    ///
    /// The package list asks for a repository answer on every non-empty search;
    /// a pattern already on screen costs no second `xbps-query`, which is what
    /// keeps Enter from re-reading what typing already found.
    answers: ListAnswers,
}

/// What a search is asked for: a moved signature makes an answer stale.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SearchSignature {
    /// The diagnostic that was run.
    mode: QueryMode,
    query: String,
}

impl Default for SearchSignature {
    /// The signature of no query at all.
    fn default() -> Self {
        Self {
            mode: QueryMode::X,
            query: String::new(),
        }
    }
}

impl SearchSignature {
    fn new(mode: QueryMode, query: &str) -> Self {
        Self {
            mode,
            query: query.to_owned(),
        }
    }
}

/// One answer of a search, tagged with the signature it answers.
///
/// A failed take keeps the error next to no text: the window shows the key and
/// never output it invented.
#[derive(Default)]
struct SearchAnswer {
    signature: SearchSignature,
    error: Option<SnapshotError>,
    /// What the take printed; `None` when it failed before printing.
    raw: Option<RawOutput>,
}

/// What a landed answer moved, so the bridge emits exactly the notifications
/// its properties declare.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AnswerEffect {
    /// The answer belongs to the newest query.
    fresh: bool,
    /// `rawText` changed and its NOTIFY has to be emitted.
    raw_text_moved: bool,
    /// `rawCommand` changed and its NOTIFY has to be emitted.
    raw_command_moved: bool,
}

impl Default for QueryBackendRust {
    fn default() -> Self {
        Self {
            installed: InstalledMap::new(),
            queue: HashMap::new(),
            state: InstalledState::new(&[], &[]),
            search_generation: Arc::new(AtomicU64::new(0)),
            facts_generation: Arc::new(AtomicU64::new(0)),
            invoke: crate::xbps_bridge::production_invoke(),
            answer: SearchAnswer::default(),
            signature: SearchSignature::default(),
            query_modes: QueryMode::ALL
                .iter()
                .map(|mode| QString::from(mode.token()))
                .collect(),
            search_error: QString::from(""),
            search_error_args: QStringList::default(),
            searching: false,
            raw_text: QString::from(""),
            raw_command: QString::from(""),
            facts: HashMap::new(),
            facts_pending: None,
            facts_error: QString::from(""),
            facts_error_args: QStringList::default(),
            facts_pkgver: QString::from(""),
            facts_revision: 0,
            answers: ListAnswers::default(),
        }
    }
}

impl cxx_qt::Initialize for qobject::QueryBackend {
    /// Nothing to read at construction: every answer comes from xbps on
    /// demand.
    fn initialize(self: Pin<&mut Self>) {}
}

impl qobject::QueryBackend {
    fn version(&self) -> QString {
        QString::from(env!("CARGO_PKG_VERSION"))
    }

    /// One diagnostic `xbps-query` take, on a worker thread; `rawText` and
    /// `rawCommand` notify when its output lands.
    fn search(mut self: Pin<&mut Self>, query: &cxx_qt_lib::QString, mode: &cxx_qt_lib::QString) {
        let token = mode.to_string();
        let query = trimmed(query);
        // The selector renders `QueryMode::ALL`, so an unknown token can only
        // come from a broken caller; running nothing is the honest answer.
        let Some(mode) = QueryMode::from_token(&token) else {
            return;
        };
        let signature = SearchSignature::new(mode, &query);
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            // The newest answer already covers this take; nothing to read.
            if data.answer.signature == signature {
                return;
            }
            if query.is_empty() {
                // No diagnostic answers an empty pattern: the field is what the
                // take asks about, and any read still in flight is now stale.
                data.clear_answer();
            } else {
                data.begin_search(signature.clone());
            }
        }
        self.as_mut().searching_changed();
        if query.is_empty() {
            self.as_mut().search_error_changed();
            self.as_mut().search_error_args_changed();
            self.as_mut().raw_text_changed();
            self.as_mut().raw_command_changed();
            return;
        }
        self.as_mut().run_search(signature);
    }

    /// The facts of one pkgver: the cached answer, or an empty map while a read
    /// of the same pkgver is in flight.
    fn get(&self, pkgver: &cxx_qt_lib::QString) -> QMap<QMapPair_QString_QVariant> {
        let pkgver = pkgver.to_string();
        let name = split_pkgver(&pkgver)
            .map(|(name, _)| name.to_owned())
            .unwrap_or_else(|| pkgver.clone());
        self.rust()
            .facts_of(&pkgver)
            .map_or_else(QMap::default, |facts| self.facts_map(&name, facts))
    }

    /// Starts a read for a pkgver whose facts are neither cached nor in flight.
    fn request_facts(mut self: Pin<&mut Self>, pkgver: &cxx_qt_lib::QString) {
        let pkgver = pkgver.to_string();
        if pkgver.is_empty() {
            return;
        }
        let started = Pin::get_mut(self.as_mut().rust_mut())
            .begin_facts(&pkgver)
            .is_some();
        if started {
            self.as_mut().run_facts(pkgver);
        }
    }

    /// The version a take reported, or the version part of the pkgver.
    fn version_of(&self, pkgver: &cxx_qt_lib::QString) -> QString {
        let pkgver = pkgver.to_string();
        QString::from(match self.rust().facts_of(&pkgver) {
            Some(facts) => facts.version().to_owned(),
            None => split_pkgver(&pkgver)
                .map(|(_, version)| version.to_owned())
                .unwrap_or_default(),
        })
    }

    /// Whether the list already shows the answer for `pattern`.
    fn list_search_is_current(&self, pattern: &cxx_qt_lib::QString) -> bool {
        !self.rust().needs_search(&pattern.to_string())
    }

    /// Reads the repository hits for the list's search field on a worker
    /// thread; the answer arrives through `listSearchDone`.
    ///
    /// Every call produces exactly one `listSearchDone`: a fresh read, a stored
    /// answer, and a failure alike. The list marks itself as loading before it
    /// asks and only an answer clears that mark, so returning without one would
    /// leave the page loading forever.
    fn read_list_search(mut self: Pin<&mut Self>, pattern: &cxx_qt_lib::QString) {
        // The empty pattern is not "no search": it is the whole repository
        // listing the Available page asks for. `xbps-query -Rs ''` answers it
        // like any other pattern.
        let pattern = pattern.to_string().trim().to_owned();
        let (generation, stored) = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            // A new generation owns the window: an answer still in flight for a
            // pattern the user has left must not land on the page on screen.
            (
                data.begin_search_read(),
                data.answers.get(&pattern).cloned(),
            )
        };
        let thread = self.qt_thread();
        if let Some(json) = stored {
            // Posted rather than emitted: this runs inside the list's
            // `searchRequested` emission, and applying the answer there would
            // mutate the model while its own setter still holds it. One turn of
            // the event loop later is still instant to the eye.
            thread
                .queue(move |backend| backend.deliver_list_search(generation, &pattern, json))
                .ok();
            return;
        }
        let invoke = self.invoke.clone();
        std::thread::spawn(move || {
            // A detached copy of the read state: `search_answer_json` only
            // touches the transport, so the worker needs nothing else.
            let reader = QueryBackendRust {
                invoke,
                ..QueryBackendRust::default()
            };
            let json = reader.search_answer_json(&pattern);
            thread
                .queue(move |backend| backend.deliver_list_search(generation, &pattern, json))
                .ok();
        });
    }

    /// Applies a landed repository answer unless a newer pattern started.
    fn deliver_list_search(
        mut self: Pin<&mut Self>,
        generation: u64,
        pattern: &str,
        json: QString,
    ) {
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            if !data.search_read_is_current(generation) {
                return;
            }
            data.remember_search_answer(pattern, &json);
        }
        self.as_mut().list_search_done(generation, json);
    }

    /// Drops the cached repository listing, so the next empty request reads the
    /// repositories again.
    ///
    /// The window calls this when the index may have moved: the toolbar Refresh
    /// and the end of a job. Nothing else about the read state changes.
    fn invalidate_list_search(mut self: Pin<&mut Self>) {
        let data = Pin::get_mut(self.as_mut().rust_mut());
        data.forget_listing();
    }

    fn set_state(
        mut self: Pin<&mut Self>,
        installed_json: &cxx_qt_lib::QString,
        queue_json: &cxx_qt_lib::QString,
        updates_json: &cxx_qt_lib::QString,
        orphans_json: &cxx_qt_lib::QString,
    ) {
        let installed: InstalledMap =
            serde_json::from_str(&installed_json.to_string()).unwrap_or_default();
        let queue: Vec<QueueItem> =
            serde_json::from_str(&queue_json.to_string()).unwrap_or_default();
        let updates: Vec<UpdateEntry> =
            serde_json::from_str(&updates_json.to_string()).unwrap_or_default();
        let orphans: Vec<String> =
            serde_json::from_str(&orphans_json.to_string()).unwrap_or_default();
        let data = Pin::get_mut(self.as_mut().rust_mut());
        data.apply_snapshot(installed, queue, updates, orphans);
    }
}

/// Whether two installed maps describe the same records.
///
/// A fact read answers from the installed record — its version, whether it is
/// installed at all, whether it is held and whether it is explicit — so the
/// comparison covers exactly those fields. The install timestamp is left out:
/// nothing a fact read reports depends on it, and a queued install that later
/// stamps the package does not change what the record says.
fn same_installed_records(left: &InstalledMap, right: &InstalledMap) -> bool {
    left.len() == right.len()
        && left.iter().all(|(name, before)| {
            right.get(name).is_some_and(|after| {
                before.version == after.version
                    && before.explicit == after.explicit
                    && before.held == after.held
            })
        })
}

impl QueryBackendRust {
    /// Replaces the installed and queue snapshots with the store's own values.
    ///
    /// `InstalledState::new` is the same call the package model makes, so
    /// `isUpdate`/`updateVersion` and the orphan predicate are derived once, in
    /// one place, from the lists xbps reported.
    ///
    /// The cached facts survive a push that keeps every installed record.
    /// A fact read answers `installed`, `state`, `installed_size` and
    /// `repository` from the installed record it saw, so after a real
    /// transaction (a package installed, upgraded or held) every entry may
    /// describe a system that no longer exists — a package that was just
    /// installed would still read as "not installed". A queue-only push moves
    /// nothing those answers depend on, and dropping the cache there would
    /// make the details pane re-read a package that did not change, so the
    /// cache goes with the records, not with the push.
    fn apply_snapshot(
        &mut self,
        installed: InstalledMap,
        queue: Vec<QueueItem>,
        updates: Vec<UpdateEntry>,
        orphans: Vec<String>,
    ) {
        let records_moved = !same_installed_records(&self.installed, &installed);
        self.state = InstalledState::new(&updates, &orphans);
        self.installed = installed;
        self.queue = queue
            .into_iter()
            .map(|item| (item.name, item.action))
            .collect();
        if records_moved {
            self.facts.clear();
            self.facts_pending = None;
            self.facts_pkgver = QString::from("");
            // A read still in flight answers for the previous system.
            self.facts_generation.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// The facts of one package as a `QVariantMap`, keyed as the details pane
    /// reads them. An xbps field that came back empty stays empty.
    fn facts_map(&self, name: &str, facts: &PackageFacts) -> QMap<QMapPair_QString_QVariant> {
        let text =
            |key: &str, value: &str| (QString::from(key), QVariant::from(&QString::from(value)));
        let flag = |key: &str, value: bool| (QString::from(key), QVariant::from(&value));
        let mut map = QMap::default();
        for (key, value) in [
            text("name", name),
            text("pkgver", &facts.pkgver),
            text("version", facts.version()),
            text(
                "installedVersion",
                if facts.installed { facts.version() } else { "" },
            ),
            text("short", &facts.short_desc),
            text("homepage", &facts.homepage),
            text("license", &facts.license),
            text("size", &facts.installed_size),
            text("download", &facts.filename_size),
            text("repo", &facts.repository),
            text("depends", &facts.depends.join(", ")),
            text("requiredBy", &facts.required_by.join(", ")),
        ] {
            map.insert_clone(&key, &value);
        }
        let installed = self.installed.get(name);
        for (key, value) in [
            flag("installed", facts.installed),
            flag("isUpdate", self.state.update_to(name).is_some()),
            flag("held", installed.is_some_and(|inst| inst.held)),
            flag("explicit", installed.is_some_and(|inst| inst.explicit)),
        ] {
            map.insert_clone(&key, &value);
        }
        let queued = self
            .queue
            .get(name)
            .map_or_else(String::new, |action| match action {
                QueueAction::Install => String::from("install"),
                QueueAction::Remove => String::from("remove"),
                QueueAction::Update => String::from("update"),
            });
        map.insert_clone(
            &QString::from("queuedAction"),
            &QVariant::from(&QString::from(queued)),
        );
        map
    }

    /// Whether `pattern` needs a read: the list does not hold its answer.
    ///
    /// The comparison is on the trimmed text, so the padding a search field
    /// keeps around the words never costs a second `xbps-query`. The empty
    /// pattern is the whole repository listing and has a slot of its own, so the
    /// first Available visit is not mistaken for an answer already on screen.
    fn needs_search(&self, pattern: &str) -> bool {
        self.answers.get(pattern).is_none()
    }

    /// Records that the list now shows the answer for `pattern`; an empty
    /// pattern records the repository listing, which is cached the same way.
    fn remember_search_answer(&mut self, pattern: &str, json: &QString) {
        self.answers.remember(pattern, json);
    }

    /// Drops the cached repository listing.
    ///
    /// Called when the repository index may have changed: the toolbar Refresh
    /// and the end of a job. The next empty request reads the repositories and
    /// stores the fresh answer. Typed answers are transient and stay.
    pub(crate) fn forget_listing(&mut self) {
        self.answers.forget_listing();
    }

    /// Starts a repository read and answers with the generation it owns.
    ///
    /// The package list reads synchronously on a worker thread; the generation
    /// is what the delivery compares, so an answer for a pattern the user has
    /// already typed past never reaches the window.
    fn begin_search_read(&self) -> u64 {
        self.search_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Whether `generation` is still the newest search read.
    fn search_read_is_current(&self, generation: u64) -> bool {
        Self::generation_is_current(generation, &self.search_generation)
    }

    fn generation_is_current(generation: u64, current: &AtomicU64) -> bool {
        current.load(Ordering::SeqCst) == generation
    }

    /// Runs one repository search for the package list and answers with the
    /// hits as JSON: `{"hits": [...], "errorKey": "…", "errorArgs": [...]}`.
    ///
    /// This is the read behind the list's search field, not the Query view's:
    /// it always asks the repositories (`xbps-query -Rs`), ranks the hits with
    /// the core's exact-name-first rule, and reports a failure as a key rather
    /// than rows. The caller runs it on a worker thread; nothing here touches
    /// a `QObject`.
    fn search_answer_json(&self, pattern: &str) -> QString {
        let pattern = pattern.trim();
        let (error, mut hits) = match read_search(&*self.invoke, SearchSource::Repository, pattern)
        {
            Ok(hits) => (None, hits),
            Err(err) => (Some(err), Vec::new()),
        };
        kxbps_core::rank_search_hits(&mut hits, pattern);
        let hits = hits_json(&hits);
        let (error_key, error_args) = match &error {
            Some(err) => (err.key(), err.args()),
            None => ("", Vec::new()),
        };
        QString::from(
            serde_json::json!({
                "hits": hits,
                "errorKey": error_key,
                "errorArgs": error_args,
            })
            .to_string(),
        )
    }

    /// The cached facts of one pkgver, if a read answered for it.
    fn facts_of(&self, pkgver: &str) -> Option<&PackageFacts> {
        self.facts.get(pkgver)
    }

    /// Starts a read for `pkgver` unless its facts are cached or in flight.
    fn begin_facts(&mut self, pkgver: &str) -> Option<u64> {
        if self.facts.contains_key(pkgver) || self.facts_pending.as_deref() == Some(pkgver) {
            return None;
        }
        self.facts_pending = Some(pkgver.to_owned());
        self.facts_pkgver = QString::from(pkgver);
        Some(self.facts_generation.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /// Moves the window to a new query; the previous answer stays until it
    /// lands.
    fn begin_search(&mut self, signature: SearchSignature) {
        self.signature = signature;
        self.search_error = QString::from("");
        self.search_error_args = QStringList::default();
        self.searching = true;
        self.search_generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Drops the answer of an emptied field and invalidates any read in
    /// flight.
    fn clear_answer(&mut self) {
        self.signature = SearchSignature::default();
        self.search_generation.fetch_add(1, Ordering::SeqCst);
        self.answer = SearchAnswer::default();
        self.search_error = QString::from("");
        self.search_error_args = QStringList::default();
        self.searching = false;
        self.raw_text = QString::from("");
        self.raw_command = QString::from("");
    }

    /// Applies a landed answer and reports which properties moved.
    ///
    /// A stale answer has no side effect. The caller emits exactly the
    /// notifications this names: the two raw properties declared NOTIFY, and
    /// nothing was ever emitting theirs before.
    fn announce_answer(
        &mut self,
        answer: &SearchAnswer,
        current: &SearchSignature,
    ) -> AnswerEffect {
        if answer.signature != *current {
            return AnswerEffect::default();
        }
        self.searching = false;
        let (text, command) = match &answer.raw {
            Some(raw) => (raw.text.as_str(), raw.command.as_str()),
            None => ("", ""),
        };
        let raw_text_moved = self.raw_text.to_string() != text;
        let raw_command_moved = self.raw_command.to_string() != command;
        self.raw_text = QString::from(text);
        self.raw_command = QString::from(command);
        match &answer.error {
            Some(err) => {
                self.search_error = QString::from(err.key());
                self.search_error_args = args_of(err);
            }
            None => {
                self.search_error = QString::from("");
                self.search_error_args = QStringList::default();
            }
        }
        AnswerEffect {
            fresh: true,
            raw_text_moved,
            raw_command_moved,
        }
    }

    /// Applies a finished facts read unless a newer one owns the window.
    fn finish_facts(&mut self, pkgver: String, result: Result<PackageFacts, SnapshotError>) {
        if self.facts_pending.as_deref() != Some(pkgver.as_str()) {
            return;
        }
        self.facts_pending = None;
        match result {
            Ok(facts) => {
                self.facts.insert(pkgver, facts);
                self.facts_error = QString::from("");
                self.facts_error_args = QStringList::default();
            }
            Err(err) => {
                self.facts_error = QString::from(err.key());
                self.facts_error_args = args_of(&err);
            }
        }
    }
}

impl qobject::QueryBackend {
    /// Runs the take on a worker thread and hands the output back on the GUI
    /// thread.
    fn run_search(self: Pin<&mut Self>, signature: SearchSignature) {
        let generation = self.rust().search_generation.load(Ordering::SeqCst);
        let thread = self.qt_thread();
        let current = self.search_generation.clone();
        let invoke = self.invoke.clone();
        let command = crate::xbps_runner::query_take_argv(signature.mode, &signature.query);
        std::thread::spawn(move || {
            let (raw, error) = match read_raw(&*invoke, &command) {
                Ok(raw) => (Some(raw), None),
                Err(err) => (None, Some(err)),
            };
            let answer = SearchAnswer {
                signature,
                error,
                raw,
            };
            // The closure runs on the GUI thread; `thread.queue` fails only
            // once the backend is gone, and then nobody wants the result.
            thread
                .queue(move |backend| backend.deliver_search(generation, &current, answer))
                .ok();
        });
    }

    /// Applies a landed search unless a newer query has started since.
    fn deliver_search(
        mut self: Pin<&mut Self>,
        generation: u64,
        current: &Arc<AtomicU64>,
        answer: SearchAnswer,
    ) {
        if current.load(Ordering::SeqCst) != generation {
            return;
        }
        let signature = self.rust().signature.clone();
        let effect = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.announce_answer(&answer, &signature)
        };
        if !effect.fresh {
            return;
        }
        // The newest answer becomes the one a repeated take is compared with,
        // so pressing Enter twice does not run xbps twice.
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.answer = answer;
        }
        self.as_mut().searching_changed();
        self.as_mut().search_error_changed();
        self.as_mut().search_error_args_changed();
        if effect.raw_text_moved {
            self.as_mut().raw_text_changed();
        }
        if effect.raw_command_moved {
            self.as_mut().raw_command_changed();
        }
    }

    /// Reads one package's facts on a worker thread.
    fn run_facts(self: Pin<&mut Self>, pkgver: String) {
        let generation = self.rust().facts_generation.load(Ordering::SeqCst);
        let thread = self.qt_thread();
        let current = self.facts_generation.clone();
        let invoke = self.invoke.clone();
        std::thread::spawn(move || {
            let name = split_pkgver(&pkgver)
                .map(|(name, _)| name.to_owned())
                .unwrap_or_else(|| pkgver.clone());
            let result = read_package(&*invoke, &name, &pkgver);
            thread
                .queue(move |backend| backend.deliver_facts(generation, &current, pkgver, result))
                .ok();
        });
    }

    /// Applies a landed facts read unless a newer one has started since.
    fn deliver_facts(
        mut self: Pin<&mut Self>,
        generation: u64,
        current: &Arc<AtomicU64>,
        pkgver: String,
        result: Result<PackageFacts, SnapshotError>,
    ) {
        if current.load(Ordering::SeqCst) != generation {
            return;
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.finish_facts(pkgver, result);
            data.facts_revision = data.facts_revision.wrapping_add(1);
        }
        self.as_mut().facts_error_changed();
        self.as_mut().facts_error_args_changed();
        self.as_mut().facts_pkgver_changed();
        self.as_mut().facts_revision_changed();
    }
}

/// The arguments of a translated error sentence, in `%1…` order.
fn args_of(err: &SnapshotError) -> QStringList {
    err.args()
        .iter()
        .map(|arg| QString::from(arg.as_str()))
        .collect::<QStringList>()
}

/// A trimmed query; a field of blanks searches for nothing.
fn trimmed(query: &cxx_qt_lib::QString) -> String {
    query.to_string().trim().to_owned()
}

/// The hits of one list-search answer, in the shape `SearchHit` serialises.
///
/// The model deserialises the same type on the other end, so the transport
/// cannot drift from it: a hand-written `json!({...})` is how `short` once
/// reached a reader that expected `description`.
fn hits_json(hits: &[kxbps_core::SearchHit]) -> Vec<serde_json::Value> {
    hits.iter()
        .filter_map(|hit| serde_json::to_value(hit).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kxbps_core::{Inst, InstalledMap};

    fn text_of(map: &QMap<QMapPair_QString_QVariant>, key: &str) -> String {
        map.get(&QString::from(key))
            .and_then(|value| value.value::<QString>())
            .map(|value| value.to_string())
            .unwrap_or_default()
    }

    fn flag_of(map: &QMap<QMapPair_QString_QVariant>, key: &str) -> bool {
        map.get(&QString::from(key))
            .and_then(|value| value.value::<bool>())
            .unwrap_or(false)
    }

    fn reply(exit_code: i32, stdout: &str, stderr: &str) -> String {
        serde_json::json!({
            "exitCode": exit_code,
            "stdout": stdout,
            "stderr": stderr,
            "timedOut": false
        })
        .to_string()
    }

    /// A transport that answers each take by every flag its argv carries.
    fn fake(calls: Vec<(&'static str, String)>) -> SharedInvoke {
        Arc::new(move |request: &str, _| {
            let argv: Vec<String> = serde_json::from_str(request).expect("request decodes");
            let mut key: Vec<&str> = Vec::new();
            for arg in argv.iter().skip(1) {
                if !arg.starts_with('-') {
                    break;
                }
                key.push(arg);
            }
            let key = key.join(" ");
            calls
                .iter()
                .find(|(flag, _)| *flag == key)
                .map(|(_, reply)| reply.clone())
                .unwrap_or_else(|| panic!("unexpected command: {argv:?}"))
        })
    }

    fn with_invoke(invoke: SharedInvoke) -> QueryBackendRust {
        QueryBackendRust {
            invoke,
            ..QueryBackendRust::default()
        }
    }

    /// A finished raw answer, as a worker would hand it back.
    fn raw_answer(signature: SearchSignature, command: &str, text: &str) -> SearchAnswer {
        SearchAnswer {
            signature,
            error: None,
            raw: Some(RawOutput {
                command: command.to_owned(),
                text: text.to_owned(),
            }),
        }
    }

    fn facts(pkgver: &str, installed: bool) -> PackageFacts {
        PackageFacts {
            installed,
            pkgver: pkgver.to_owned(),
            short_desc: format!("{pkgver} description"),
            homepage: "https://example.org".to_owned(),
            license: "MIT".to_owned(),
            installed_size: if installed {
                "2125KB".to_owned()
            } else {
                String::new()
            },
            repository: "https://repo.example/current".to_owned(),
            state: if installed {
                "installed"
            } else {
                "not installed"
            }
            .to_owned(),
            filename_size: "1735KB".to_owned(),
            depends: vec!["glibc>=2.41_1".to_owned()],
            required_by: vec!["base-system-0.114_2".to_owned()],
        }
    }

    #[test]
    fn a_raw_answer_publishes_its_text_and_command_and_asks_for_both_notifications() {
        let mut backend = QueryBackendRust::default();
        let signature = SearchSignature::new(QueryMode::X, "vim");
        backend.begin_search(signature.clone());
        let landed = raw_answer(signature.clone(), "xbps-query -x vim", "glibc>=2.41_1\n");
        let effect = backend.announce_answer(&landed, &signature);
        assert!(effect.fresh);
        assert!(
            effect.raw_text_moved,
            "the output block has to re-read its text"
        );
        assert!(
            effect.raw_command_moved,
            "the command label has to re-read its command"
        );
        assert_eq!(backend.raw_text.to_string(), "glibc>=2.41_1\n");
        assert_eq!(backend.raw_command.to_string(), "xbps-query -x vim");
        assert!(!backend.searching);
        assert_eq!(backend.search_error.to_string(), "");
    }

    #[test]
    fn a_stale_raw_answer_never_overwrites_a_newer_query() {
        let mut backend = QueryBackendRust::default();
        let slow = raw_answer(
            SearchSignature::new(QueryMode::X, "vim"),
            "xbps-query -x vim",
            "glibc>=2.41_1\n",
        );
        // The field moved on before the slow answer arrived.
        backend.begin_search(SearchSignature::new(QueryMode::X, "vi"));
        let current = backend.signature.clone();
        let effect = backend.announce_answer(&slow, &current);
        assert!(
            !effect.fresh,
            "an answer for a query nobody is asking any more is dropped"
        );
        assert!(!effect.raw_text_moved);
        assert_eq!(backend.raw_text.to_string(), "");
        assert_eq!(backend.raw_command.to_string(), "");
    }

    #[test]
    fn a_failed_raw_take_names_the_key_and_keeps_the_block_empty() {
        let mut backend = QueryBackendRust::default();
        let signature = SearchSignature::new(QueryMode::P, "pkgver vim");
        backend.begin_search(signature.clone());
        let landed = SearchAnswer {
            signature: signature.clone(),
            error: Some(SnapshotError::Exit {
                program: "xbps-query".into(),
                code: 2,
                stderr: "missing package".into(),
            }),
            raw: None,
        };
        let effect = backend.announce_answer(&landed, &signature);
        assert!(effect.fresh);
        assert!(
            !effect.raw_text_moved,
            "a failure prints no text and the empty state says so"
        );
        assert_eq!(backend.raw_text.to_string(), "");
        assert_eq!(backend.search_error.to_string(), "errorXbpsExit");
        assert_eq!(
            backend
                .search_error_args
                .iter()
                .map(QString::to_string)
                .collect::<Vec<_>>(),
            ["xbps-query", "2", "missing package"]
        );
    }

    #[test]
    fn an_empty_query_clears_the_text_the_command_and_the_error() {
        let mut backend = QueryBackendRust::default();
        let signature = SearchSignature::new(QueryMode::X, "vim");
        backend.begin_search(signature.clone());
        let landed = raw_answer(signature.clone(), "xbps-query -x vim", "glibc\n");
        backend.announce_answer(&landed, &signature);
        assert_eq!(backend.raw_text.to_string(), "glibc\n");

        backend.clear_answer();
        assert_eq!(
            backend.raw_text.to_string(),
            "",
            "an empty field answers nothing"
        );
        assert_eq!(backend.raw_command.to_string(), "");
        assert_eq!(backend.search_error.to_string(), "");
        assert!(!backend.searching);
        assert_eq!(backend.signature, SearchSignature::default());
    }

    #[test]
    fn a_new_installed_set_drops_the_facts_of_a_package_it_changed() {
        let mut backend = QueryBackendRust::default();
        // A package read while it was not installed: no size, not installed.
        backend.begin_facts("zramen-1.0.1_1");
        backend.finish_facts(
            "zramen-1.0.1_1".to_owned(),
            Ok(facts("zramen-1.0.1_1", false)),
        );
        assert_eq!(
            backend
                .facts_of("zramen-1.0.1_1")
                .map(|facts| facts.installed),
            Some(false)
        );

        let mut installed = InstalledMap::new();
        installed.insert(
            "zramen".to_owned(),
            Inst {
                version: "1.0.1_1".to_owned(),
                explicit: true,
                held: false,
                at: 0,
            },
        );
        backend.apply_snapshot(installed, Vec::new(), Vec::new(), Vec::new());

        assert!(
            backend.facts_of("zramen-1.0.1_1").is_none(),
            "a read from before the transaction must not answer for after it"
        );
        assert_eq!(
            backend.facts_pkgver.to_string(),
            "",
            "the pane re-reads, it does not echo a stale pkgver"
        );
    }

    /// One installed record, for the facts-cache tests.
    fn one_installed(name: &str, version: &str, explicit: bool, held: bool) -> InstalledMap {
        let mut installed = InstalledMap::new();
        installed.insert(
            name.to_owned(),
            Inst {
                version: version.to_owned(),
                explicit,
                held,
                at: 0,
            },
        );
        installed
    }

    /// A backend that has read the system once and holds one cached package.
    ///
    /// The first push establishes the installed set, the way the first read
    /// does; the facts are read after it, so the next push is the one under
    /// test.
    fn backend_with_facts(pkgver: &str, installed: &InstalledMap) -> QueryBackendRust {
        let mut backend = QueryBackendRust::default();
        backend.apply_snapshot(installed.clone(), Vec::new(), Vec::new(), Vec::new());
        backend.begin_facts(pkgver);
        backend.finish_facts(pkgver.to_owned(), Ok(facts(pkgver, true)));
        backend
    }

    #[test]
    fn a_state_push_that_keeps_the_installed_set_keeps_the_facts() {
        // A queue toggle pushes the same installed set; dropping the cache
        // would make the details pane re-read a package nothing changed.
        let installed = one_installed("zramen", "1.0.1_1", true, false);
        let mut backend = backend_with_facts("zramen-1.0.1_1", &installed);
        backend.apply_snapshot(installed.clone(), Vec::new(), Vec::new(), Vec::new());

        assert!(
            backend.facts_of("zramen-1.0.1_1").is_some(),
            "the installed record did not move, so the facts still describe it"
        );

        // The same push again, this time with a queue item: still identical.
        let queue = vec![QueueItem {
            name: "vim".into(),
            action: QueueAction::Install,
        }];
        backend.apply_snapshot(installed, queue, Vec::new(), Vec::new());
        assert!(
            backend.facts_of("zramen-1.0.1_1").is_some(),
            "a queue change alone does not invalidate the facts"
        );
    }

    #[test]
    fn a_changed_installed_record_drops_the_facts() {
        // Every field a fact read answers from must count as a change:
        // version, held and explicit.
        for (version, explicit, held) in [
            ("1.0.2_1", true, false),
            ("1.0.1_1", false, false),
            ("1.0.1_1", true, true),
        ] {
            let mut backend = backend_with_facts(
                "zramen-1.0.1_1",
                &one_installed("zramen", "1.0.1_1", true, false),
            );
            backend.apply_snapshot(
                one_installed("zramen", version, explicit, held),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            );
            assert!(
                backend.facts_of("zramen-1.0.1_1").is_none(),
                "version {version}, explicit {explicit}, held {held} changes the facts"
            );
        }
    }

    #[test]
    fn a_reset_that_empties_the_installed_set_drops_the_facts() {
        let mut backend = backend_with_facts(
            "zramen-1.0.1_1",
            &one_installed("zramen", "1.0.1_1", true, false),
        );
        backend.apply_snapshot(InstalledMap::new(), Vec::new(), Vec::new(), Vec::new());
        assert!(
            backend.facts_of("zramen-1.0.1_1").is_none(),
            "nothing is installed any more"
        );
        assert_eq!(backend.facts_pkgver.to_string(), "");
    }

    #[test]
    fn a_facts_read_for_an_older_pkgver_is_dropped() {
        let mut backend = QueryBackendRust::default();
        backend.begin_facts("vim-9.2.1031_1");
        backend.begin_facts("ark-25.08.0_1");
        backend.finish_facts(
            "vim-9.2.1031_1".to_owned(),
            Ok(facts("vim-9.2.1031_1", true)),
        );
        assert!(
            backend.facts_of("vim-9.2.1031_1").is_none(),
            "a newer selection owns the pane"
        );
        assert_eq!(backend.facts_pkgver.to_string(), "ark-25.08.0_1");
    }

    #[test]
    fn a_failed_facts_read_reports_the_key_and_leaves_the_fields_empty() {
        let mut backend = QueryBackendRust::default();
        backend.begin_facts("zramen-1.0.1_1");
        backend.finish_facts(
            "zramen-1.0.1_1".to_owned(),
            Err(SnapshotError::Exit {
                program: "xbps-query".into(),
                code: 2,
                stderr: "not found".into(),
            }),
        );
        assert!(backend.facts_of("zramen-1.0.1_1").is_none());
        assert_eq!(backend.facts_error.to_string(), "errorXbpsExit");
        assert_eq!(backend.facts_pkgver.to_string(), "zramen-1.0.1_1");
        assert_eq!(
            backend
                .facts_error_args
                .iter()
                .map(QString::to_string)
                .collect::<Vec<_>>(),
            ["xbps-query", "2", "not found"]
        );
    }

    #[test]
    fn the_facts_map_of_an_installed_package_holds_every_read_field() {
        let backend = QueryBackendRust::default();
        let map = backend.facts_map("vim", &facts("vim-9.2.1031_1", true));
        assert_eq!(text_of(&map, "name"), "vim");
        assert_eq!(text_of(&map, "version"), "9.2.1031_1");
        assert_eq!(text_of(&map, "installedVersion"), "9.2.1031_1");
        assert_eq!(text_of(&map, "short"), "vim-9.2.1031_1 description");
        assert_eq!(text_of(&map, "homepage"), "https://example.org");
        assert_eq!(text_of(&map, "license"), "MIT");
        assert_eq!(text_of(&map, "repo"), "https://repo.example/current");
        assert_eq!(text_of(&map, "size"), "2125KB");
        assert_eq!(text_of(&map, "download"), "1735KB");
        assert_eq!(text_of(&map, "depends"), "glibc>=2.41_1");
        assert_eq!(text_of(&map, "requiredBy"), "base-system-0.114_2");
        assert_eq!(text_of(&map, "queuedAction"), "");
        assert!(flag_of(&map, "installed"));
        assert!(!flag_of(&map, "isUpdate"));
    }

    #[test]
    fn the_facts_map_of_a_package_that_is_not_installed_leaves_the_unknown_fields_empty() {
        let backend = QueryBackendRust::default();
        let map = backend.facts_map("zramen", &facts("zramen-1.0.1_1", false));
        assert!(!flag_of(&map, "installed"));
        assert_eq!(text_of(&map, "version"), "1.0.1_1");
        assert_eq!(text_of(&map, "installedVersion"), "");
        assert_eq!(text_of(&map, "size"), "");
        assert_eq!(text_of(&map, "download"), "1735KB");
        assert!(!flag_of(&map, "held"));
        assert!(!flag_of(&map, "explicit"));
    }

    #[test]
    fn a_facts_map_answers_held_update_and_queue_from_the_snapshot() {
        let mut installed = InstalledMap::new();
        installed.insert(
            "vim".to_owned(),
            Inst {
                version: "9.2.1031_1".to_owned(),
                explicit: true,
                held: true,
                at: 7,
            },
        );
        let updates: Vec<UpdateEntry> = serde_json::from_str(
            r#"[{"name":"vim","version":"9.9","pkgver":"vim-9.9_1","action":"update",
                 "arch":"x86_64","repository":"https://r/current",
                 "installedSize":1,"downloadSize":1}]"#,
        )
        .unwrap();
        let mut backend = QueryBackendRust::default();
        backend.apply_snapshot(
            installed,
            vec![QueueItem {
                name: "vim".into(),
                action: QueueAction::Remove,
            }],
            updates,
            Vec::new(),
        );
        let map = backend.facts_map("vim", &facts("vim-9.2.1031_1", true));
        assert!(flag_of(&map, "held"));
        assert!(flag_of(&map, "explicit"));
        assert!(flag_of(&map, "isUpdate"));
        assert_eq!(text_of(&map, "queuedAction"), "remove");
    }

    #[test]
    fn a_second_selection_reuses_the_cached_facts() {
        let calls = Arc::new(AtomicU64::new(0));
        let counted = calls.clone();
        let invoke: SharedInvoke = Arc::new(move |_request: &str, _| {
            counted.fetch_add(1, Ordering::SeqCst);
            serde_json::json!({"exitCode": 0, "stdout": "", "stderr": "", "timedOut": false})
                .to_string()
        });
        let mut backend = QueryBackendRust {
            invoke,
            ..QueryBackendRust::default()
        };
        assert!(backend.begin_facts("vim-9.2.1031_1").is_some());
        backend.finish_facts("vim-9.2.1031_1".to_owned(), Ok(PackageFacts::default()));
        let before = calls.load(Ordering::SeqCst);
        assert!(
            backend.begin_facts("vim-9.2.1031_1").is_none(),
            "a cached pkgver never starts a second read"
        );
        assert_eq!(calls.load(Ordering::SeqCst), before);
    }

    /// The sorted names of a `read_search_now` reply, decoded the way QML's
    /// list update will read it.
    fn reply_names(json: &QString) -> Vec<String> {
        let value: serde_json::Value = serde_json::from_str(&json.to_string()).expect("JSON");
        value["hits"]
            .as_array()
            .expect("hits array")
            .iter()
            .map(|hit| hit["name"].as_str().expect("name").to_owned())
            .collect()
    }

    fn reply_error(json: &QString) -> String {
        let value: serde_json::Value = serde_json::from_str(&json.to_string()).expect("JSON");
        value["errorKey"].as_str().unwrap_or_default().to_owned()
    }

    #[test]
    fn a_read_search_now_ranks_the_exact_name_first() {
        // The three lines xbps prints for "vim" in the order it prints them:
        // the exact match is not first.
        let backend = with_invoke(fake(vec![(
            "-Rs",
            reply(
                0,
                "[-] apvlv-0.5.0_1 Vim-like viewer\n\
                 [-] vimb-4.0.0_1 WebKit browser\n\
                 [-] vim-9.2.1031_1 Vim editor\n",
                "",
            ),
        )]));
        let json = backend.search_answer_json("vim");
        assert_eq!(reply_names(&json), ["vim", "vimb", "apvlv"]);
        assert_eq!(reply_error(&json), "", "a good read names no error");
    }

    #[test]
    fn a_read_search_now_carries_the_error_key_instead_of_rows() {
        let backend = with_invoke(fake(vec![("-Rs", reply(2, "", "broken index"))]));
        let json = backend.search_answer_json("vim");
        assert_eq!(reply_error(&json), "errorXbpsExit");
        assert!(
            reply_names(&json).is_empty(),
            "a failed read never invents a row"
        );
    }

    #[test]
    fn a_trimmed_pattern_that_already_answered_is_not_read_again() {
        let calls = Arc::new(AtomicU64::new(0));
        let counted = calls.clone();
        let invoke: SharedInvoke = Arc::new(move |_request: &str, _| {
            counted.fetch_add(1, Ordering::SeqCst);
            reply(0, "[-] vim-9.2.1031_1 Vim editor\n", "")
        });
        let mut backend = QueryBackendRust {
            invoke,
            ..QueryBackendRust::default()
        };
        assert!(backend.needs_search("vim"), "a new pattern is read");
        backend.remember_search_answer("vim", &QString::from(r#"{"hits":[]}"#));
        assert_eq!(calls.load(Ordering::SeqCst), 0, "remembering reads nothing");
        assert!(
            !backend.needs_search(" vim "),
            "the same pattern, padded, is the answer already on screen"
        );
        assert!(
            backend.needs_search("vim-"),
            "a longer pattern is a new read"
        );
        backend.remember_search_answer("", &QString::from(r#"{"hits":[]}"#));
        assert!(
            backend.needs_search("vim"),
            "a fresh listing forgets the typed answer it may have moved past"
        );
    }

    #[test]
    fn an_older_search_read_is_dropped_when_a_newer_pattern_started() {
        let backend = QueryBackendRust::default();
        let older = backend.begin_search_read();
        assert!(
            backend.search_read_is_current(older),
            "a fresh read owns the window"
        );
        // The user typed on: a newer read bumps the generation.
        let newer = backend.begin_search_read();
        assert_ne!(older, newer, "each read is a new generation");
        assert!(
            backend.search_read_is_current(newer),
            "the newest read owns the window"
        );
        assert!(
            !backend.search_read_is_current(older),
            "the older answer lost its turn and is dropped"
        );
    }

    #[test]
    fn a_list_search_answer_deserialises_as_the_model_reads_it() {
        use kxbps_core::SearchHit;

        let hits = vec![
            SearchHit {
                installed: false,
                name: "vim".to_owned(),
                version: "9.2.1031_1".to_owned(),
                pkgver: "vim-9.2.1031_1".to_owned(),
                description: "Vim editor (vi clone) without GUI".to_owned(),
            },
            SearchHit {
                installed: true,
                name: "vimb".to_owned(),
                version: "4.0.0_1".to_owned(),
                pkgver: "vimb-4.0.0_1".to_owned(),
                description: "Vim-like browser".to_owned(),
            },
        ];
        let answer = serde_json::json!({
            "hits": hits_json(&hits),
            "errorKey": "",
            "errorArgs": [],
        });
        // What the package list does with `hits`: the model parses this very
        // string, so the transport shape and its reader are one contract.
        let parsed: Vec<SearchHit> =
            serde_json::from_str(&answer["hits"].to_string()).expect("the model reads these hits");
        assert_eq!(
            parsed, hits,
            "every field survives the trip, description included"
        );
    }

    #[test]
    fn a_cached_listing_answers_the_second_request_with_the_stored_json() {
        // The window sets `searchLoading` before it asks and only an answer
        // clears it, so the backend may skip the read but never the answer.
        // The stored JSON is exactly what `read_list_search` hands to
        // `listSearchDone` on a cache hit.
        let mut backend = QueryBackendRust::default();
        assert!(
            backend.needs_search(""),
            "the first Available visit reads the listing"
        );
        let json = QString::from(r#"{"hits":[],"errorKey":"","errorArgs":[]}"#);
        backend.remember_search_answer("", &json);
        assert!(
            !backend.needs_search(""),
            "the listing is cached after the answer lands"
        );
        assert_eq!(
            backend.answers.get("").map(QString::to_string),
            Some(json.to_string()),
            "the second request is answered with the stored JSON, not silence"
        );
    }

    #[test]
    fn invalidate_list_search_forgets_the_listing() {
        let mut backend = QueryBackendRust::default();
        backend.remember_search_answer("", &QString::from(r#"{"hits":[]}"#));
        backend.forget_listing();
        assert!(
            backend.needs_search(""),
            "after an invalidation the next empty request reads again"
        );
        assert!(
            backend.answers.get("").is_none(),
            "the invalidated answer is not handed out"
        );
    }

    #[test]
    fn a_typed_pattern_does_not_evict_the_listing() {
        // Clearing the field must be instant: a search for "vim" may not push
        // the twenty thousand rows the listing holds out of the cache.
        let mut backend = QueryBackendRust::default();
        let listing = QString::from(r#"{"hits":[1]}"#);
        let typed = QString::from(r#"{"hits":[2]}"#);
        backend.remember_search_answer("", &listing);
        backend.remember_search_answer("vim", &typed);
        assert_eq!(
            backend.answers.get("").map(QString::to_string),
            Some(listing.to_string())
        );
        assert_eq!(
            backend.answers.get("vim").map(QString::to_string),
            Some(typed.to_string())
        );
        backend.remember_search_answer("emacs", &typed);
        assert_eq!(
            backend.answers.get("vim"),
            None,
            "the typed slot keeps only the newest pattern"
        );
        assert_eq!(
            backend.answers.get("").map(QString::to_string),
            Some(listing.to_string()),
            "the listing survives every typed search"
        );
    }
}
