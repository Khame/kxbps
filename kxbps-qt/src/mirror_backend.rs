//! The mirror view's backend: the model, the selection and the probe batch.
//!
//! Probing is the one part of the app that talks to the network, so it is also
//! the one part that must not run on the GUI thread. `probe_all` marks every row
//! `running`, then hands the list to a `std::thread`; that thread owns one
//! `QNetworkAccessManager` for the whole batch and walks the mirrors one at a
//! time (34 parallel requests would be rude and would measure queueing, not the
//! mirror). Every result is delivered back with `qt_thread().queue(...)`, which
//! is the only way CXX-Qt lets a background thread touch a QObject, and each one
//! updates a single row.
//!
//! Two rules come from the core, not from here: [`check_probe_url`] refuses a
//! plain-HTTP mirror *before* any request is made, and the request budget is
//! [`PROBE_TIMEOUT_MS`]. A refused mirror therefore never reaches the network
//! and lands in the `fail` state with the `plainHttp` key.
//!
//! The selected mirror is persisted by [`crate::installed_backend`], the single
//! owner of the [`Store`](kxbps_core::Store); this backend reads it at startup
//! and writes through the store on every change, so a restart remembers it.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use cxx_qt::casting::Upcast;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QObjectMutPtr, QString};
use kxbps_core::{DEFAULT_MIRROR, MIRRORS, PROBE_TIMEOUT_MS, ProbeError, check_probe_url, measure};

use crate::mirror_model::{STATE_FAIL, STATE_OK, STATE_RUNNING};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qobjectmutptr.h");
        type QObjectMutPtr = cxx_qt_lib::QObjectMutPtr;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        // Model factory, per-row updates and the `HEAD` transport.
        include!("mirror_shim.h");

        #[cxx_name = "kxbps_create_mirror_model"]
        fn create_mirror_model(parent: QObjectMutPtr) -> QObjectMutPtr;

        #[cxx_name = "kxbps_mirror_model_set_all_running"]
        fn mirror_model_set_all_running(model: QObjectMutPtr);

        #[cxx_name = "kxbps_mirror_model_finish_running"]
        fn mirror_model_finish_running(model: QObjectMutPtr);

        #[cxx_name = "kxbps_mirror_model_set_result"]
        fn mirror_model_set_result(
            model: QObjectMutPtr,
            row: i32,
            state_key: &QString,
            latency_ms: u32,
            error_key: &QString,
        );

        #[cxx_name = "kxbps_probe_batch_begin"]
        fn probe_batch_begin() -> usize;

        #[cxx_name = "kxbps_probe_batch_end"]
        fn probe_batch_end(handle: usize);

        #[cxx_name = "kxbps_probe_head"]
        fn probe_head(handle: usize, url: &QString, timeout_ms: u32) -> i32;

        // True once the application is shutting down; the batch polls it between
        // mirrors and inside a request, so closing the window stops the sweep.
        #[cxx_name = "kxbps_probe_shutdown_requested"]
        fn probe_shutdown_requested() -> bool;

        // The store lives in the installed backend; the mirror is read and
        // written through it so there is only ever one `Store`.
        include!("backend_shim.h");

        #[cxx_name = "kxbps_installed_mirror"]
        fn installed_mirror(owner: QObjectMutPtr) -> QString;

        #[cxx_name = "kxbps_installed_set_mirror"]
        fn installed_set_mirror(owner: QObjectMutPtr, url: &QString);
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QObjectMutPtr, model, READ, CONSTANT)]
        #[qproperty(QObjectMutPtr, owner, READ, WRITE = set_owner, NOTIFY)]
        #[qproperty(QString, current_mirror, READ, WRITE = set_current_mirror, NOTIFY)]
        #[qproperty(bool, probing, READ, NOTIFY)]
        #[qproperty(i32, finished_count, READ, NOTIFY)]
        type MirrorBackend = super::MirrorBackendRust;

        #[qinvokable]
        fn probe_all(self: Pin<&mut MirrorBackend>);

        #[qinvokable]
        fn probe_one(self: Pin<&mut MirrorBackend>, url: &QString);

        /// Emitted once per batch, after the last row has its result.
        #[qsignal]
        fn probe_finished(self: Pin<&mut MirrorBackend>);

        fn set_owner(self: Pin<&mut MirrorBackend>, value: QObjectMutPtr);

        fn set_current_mirror(self: Pin<&mut MirrorBackend>, value: QString);

        // Called from the queued closures below; not invokable, because QML must
        // not be able to write a result the transport did not produce. The
        // result crosses the bridge as its three wire values, since a CXX-Qt
        // method signature can only name CXX types.
        fn apply_probe_result(
            self: Pin<&mut MirrorBackend>,
            row: i32,
            batch: u64,
            state_key: &QString,
            latency_ms: u32,
            error_key: &QString,
        );

        fn apply_single_result(
            self: Pin<&mut MirrorBackend>,
            row: i32,
            state_key: &QString,
            latency_ms: u32,
            error_key: &QString,
        );

        // Ends a batch that stopped early (shutdown, teardown or a newer batch):
        // re-enables probing and returns the unanswered rows to `idle`.
        fn finish_cancelled(self: Pin<&mut MirrorBackend>, batch: u64);
    }

    impl cxx_qt::Constructor<()> for MirrorBackend {}
    impl cxx_qt::Threading for MirrorBackend {}
}

