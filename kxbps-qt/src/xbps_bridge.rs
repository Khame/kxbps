//! Bridge to the C++ `QProcess` runner in `src/xbps_shim.cpp`.
//!
//! The shim is one synchronous call: JSON argv in, JSON reply out, with a hard
//! timeout. It is declared here and nowhere else, and [`production_invoke`]
//! wraps it in the closure the read layer takes, so every test can install a
//! fake instead and the suite never starts xbps.
//!
//! Nothing in this file decides what the output means, touches a QObject or
//! waits on a process from the GUI thread; the worker threads in
//! `installed_backend.rs` own those rules.
//!
//! ## The two hooks of the streaming call
//!
//! cxx has no mapping for `std::function`, and a `type X = super::X` alias does
//! not create one either: the generated C++ would name a type that does not
//! exist. A `fn(..)` item in an `extern "C++"` block does work, but on the Rust
//! side it is a bare function pointer: it cannot capture.
//!
//! So the hooks are two free functions reading a scoped thread-local slot, and
//! [`run_streaming`] puts the caller's closures there for the duration of the
//! shim call. The shim is synchronous on the calling thread, which is exactly
//! the lifetime a scoped thread-local gives: the borrows are checked by the
//! compiler, and the slot is restored on every exit path, panic included.

use std::cell::RefCell;

use cxx_qt_lib::QString;

use crate::xbps_snapshot::SharedInvoke;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "C++" {
        include!("xbps_shim.h");

        // `QString kxbps_run_xbps(const QString &argvJson, int timeoutMs)`;
        // the reply shape is documented in `xbps_shim.h`.
        #[cxx_name = "kxbps_run_xbps"]
        fn run_xbps(argv_json: &QString, timeout_ms: i32) -> QString;

        // The streaming variant of the same call: `line` and `spawned` are
        // called while the process runs; see the module docs for why they are
        // plain function pointers over a scoped thread-local.
        #[cxx_name = "kxbps_run_xbps_streaming"]
        fn run_xbps_streaming(
            argv_json: &QString,
            timeout_ms: i32,
            cancel_json: &QString,
            line: fn(&QString),
            spawned: fn(),
        ) -> QString;
    }
}

/// The hooks [`run_streaming`] installs for the shim call.
struct Hooks {
    line: Box<dyn Fn(&str) + Send>,
    spawned: Box<dyn Fn() + Send>,
}

thread_local! {
    /// The hooks of the streaming call running on this thread, if any. They are
    /// `'static` and owned, so the slot may hold them safely; the guard in
    /// [`run_streaming`] clears it before that function returns.
    static HOOKS: RefCell<Option<Hooks>> = const { RefCell::new(None) };
}

/// The function pointer handed to the shim: runs the installed line hook.
///
/// An empty slot means the shim was called without [`HOOKS`]: nothing is
/// listening, so the line is dropped rather than panicking across the FFI.
fn line_trampoline(text: &QString) {
    let line = text.to_string();
    HOOKS.with_borrow(|hooks| {
        if let Some(hooks) = hooks.as_ref() {
            (hooks.line)(&line);
        }
    });
}

/// The function pointer handed to the shim for the spawn notification.
fn spawned_trampoline() {
    HOOKS.with_borrow(|hooks| {
        if let Some(hooks) = hooks.as_ref() {
            (hooks.spawned)();
        }
    });
}

/// Clears the slot on drop, so no exit path leaves a stale hook behind.
struct HooksGuard;

impl Drop for HooksGuard {
    fn drop(&mut self) {
        HOOKS.with_borrow_mut(Option::take);
    }
}

/// The shim-backed transport the application installs.
///
/// A timeout above `i32::MAX` milliseconds is not a timeout anyone waits for;
/// it is clamped so the conversion cannot wrap into a negative `int`.
pub fn production_invoke() -> SharedInvoke {
    std::sync::Arc::new(|request: &str, timeout_ms: u32| {
        let timeout = i32::try_from(timeout_ms).unwrap_or(i32::MAX);
        qobject::run_xbps(&QString::from(request), timeout).to_string()
    })
}

