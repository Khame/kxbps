//! Rules for mirror latency probing.
//!
//! The web app's `probe.ts` does two things: it refuses a plain-HTTP URL before
//! touching the network, and it measures how long a `HEAD` request takes. Only
//! the first is a rule; the request itself needs a browser or Qt Network, so it
//! lives in `kxbps-qt` and this module stays free of I/O, threads and sockets
//! (`reqwest`/`tokio` are banned by the project constraints).
//!
//! Nothing here returns user-facing text: [`ProbeError`] is an enum and
//! [`ProbeError::key`] hands out a stable translation key, so QML can run
//! `i18nd("khaxbps", …)` over it.

use std::time::{Duration, Instant};

/// Time a single probe may take before it counts as [`ProbeError::Timeout`].
///
/// `probe.ts` aborts after `6500` ms; the Qt transport passes this to its timer.
pub const PROBE_TIMEOUT_MS: u64 = 6500;

/// [`PROBE_TIMEOUT_MS`] as a [`Duration`], for timers that want one.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(PROBE_TIMEOUT_MS);

/// Why a probe could not produce a latency.
///
/// Deliberately carries no message: the Qt layer stores [`Self::key`] in
/// `ProbeState::Fail` and QML turns that key into a translated sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProbeError {
    /// The URL is served over plain HTTP, which a browser blocks on an HTTPS
    /// page; refused without sending a request.
    PlainHttp,
    /// No answer within [`PROBE_TIMEOUT_MS`].
    Timeout,
    /// The request failed for any other reason (DNS, TLS, connection refused).
    NoResponse,
}

impl ProbeError {
    /// Stable translation key for QML; never shown to the user as is.
    pub const fn key(self) -> &'static str {
        match self {
            Self::PlainHttp => "plainHttp",
            Self::Timeout => "timeout",
            Self::NoResponse => "noResponse",
        }
    }
}

/// Checks a mirror URL against the probe rules before any request is made.
///
/// Mirrors the web app's `url.startsWith("http://")` guard: an `http://` URL is
/// rejected with [`ProbeError::PlainHttp`], everything else is accepted and left
/// for the transport to try.
///
/// # Examples
///
/// ```
/// use kxbps_core::probe::{ProbeError, check_probe_url};
///
/// assert_eq!(check_probe_url("https://repo-default.voidlinux.org/"), Ok(()));
/// assert_eq!(check_probe_url("http://example.org/"), Err(ProbeError::PlainHttp));
/// ```
pub fn check_probe_url(url: &str) -> Result<(), ProbeError> {
    if url.starts_with("http://") {
        Err(ProbeError::PlainHttp)
    } else {
        Ok(())
    }
}

/// Milliseconds elapsed since `start`, rounded down and floored at 1.
///
/// `probe.ts` reports `Math.max(1, Math.round(…) - started)`: a mirror that
/// answered within the timer's resolution shows `1 ms`, never `0`, so the UI
/// never has to treat "0 ms" as a special case. Saturates instead of wrapping
/// for a clock that jumped absurdly far.
pub fn measure(start: Instant) -> u32 {
    let ms = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
    ms.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_plain_http() {
        assert_eq!(
            check_probe_url("http://mirror.example/"),
            Err(ProbeError::PlainHttp)
        );
    }

    #[test]
    fn accepts_https() {
        assert_eq!(check_probe_url("https://mirror.example/"), Ok(()));
    }

    #[test]
    fn latency_is_at_least_one_ms() {
        assert_eq!(measure(Instant::now()), 1);
    }

    #[test]
    fn latency_rounds_down_to_whole_milliseconds() {
        let start = Instant::now() - Duration::from_millis(250);
        // The call itself costs microseconds, so allow a millisecond of slack
        // instead of pinning 250 exactly.
        let ms = measure(start);
        assert!((250..=260).contains(&ms), "expected ~250 ms, got {ms}");
    }

    #[test]
    fn timeout_is_6500_ms() {
        assert_eq!(PROBE_TIMEOUT_MS, 6500);
        assert_eq!(PROBE_TIMEOUT, Duration::from_millis(6500));
    }

    #[test]
    fn error_keys_are_stable_and_distinct() {
        let keys = [
            ProbeError::PlainHttp.key(),
            ProbeError::Timeout.key(),
            ProbeError::NoResponse.key(),
        ];
        assert_eq!(keys, ["plainHttp", "timeout", "noResponse"]);
        let mut sorted: Vec<&str> = keys.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len(), "keys must be unique");
    }

    #[test]
    fn non_http_schemes_pass_the_rule() {
        // The rule is the web app's plain-HTTP guard only; the transport decides
        // whether it can speak a scheme at all.
        assert_eq!(check_probe_url("ftp://mirror.example/"), Ok(()));
        assert_eq!(check_probe_url("mirror.example"), Ok(()));
    }
}