/// One probe's wire result: the three values the model and the counters need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeOutcome {
    /// `ok` or `fail`.
    pub state_key: &'static str,
    /// Milliseconds for an `ok` outcome, `0` otherwise.
    pub latency_ms: u32,
    /// Core probe key for a `fail` outcome, `""` otherwise.
    pub error_key: &'static str,
}

impl ProbeOutcome {
    /// Maps the transport's return code onto a state; `None` when the request
    /// was abandoned because the application is shutting down.
    ///
    /// `0` — the mirror answered, `1` — the timer won, `2` — the request never
    /// reached an answer (DNS, TLS, refused), `3` — cancelled, so there is no
    /// result to show.
    pub fn classify(code: i32, latency_ms: u32) -> Option<Self> {
        match code {
            0 => Some(Self {
                state_key: STATE_OK,
                latency_ms,
                error_key: "",
            }),
            1 => Some(Self {
                state_key: STATE_FAIL,
                latency_ms: 0,
                error_key: ProbeError::Timeout.key(),
            }),
            3 => None,
            _ => Some(Self {
                state_key: STATE_FAIL,
                latency_ms: 0,
                error_key: ProbeError::NoResponse.key(),
            }),
        }
    }

    /// The outcome of a mirror that never gets a request: the core rule says a
    /// plain-HTTP URL is refused before the network is touched.
    pub fn refused(error: ProbeError) -> Self {
        Self {
            state_key: STATE_FAIL,
            latency_ms: 0,
            error_key: error.key(),
        }
    }
}

/// Rust side of [`qobject::MirrorBackend`].
pub struct MirrorBackendRust {
    /// Child model created in `initialize`; Qt owns it.
    model: QObjectMutPtr,
    /// The installed backend, i.e. the owner of the store.
    owner: QObjectMutPtr,
    current_mirror: QString,
    probing: bool,
    finished_count: i32,
    /// Bumped by every `probe_all`. A result from an older batch is dropped, so
    /// a second click cannot interleave two batches into one row set.
    batch: Arc<AtomicU64>,
    /// Stops the running batch when this backend is detached from its store
    /// owner, i.e. when the page that owns it is being torn down. A separate
    /// flag from the application-wide shutdown, which other probes share.
    cancel: Arc<AtomicBool>,
}

