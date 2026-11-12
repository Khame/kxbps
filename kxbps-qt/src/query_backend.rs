//! Query façade for QML: the repository search and the facts of one package,
//! both read from xbps through the runner.
//!
//! The package *model* is owned by [`crate::installed_backend::InstalledBackend`],
//! because it needs the store's installed and queue snapshots. This object
//! answers `search()` / `results()` and `get()` — and it receives the *same*
//! snapshot from that backend (`queryBackend` in QML), so a search hit answers
//! `isUpdate`/`held` from the same source as a list row.
//!
//! Neither answer comes from the embedded catalog: a search runs
//! `xbps-query -Rs <pattern>` (or `-s` for a mode that lists the installed
//! set) and the facts of one package run the two property takes and the two
//! dependency lists, both on a worker thread through the same injected
//! transport as the snapshot reads. The catalog is still loaded for the first
//! frame of the package list; 17b3 retires it.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cxx_qt::CxxQtType;
use cxx_qt::Threading;
use cxx_qt_lib::{QList, QMap, QMapPair_QString_QVariant, QString, QStringList, QVariant};
use kxbps_core::{InstalledMap, QueueAction, QueueItem, SearchHit, UpdateEntry, split_pkgver};

use crate::package_model::{self, InstalledState, RowView};
use crate::xbps_snapshot::{
    PackageFacts, RawOutput, SearchSource, SharedInvoke, SnapshotError, read_package, read_raw,
    read_search,
};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/core/qlist/qlist_QVariant.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;

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
        #[qproperty(i32, result_count, READ, NOTIFY)]
        #[qproperty(i32, results_revision, READ, NOTIFY)]
        /// Whether the newest query is still being read.
        #[qproperty(bool, searching, READ, NOTIFY)]
        // A mode that is not a package list prints text, not rows: the text of
        // the newest `read_raw` answer and the command it came from. Empty
        // `raw_text` means the newest answer was a list.
        #[qproperty(QString, raw_text, READ, NOTIFY)]
        #[qproperty(QString, raw_command, READ, NOTIFY)]
        #[qproperty(QString, facts_error, READ, NOTIFY)]
        #[qproperty(QStringList, facts_error_args, READ, NOTIFY)]
        #[qproperty(QString, facts_pkgver, READ, NOTIFY)]
        #[qproperty(i32, facts_revision, READ, NOTIFY)]
        type QueryBackend = super::QueryBackendRust;

        #[qinvokable]
        fn version(self: &QueryBackend) -> QString;

        /// Runs a search on a worker thread. The hits of the previous answer
        /// stay until it lands; QML re-reads through `resultsRevision`.
        #[qinvokable]
        fn search(self: Pin<&mut QueryBackend>, query: &QString, mode: &QString);

        /// The hits of the newest answer, as role-keyed maps.
        #[qinvokable]
        fn results(self: &QueryBackend) -> QList_QVariant;

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
    /// Which database the newest search reads.
    search_source: SearchSource,
    search_error: QString,
    search_error_args: QStringList,
    result_count: i32,
    results_revision: i32,
    searching: bool,
    /// The text a mode that is not a package list printed, and its command.
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
}

/// What a search is asked for: a moved signature makes an answer stale.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SearchSignature {
    source: SearchSource,
    /// The `-Rs`/`-s` token or, for a mode that is not a package list, the
    /// `-L`/`-x`/… token. Empty for the default repository search.
    mode: String,
    query: String,
}

impl Default for SearchSignature {
    /// The signature of no query at all.
    fn default() -> Self {
        Self {
            source: SearchSource::Repository,
            mode: String::new(),
            query: String::new(),
        }
    }
}

impl SearchSignature {
    fn new(source: SearchSource, mode: &str, query: &str) -> Self {
        Self {
            source,
            mode: mode.to_owned(),
            query: query.to_owned(),
        }
    }

    /// Whether this signature asks for a package list at all.
    fn lists_packages(&self) -> bool {
        self.mode.is_empty()
    }
}

