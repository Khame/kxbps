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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xbps_runner::decode_reply;

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
}