impl Default for MirrorBackendRust {
    /// Only used before `initialize` runs; the model stays null until then.
    fn default() -> Self {
        Self {
            model: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            owner: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            current_mirror: QString::from(""),
            probing: false,
            finished_count: 0,
            batch: Arc::new(AtomicU64::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl cxx_qt::Initialize for qobject::MirrorBackend {
    /// Creates the model as a child of this backend, fully populated before QML
    /// can bind to it.
    fn initialize(mut self: Pin<&mut Self>) {
        let parent = unsafe {
            QObjectMutPtr::from_raw(
                <qobject::MirrorBackend as Upcast<cxx_qt::QObject>>::upcast_ptr(&*self).cast_mut(),
            )
        };
        let model = qobject::create_mirror_model(parent);
        let data = Pin::get_mut(self.as_mut().rust_mut());
        data.model = model;
    }
}

impl qobject::MirrorBackend {
    /// Probes every mirror on a worker thread.
    ///
    /// The GUI thread only marks the rows and starts the thread; results arrive
    /// through `qt_thread().queue(...)` one row at a time, so a QML timer keeps
    /// firing and the view keeps scrolling while the batch runs.
    ///
    /// The sweep stops early — leaving the remaining rows untouched — when a
    /// newer batch takes over, when the backend has been destroyed, or when the
    /// application is shutting down. The last case matters: a full sweep takes
    /// ~20 s, and a window closed mid-batch must not keep a worker and its
    /// network manager alive behind a dead event loop.
    fn probe_all(mut self: Pin<&mut Self>) {
        let model = self.model;
        if model.as_ptr().is_null() {
            return;
        }
        let batch = self.batch.fetch_add(1, Ordering::SeqCst) + 1;
        // A batch started after a re-attach must be allowed to run.
        self.cancel.store(false, Ordering::Relaxed);

        qobject::mirror_model_set_all_running(model);
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.probing = true;
            data.finished_count = 0;
        }
        self.as_mut().probing_changed();
        self.as_mut().finished_count_changed();

        let thread = self.qt_thread();
        let generation = self.batch.clone();
        let cancel = self.cancel.clone();
        let targets = row_targets();

        std::thread::spawn(move || {
            // One manager for the batch, created and destroyed on this thread.
            let handle = qobject::probe_batch_begin();
            let mut stopped = false;
            for (row, url) in targets {
                if generation.load(Ordering::SeqCst) != batch
                    || thread.is_destroyed()
                    || cancel.load(Ordering::Relaxed)
                    || qobject::probe_shutdown_requested()
                {
                    // A newer batch owns the rows, this backend was detached
                    // from its store, or the app is going away: stop asking the
                    // network.
                    stopped = true;
                    break;
                }
                let Some(outcome) = probe_url(handle, url) else {
                    // Shutdown cancelled the request in flight.
                    stopped = true;
                    break;
                };
                thread
                    .queue(move |backend| {
                        backend.apply_probe_result(
                            row,
                            batch,
                            &QString::from(outcome.state_key),
                            outcome.latency_ms,
                            &QString::from(outcome.error_key),
                        );
                    })
                    .ok();
            }
            qobject::probe_batch_end(handle);
            if stopped {
                // Clears `probing` (the button must not stay disabled) and puts
                // the unanswered rows back to `idle`; a no-op when a newer batch
                // already owns the rows.
                thread
                    .queue(move |backend| backend.finish_cancelled(batch))
                    .ok();
            }
        });
    }

    /// Probes one mirror, for a row's own "refresh" action.
    ///
    /// It never touches the batch counters: `finishedCount` describes a batch,
    /// and a single probe that ran next to one must not move it.
    fn probe_one(self: Pin<&mut Self>, url: &cxx_qt_lib::QString) {
        let url = url.to_string();
        let Some(row) = row_of(&url) else {
            // Not a listed mirror: there is no row to report into.
            return;
        };
        let model = self.model;
        if model.as_ptr().is_null() {
            return;
        }
        qobject::mirror_model_set_result(
            model,
            row,
            &QString::from(STATE_RUNNING),
            0,
            &QString::from(""),
        );

        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let handle = qobject::probe_batch_begin();
            let outcome = probe_url(handle, &url);
            qobject::probe_batch_end(handle);
            // A cancelled request reports no result: put the row back to `idle`
            // instead of leaving it `running` forever.
            let outcome = outcome.unwrap_or(ProbeOutcome {
                state_key: crate::mirror_model::STATE_IDLE,
                latency_ms: 0,
                error_key: "",
            });
            thread
                .queue(move |backend| {
                    backend.apply_single_result(
                        row,
                        &QString::from(outcome.state_key),
                        outcome.latency_ms,
                        &QString::from(outcome.error_key),
                    );
                })
                .ok();
        });
    }

    /// Points the backend at the store owner and adopts its saved mirror.
    ///
    /// Detaching (a null owner) is the teardown path of the page that owns this
    /// backend: the running sweep stops instead of holding a socket open for a
    /// view that is going away, and the last selection is kept for whoever binds
    /// next.
    fn set_owner(mut self: Pin<&mut Self>, value: QObjectMutPtr) {
        if value.as_ptr().is_null() {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.cancel.store(true, Ordering::Relaxed);
            data.owner = value;
            self.as_mut().owner_changed();
            return;
        }
        let saved = qobject::installed_mirror(value);
        let saved_text = saved.to_string();
        let current = if saved_text.is_empty() {
            // A store without a mirror means the default; write it back so the
            // next start does not have to guess again.
            qobject::installed_set_mirror(value, &QString::from(DEFAULT_MIRROR));
            QString::from(DEFAULT_MIRROR)
        } else {
            saved
        };
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.owner = value;
            data.current_mirror = current;
            data.cancel.store(false, Ordering::Relaxed);
        }
        self.as_mut().owner_changed();
        self.as_mut().current_mirror_changed();
    }