/// The transport of one privileged transaction.
///
/// `argv` is the complete `pkexec …` list; `may_start` is asked before the
/// process is spawned and a `false` answer aborts without running anything;
/// `spawned` reports that the process exists; `line` receives every whole
/// output line while it runs. The answer is the raw exit code, `-1` when the
/// program could not be started at all.
///
/// The three callbacks are taken by value and must be `'static` because the
/// shim's hooks are bare function pointers: the worker shares its state through
/// an `Arc`, the closures own their clones, and the slot below holds them for
/// the duration of the call only.
pub fn run_streaming(
    argv: &[String],
    may_start: impl Fn() -> bool + 'static,
    spawned: impl Fn() + Send + 'static,
    line: impl Fn(&str) + Send + 'static,
) -> i32 {
    let Ok(request) = crate::xbps_runner::argv_json(argv) else {
        return -1;
    };
    let cancel_json = if may_start() { "false" } else { "true" };
    let hooks = Hooks {
        line: Box::new(line),
        spawned: Box::new(spawned),
    };
    HOOKS.with_borrow_mut(|slot| *slot = Some(hooks));
    let _guard = HooksGuard;
    let reply = qobject::run_xbps_streaming(
        &QString::from(request.as_str()),
        crate::xbps_runner::TRANSACTION_TIMEOUT_MS as i32,
        &QString::from(cancel_json),
        line_trampoline,
        spawned_trampoline,
    );
    crate::xbps_runner::decode_reply(&reply.to_string())
        .map(|output| output.exit_code)
        .unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::xbps_runner::decode_reply;

    /// Locks a mutex whose owner never panics mid-assertion.
    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    #[test]
    fn the_shim_answers_a_well_formed_reply_for_a_missing_program() {
        // The one process this test starts cannot exist and never reaches a
        // shell: the reply is the documented object with `exitCode: -1`.
        let reply = production_invoke()(r#"["kxbps-no-such-program"]"#, 5_000);
        let output =
            decode_reply(&reply).expect("the shim always replies with the documented object");
        assert_eq!(output.exit_code, -1);
        assert!(!output.timed_out);
        assert!(!output.success());
    }

    #[test]
    fn a_real_streaming_run_hands_every_line_to_the_sink() {
        // `/bin/echo` prints one line and exits 0; no root, no xbps involved.
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let did_spawn = std::sync::Arc::new(AtomicBool::new(false));
        let sink = std::sync::Arc::clone(&seen);
        let spawn = std::sync::Arc::clone(&did_spawn);
        let code = run_streaming(
            &[
                String::from("/bin/echo"),
                String::from("kxbps streaming works"),
            ],
            || true,
            move || spawn.store(true, Ordering::SeqCst),
            move |line: &str| lock(&sink).push(line.to_owned()),
        );
        assert_eq!(code, 0);
        assert!(
            did_spawn.load(Ordering::SeqCst),
            "the spawn notification fires once the process exists"
        );
        let seen = lock(&seen);
        assert!(
            seen.iter().any(|line| line == "kxbps streaming works"),
            "the line reached the sink: {seen:?}"
        );
    }

    #[test]
    fn a_cancelled_run_kills_the_process_before_it_prints() {
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&seen);
        let code = run_streaming(
            &[String::from("/bin/echo"), String::from("must not appear")],
            || false,
            || {},
            move |line: &str| lock(&sink).push(line.to_owned()),
        );
        assert_eq!(code, -1, "a process killed before it ran reports -1");
        let seen = lock(&seen);
        assert!(seen.is_empty(), "nothing ran: {seen:?}");
    }

    #[test]
    fn stderr_lines_stream_too_and_the_exit_code_is_passed_through() {
        // A child that writes to stderr and exits 19: the shim must report 19,
        // not translate it, and both channels must reach the sink.
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&seen);
        let code = run_streaming(
            &[
                String::from("/bin/sh"),
                String::from("-c"),
                String::from("echo out; echo err 1>&2; exit 19"),
            ],
            || true,
            || {},
            move |line: &str| lock(&sink).push(line.to_owned()),
        );
        assert_eq!(code, 19, "an xbps code is never translated");
        let seen = lock(&seen);
        assert!(seen.iter().any(|line| line == "out"), "{seen:?}");
        assert!(seen.iter().any(|line| line == "err"), "{seen:?}");
    }

    #[test]
    fn the_hooks_are_unset_after_the_call() {
        let _ = run_streaming(
            &[String::from("/bin/echo"), String::from("one")],
            || true,
            || {},
            |_: &str| {},
        );
        assert!(
            HOOKS.with_borrow(Option::is_none),
            "the slot must not outlive the call"
        );
        // A trampoline with no slot is a no-op rather than a panic: the shim
        // may be called from the plain read path, which installs no hooks.
        line_trampoline(&QString::from("ignored"));
        spawned_trampoline();
    }
}