/// One answer of a search, tagged with the signature it answers.
///
/// A failed search keeps the error next to no hits: the window shows the key
/// and never a row it invented.
#[derive(Default)]
struct SearchAnswer {
    signature: SearchSignature,
    error: Option<SnapshotError>,
    hits: Vec<SearchHit>,
    /// The text of a mode that is not a package list; `None` for a list.
    raw: Option<RawOutput>,
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
            search_source: SearchSource::Repository,
            search_error: QString::from(""),
            search_error_args: QStringList::default(),
            result_count: 0,
            results_revision: 0,
            searching: false,
            raw_text: QString::from(""),
            raw_command: QString::from(""),
            facts: HashMap::new(),
            facts_pending: None,
            facts_error: QString::from(""),
            facts_error_args: QStringList::default(),
            facts_pkgver: QString::from(""),
            facts_revision: 0,
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

    /// One `xbps-query` search, on a worker thread; `resultsRevision` re-reads
    /// the hits.
    fn search(mut self: Pin<&mut Self>, query: &cxx_qt_lib::QString, mode: &cxx_qt_lib::QString) {
        let mode = mode.to_string();
        let source = self.rust().source_for_mode(&mode);
        let list_mode = self.rust().lists_packages(&mode);
        let query = trimmed(query);
        let signature = if list_mode {
            SearchSignature::new(source, "", &query)
        } else {
            SearchSignature::new(source, &QueryBackendRust::raw_flag(&mode), &query)
        };
        let empty = list_mode && query.is_empty();
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            // The newest answer already covers this take; nothing to read.
            if data.answer.signature == signature {
                return;
            }
            if empty {
                // A listing mode needs a pattern: an empty field answers
                // nothing, and any read still in flight is now stale.
                data.clear_answer();
                data.results_revision = data.results_revision.wrapping_add(1);
            } else {
                data.begin_search(signature.clone());
            }
        }
        self.as_mut().searching_changed();
        if empty {
            self.as_mut().result_count_changed();
            self.as_mut().results_revision_changed();
            return;
        }
        self.as_mut().run_search(signature);
    }

    /// The hits of the newest answer, as role-keyed maps.
    fn results(&self) -> QList<QVariant> {
        QList::from(
            self.rust()
                .answer
                .hits
                .iter()
                .map(|hit| QVariant::from(&self.hit_map(hit)))
                .collect::<Vec<_>>(),
        )
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

    /// Which database a mode searches. Modes that list a set read the
    /// installed database; every other mode is a repository search, which is
    /// all `xbps-query` offers for a pattern.
    fn source_for_mode(&self, token: &str) -> SearchSource {
        match token {
            "l" | "m" | "O" | "H" => SearchSource::Installed,
            _ => SearchSource::Repository,
        }
    }

    /// Whether the mode answers with package rows. `-Rs`/`-s` search and
    /// `-l`/`-m`/`-O`/`-H` list a set; every other mode prints something that
    /// is not a list of packages (`-L` repositories, `-x` dependencies, …),
    /// and a row built from one of its lines would be invented.
    fn lists_packages(&self, token: &str) -> bool {
        matches!(token, "Rs" | "s" | "l" | "m" | "O" | "H")
    }

    /// The `xbps-query` flag of a QML mode token: `"Rs"` is `-Rs`, `"p"` is
    /// `-p`, and the rest read the same way.
    fn raw_flag(token: &str) -> String {
        format!("-{token}")
    }

    /// One hit as a `QVariantMap`, keyed like a model row plus `pkgver`.
    ///
    /// Only what the search printed is filled in: the tag decides `installed`,
    /// the version is the one the hit carries, and the installed record fills
    /// the rest when the snapshot knows the package. Size, category and
    /// repository stay empty, because a `-Rs` line names none of them.
    fn hit_map(&self, hit: &SearchHit) -> QMap<QMapPair_QString_QVariant> {
        let installed = self.installed.get(&hit.name);
        let queued = self.queue.get(&hit.name).copied();
        let update_to = self.state.update_to(&hit.name);
        let row = RowView {
            name: &hit.name,
            version: &hit.version,
            installed,
            queued,
            update_to,
            description: Some(&hit.description),
            // A `-Rs` line names no size; the list column for a search result
            // stays empty rather than borrowing the installed package's size
            // for a repository hit of another version.
            size: None,
            update_size: None,
            download: None,
        };
        let mut map = QMap::default();
        for (_, name) in package_model::ROLES {
            if let Some(value) = package_model::field_value(name, &row) {
                map.insert_clone(&QString::from(name), &value);
            }
        }
        map.insert_clone(
            &QString::from("pkgver"),
            &QVariant::from(&QString::from(hit.pkgver.clone())),
        );
        map
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
        self.search_source = signature.source;
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
        self.result_count = 0;
        self.searching = false;
        self.raw_text = QString::from("");
        self.raw_command = QString::from("");
    }

    /// Drops a stale answer with no side effect; a fresh one names its error
    /// and counts its hits.
    fn announce_answer(&mut self, answer: &SearchAnswer, current: &SearchSignature) -> bool {
        if answer.signature != *current {
            return false;
        }
        self.searching = false;
        self.result_count = i32::try_from(answer.hits.len()).unwrap_or(i32::MAX);
        match &answer.raw {
            Some(raw) => {
                self.raw_text = QString::from(raw.text.as_str());
                self.raw_command = QString::from(raw.command.as_str());
            }
            None => {
                self.raw_text = QString::from("");
                self.raw_command = QString::from("");
            }
        }
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
        true
    }

    /// Applies a fresh answer, unless the newest one already has those hits.
    fn finish_answer(&mut self, answer: SearchAnswer, current: &SearchSignature) -> bool {
        if answer.signature != *current {
            return false;
        }
        let same = self.answer.signature == answer.signature && self.answer.hits == answer.hits;
        self.answer = answer;
        !same
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
    /// Reads the hits on a worker thread and hands them back on the GUI thread.
    fn run_search(self: Pin<&mut Self>, signature: SearchSignature) {
        let generation = self.rust().search_generation.load(Ordering::SeqCst);
        let thread = self.qt_thread();
        let current = self.search_generation.clone();
        let invoke = self.invoke.clone();
        std::thread::spawn(move || {
            let answer = if signature.lists_packages() {
                let result = read_search(&*invoke, signature.source, &signature.query);
                SearchAnswer {
                    signature,
                    error: result.as_ref().err().cloned(),
                    hits: result.unwrap_or_default(),
                    raw: None,
                }
            } else {
                let result = read_raw(&*invoke, &signature.mode, &signature.query);
                let (raw, error) = match result {
                    Ok(raw) => (Some(raw), None),
                    Err(err) => (None, Some(err)),
                };
                SearchAnswer {
                    signature,
                    error,
                    hits: Vec::new(),
                    raw,
                }
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
        let fresh = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.announce_answer(&answer, &signature)
        };
        if !fresh {
            return;
        }
        let changed = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.finish_answer(answer, &signature)
        };
        self.as_mut().searching_changed();
        self.as_mut().result_count_changed();
        self.as_mut().search_error_changed();
        self.as_mut().search_error_args_changed();
        if changed {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.results_revision = data.results_revision.wrapping_add(1);
            self.as_mut().results_revision_changed();
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

    fn hit(name: &str, version: &str, installed: bool) -> SearchHit {
        SearchHit {
            installed,
            name: name.to_owned(),
            version: version.to_owned(),
            pkgver: format!("{name}-{version}"),
            description: format!("{name} description"),
        }
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

    /// A finished answer for `query`, as a worker would hand it back.
    fn answer(source: SearchSource, query: &str, hits: Vec<SearchHit>) -> SearchAnswer {
        SearchAnswer {
            signature: SearchSignature::new(source, "", query),
            error: None,
            hits,
            raw: None,
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
    fn every_mode_token_selects_the_database_it_names() {
        let backend = QueryBackendRust::default();
        for token in ["l", "m", "O", "H"] {
            assert_eq!(
                backend.source_for_mode(token),
                SearchSource::Installed,
                "token {token}"
            );
        }
        for token in [
            "Rs", "s", "R", "show", "x", "X", "f", "o", "L", "p", "nonsense",
        ] {
            assert_eq!(
                backend.source_for_mode(token),
                SearchSource::Repository,
                "token {token}"
            );
        }
    }

    #[test]
    fn a_finished_search_replaces_the_hits_and_the_count() {
        let mut backend = with_invoke(fake(vec![("-Rs", reply(0, "", ""))]));
        backend.begin_search(SearchSignature::new(SearchSource::Repository, "", "lib"));
        let signature = backend.signature.clone();
        let landed = answer(
            SearchSource::Repository,
            "lib",
            vec![hit("lib", "6.18.63_1", true), hit("libx", "1_1", false)],
        );
        assert!(backend.announce_answer(&landed, &signature));
        let changed = backend.finish_answer(landed, &signature);
        assert!(changed, "a new set of hits is a new revision");
        assert_eq!(backend.result_count, 2);
        assert_eq!(backend.answer.hits[0].name, "lib");
        assert!(!backend.searching);
        assert_eq!(backend.search_error.to_string(), "");
    }

    #[test]
    fn a_stale_search_answer_never_overwrites_a_newer_query() {
        let mut backend = with_invoke(fake(vec![("-Rs", reply(0, "", ""))]));
        backend.begin_search(SearchSignature::new(SearchSource::Repository, "", "lib"));
        let slow = answer(
            SearchSource::Repository,
            "lib",
            vec![hit("lib", "6.18.63_1", true)],
        );
        // The field moved on before the slow answer arrived.
        backend.begin_search(SearchSignature::new(SearchSource::Repository, "", "li"));
        let current = backend.signature.clone();
        assert!(
            !backend.announce_answer(&slow, &current),
            "an answer for a query nobody is asking any more is dropped"
        );
        assert!(
            !backend.finish_answer(slow, &current),
            "and it never becomes the window"
        );
        assert_eq!(backend.result_count, 0);
        assert_eq!(backend.answer.hits.len(), 0);
    }

    #[test]
    fn a_failed_search_keeps_no_rows_and_names_the_key() {
        let mut backend = with_invoke(fake(vec![("-Rs", reply(0, "", ""))]));
        backend.begin_search(SearchSignature::new(SearchSource::Repository, "", "lib"));
        let signature = backend.signature.clone();
        let landed = SearchAnswer {
            signature: signature.clone(),
            error: Some(SnapshotError::Exit {
                program: "xbps-query".into(),
                code: 2,
                stderr: "broken index".into(),
            }),
            hits: Vec::new(),
            raw: None,
        };
        assert!(backend.announce_answer(&landed, &signature));
        backend.finish_answer(landed, &signature);
        assert_eq!(backend.result_count, 0, "a failure leaves no rows");
        assert!(backend.answer.hits.is_empty());
        assert_eq!(backend.search_error.to_string(), "errorXbpsExit");
        assert_eq!(
            backend
                .search_error_args
                .iter()
                .map(QString::to_string)
                .collect::<Vec<_>>(),
            ["xbps-query", "2", "broken index"]
        );
    }

    #[test]
    fn a_mode_that_is_not_a_list_answers_with_text_and_no_rows() {
        let mut backend = with_invoke(fake(vec![(
            "-L",
            reply(0, "1 https://repo.voidlinux.org/current (RSA signed)\n", ""),
        )]));
        // `-L` never reads the installed database and never parses a hit.
        assert!(!backend.lists_packages("L"));
        backend.begin_search(SearchSignature::new(SearchSource::Repository, "L", ""));
        let signature = backend.signature.clone();
        let raw = read_raw(
            &*backend.invoke.clone(),
            &QueryBackendRust::raw_flag("L"),
            "",
        )
        .expect("a repository list lands");
        let landed = SearchAnswer {
            signature: signature.clone(),
            error: None,
            hits: Vec::new(),
            raw: Some(raw),
        };
        backend.announce_answer(&landed, &signature);
        backend.finish_answer(landed, &signature);
        assert_eq!(backend.result_count, 0, "no row is invented from a line");
        assert!(backend.answer.hits.is_empty());
        assert_eq!(backend.raw_command.to_string(), "xbps-query -L");
        assert!(
            backend
                .raw_text
                .to_string()
                .starts_with("1 https://repo.voidlinux.org/current"),
            "the block shows what xbps printed"
        );
    }

    #[test]
    fn only_the_list_modes_are_read_as_packages() {
        let backend = QueryBackendRust::default();
        for token in ["Rs", "s", "l", "m", "O", "H"] {
            assert!(backend.lists_packages(token), "token {token}");
        }
        for token in ["L", "show", "x", "X", "f", "o", "R", "p"] {
            assert!(!backend.lists_packages(token), "token {token}");
        }
        assert_eq!(QueryBackendRust::raw_flag("show"), "-show");
        assert_eq!(QueryBackendRust::raw_flag("Rs"), "-Rs");
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
    fn an_empty_query_clears_the_rows_and_the_error() {
        let mut backend = with_invoke(fake(vec![("-Rs", reply(0, "", ""))]));
        backend.begin_search(SearchSignature::new(SearchSource::Repository, "", "lib"));
        let signature = backend.signature.clone();
        let landed = answer(
            SearchSource::Repository,
            "lib",
            vec![hit("lib", "6.18.63_1", true)],
        );
        backend.announce_answer(&landed, &signature);
        backend.finish_answer(landed, &signature);
        assert_eq!(backend.result_count, 1);

        backend.clear_answer();
        assert_eq!(backend.result_count, 0, "an empty query answers nothing");
        assert!(backend.answer.hits.is_empty());
        assert!(!backend.searching);
        assert_eq!(backend.search_error.to_string(), "");
        assert_eq!(backend.signature, SearchSignature::default());
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
    fn a_search_hit_map_carries_the_snapshot_and_no_invented_facts() {
        let backend = QueryBackendRust::default();
        let map = backend.hit_map(&hit("libx", "1_1", false));
        assert_eq!(text_of(&map, "name"), "libx");
        assert_eq!(text_of(&map, "version"), "1_1");
        assert_eq!(text_of(&map, "pkgver"), "libx-1_1");
        assert_eq!(text_of(&map, "installedVersion"), "");
        assert_eq!(text_of(&map, "short"), "libx description");
        assert_eq!(text_of(&map, "repo"), "", "`-Rs` reports no repository");
        assert_eq!(text_of(&map, "category"), "", "and no category");
        assert_eq!(text_of(&map, "size"), "", "and no size");
        assert!(!flag_of(&map, "installed"));
    }

    #[test]
    fn a_search_hit_map_reports_the_installed_record_it_knows() {
        let mut installed = InstalledMap::new();
        installed.insert(
            "vim".to_owned(),
            Inst {
                version: "9.2.1031_1".to_owned(),
                explicit: true,
                held: false,
                at: 7,
            },
        );
        let mut backend = QueryBackendRust::default();
        backend.apply_snapshot(installed, Vec::new(), Vec::new(), Vec::new());
        let map = backend.hit_map(&hit("vim", "9.2.1031_1", true));
        assert_eq!(text_of(&map, "installedVersion"), "9.2.1031_1");
        assert!(
            flag_of(&map, "explicit"),
            "the hit repeats the installed record"
        );
        assert_eq!(text_of(&map, "short"), "vim description");
        assert!(!flag_of(&map, "isUpdate"), "nothing is pending for vim");
        assert!(!flag_of(&map, "held"));
        assert_eq!(text_of(&map, "queuedAction"), "");
    }

    #[test]
    fn a_search_hit_map_reports_the_pending_update_and_the_queue() {
        let mut installed = InstalledMap::new();
        installed.insert(
            "vim".to_owned(),
            Inst {
                version: "9.2.1031_1".to_owned(),
                explicit: true,
                held: false,
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
                action: QueueAction::Update,
            }],
            updates,
            Vec::new(),
        );
        let map = backend.hit_map(&hit("vim", "9.2.1031_1", true));
        assert!(flag_of(&map, "isUpdate"));
        assert_eq!(text_of(&map, "updateVersion"), "9.9");
        assert_eq!(text_of(&map, "queuedAction"), "update");
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
}