    /// Selects a mirror and persists it through the store.
    fn set_current_mirror(mut self: Pin<&mut Self>, value: QString) {
        let url = value.to_string();
        if url.is_empty() || self.current_mirror.to_string() == url {
            return;
        }
        let owner = self.owner;
        if !owner.as_ptr().is_null() {
            qobject::installed_set_mirror(owner, &value);
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.current_mirror = value;
        }
        self.as_mut().current_mirror_changed();
    }

    /// Applies one result of the current batch, then advances the counter.
    fn apply_probe_result(
        mut self: Pin<&mut Self>,
        row: i32,
        batch: u64,
        state_key: &cxx_qt_lib::QString,
        latency_ms: u32,
        error_key: &cxx_qt_lib::QString,
    ) {
        if self.batch.load(Ordering::SeqCst) != batch {
            return;
        }
        self.write_row(row, state_key, latency_ms, error_key);

        let total = i32::try_from(MIRRORS.len()).unwrap_or(i32::MAX);
        let done = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.finished_count = data.finished_count.saturating_add(1);
            let done = data.finished_count >= total;
            if done {
                data.probing = false;
            }
            done
        };
        self.as_mut().finished_count_changed();
        if done {
            self.as_mut().probing_changed();
            self.as_mut().probe_finished();
        }
    }

    /// Applies the result of a single-mirror probe; counters are untouched.
    fn apply_single_result(
        self: Pin<&mut Self>,
        row: i32,
        state_key: &cxx_qt_lib::QString,
        latency_ms: u32,
        error_key: &cxx_qt_lib::QString,
    ) {
        self.write_row(row, state_key, latency_ms, error_key);
    }

    /// Ends a batch that stopped before its last row (shutdown, teardown, or a
    /// newer batch).
    ///
    /// The generation guard keeps ownership honest: only the batch that is still
    /// current resets `probing` and returns its unanswered rows to `idle`, so a
    /// superseded batch cannot re-enable a button the new batch disabled.
    fn finish_cancelled(mut self: Pin<&mut Self>, batch: u64) {
        if self.batch.load(Ordering::SeqCst) != batch {
            return;
        }
        let model = self.model;
        if !model.as_ptr().is_null() {
            qobject::mirror_model_finish_running(model);
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            if !data.probing {
                return;
            }
            data.probing = false;
        }
        self.as_mut().probing_changed();
        self.as_mut().probe_finished();
    }

    /// Writes one row into the model, if the model exists.
    fn write_row(
        &self,
        row: i32,
        state_key: &cxx_qt_lib::QString,
        latency_ms: u32,
        error_key: &cxx_qt_lib::QString,
    ) {
        let model = self.model;
        if model.as_ptr().is_null() {
            return;
        }
        qobject::mirror_model_set_result(model, row, state_key, latency_ms, error_key);
    }
}

/// Row index of a mirror URL, `None` when the URL is not in the list.
fn row_of(url: &str) -> Option<i32> {
    MIRRORS
        .iter()
        .position(|mirror| mirror.url == url)
        .and_then(|row| i32::try_from(row).ok())
}

/// Every row with the URL it probes; row `i` is `MIRRORS[i]` by construction.
fn row_targets() -> Vec<(i32, &'static str)> {
    MIRRORS
        .iter()
        .enumerate()
        .filter_map(|(row, mirror)| Some((i32::try_from(row).ok()?, mirror.url)))
        .collect()
}

/// Probes one URL: the core rule first, then the transport, then [`measure`].
///
/// Runs on the worker thread. The returned outcome carries keys only; `None`
/// means the application began shutting down and the request was abandoned, so
/// there is nothing to report for that row.
fn probe_url(handle: usize, url: &str) -> Option<ProbeOutcome> {
    if let Err(error) = check_probe_url(url) {
        // Never requested: the row says why instead.
        return Some(ProbeOutcome::refused(error));
    }
    let start = Instant::now();
    let code = qobject::probe_head(
        handle,
        &QString::from(url),
        u32::try_from(PROBE_TIMEOUT_MS).unwrap_or(u32::MAX),
    );
    ProbeOutcome::classify(code, measure(start))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_is_ok_with_its_latency() {
        let outcome = ProbeOutcome::classify(0, 178).expect("an answer is a result");
        assert_eq!(outcome.state_key, "ok");
        assert_eq!(outcome.latency_ms, 178);
        assert_eq!(outcome.error_key, "");
    }

    #[test]
    fn the_timer_and_a_dead_host_get_distinct_keys() {
        let key = |code| ProbeOutcome::classify(code, 6500).unwrap().error_key;
        assert_eq!(key(1), "timeout");
        assert_eq!(key(2), "noResponse");
        assert_eq!(key(-1), "noResponse");
        for code in [1, 2] {
            let outcome = ProbeOutcome::classify(code, 999).unwrap();
            assert_eq!(outcome.state_key, "fail");
            assert_eq!(outcome.latency_ms, 0, "a failure has no latency");
        }
    }

    #[test]
    fn a_cancelled_request_reports_no_result() {
        // Code 3 means the shutdown flag aborted the request: there is nothing
        // to show, so the row keeps whatever it had and the batch stops.
        assert_eq!(ProbeOutcome::classify(3, 1200), None);
    }

    #[test]
    fn a_plain_http_mirror_is_refused_before_any_request() {
        let outcome = ProbeOutcome::refused(ProbeError::PlainHttp);
        assert_eq!(outcome.state_key, "fail");
        assert_eq!(outcome.error_key, "plainHttp");
        assert_eq!(outcome.latency_ms, 0);
        // The rule, not the transport, is what produces it.
        assert_eq!(
            check_probe_url("http://ftp.dk.xemacs.org/voidlinux/"),
            Err(ProbeError::PlainHttp)
        );
    }

    #[test]
    fn the_batch_covers_every_row_of_the_model() {
        let targets = row_targets();
        assert_eq!(targets.len(), 34);
        for (row, url) in &targets {
            assert_eq!(MIRRORS[usize::try_from(*row).unwrap()].url, *url);
        }
        let plain_http: Vec<&str> = targets
            .iter()
            .filter(|(_, url)| check_probe_url(url).is_err())
            .map(|(_, url)| *url)
            .collect();
        assert_eq!(
            plain_http,
            [
                "http://ftp.dk.xemacs.org/voidlinux/",
                "http://ftp.debian.ru/mirrors/voidlinux/"
            ],
            "the two plain-HTTP entries stay in the list and are never requested"
        );
    }

    #[test]
    fn a_url_maps_back_to_its_row() {
        assert_eq!(row_of(DEFAULT_MIRROR), Some(0));
        assert_eq!(row_of("http://ftp.debian.ru/mirrors/voidlinux/"), Some(22));
        assert_eq!(row_of("https://not-a-mirror.example/"), None);
    }
}
