//! When to ask the repositories for updates, and how to word the answer.
//!
//! `xbps-install -un` reads the local index only, so the list it prints is as
//! old as the last repository sync somebody else ran. The fresh read is
//! `xbps-install -M -un`, which fetches the indexes into memory and writes
//! nothing. This module holds the decisions that surround it: whether a check is
//! due, whether a local read may replace a list the network just produced, and
//! which sentence an outcome becomes.
//!
//! Everything here is pure. The Qt layer owns the clock, the process and the
//! network; it passes them in and translates what comes back.

use crate::xbps::UpdateEntry;

/// How long a successful check counts as fresh enough to skip another one.
pub const AUTO_GAP_MS: i64 = 5 * 60 * 1000;

/// Where the pending-update list came from.
///
/// A local read (`xbps-install -un`) describes whatever the last sync left
/// behind; a remote one (`-M -un`) describes the repositories as of `at`. Only
/// the caller that remembers this can tell a stale answer from a fresh one,
/// which is why it is memory-only and never persisted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdatesOrigin {
    /// The read touched the local index only.
    #[default]
    Local,
    /// The check fetched the indexes; `at` is when it succeeded, epoch ms.
    Remote { at: i64 },
}

/// Whether a check may start at `now`.
///
/// A check that never succeeded (`last == 0`) is always due, and so is one whose
/// recorded time lies in the future: a clock that went backwards must not silence
/// the check for as long as the skew lasts.
pub fn is_due(last: i64, now: i64, gap_ms: i64) -> bool {
    if last <= 0 || last > now {
        return true;
    }
    now - last >= gap_ms
}

/// Whether a list read from the local index may replace the current one.
///
/// A remote list younger than `gap_ms` wins: the local read cannot see anything
/// the check did not already see, and replacing a fresh list with an older one
/// would make the repositories look older than they are. Anything else (no
/// remote list, a stale one, a remote list stamped in the future) yields to the
/// local read.
pub fn local_replaces(origin: UpdatesOrigin, now: i64, gap_ms: i64) -> bool {
    match origin {
        UpdatesOrigin::Local => true,
        UpdatesOrigin::Remote { at } => is_due(at, now, gap_ms),
    }
}

/// What a check produced, as the Qt layer reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    /// A check is already running; nothing new was started.
    Busy,
    /// Not due yet, and the caller did not insist.
    NotDue,
    /// The network reported no connectivity, so no process ran.
    Offline,
    /// `xbps-install -M -un` answered; the list is fresh.
    UpToDate,
    /// `xbps-install -M -un` answered with pending updates.
    Updates { count: usize },
    /// At least one repository could not be read; `failed` of `total` failed.
    /// `detail` is the first line xbps printed, untranslated.
    RepositoriesFailed {
        failed: usize,
        total: usize,
        detail: String,
    },
    /// The process itself failed: missing, timed out, or unparsable.
    ReadFailed { key: &'static str },
}

/// The translated sentence a check becomes: the i18n key and its `%1…` arguments.
///
/// The age of the last successful check is part of the sentence for every
/// failure, so the user always learns how old the list on screen is. `age` is
/// already worded by the caller (`age_args`), because only it can pluralise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckStatus {
    /// The key QML maps; empty means no text.
    pub key: &'static str,
    /// Arguments in `%1…` order.
    pub args: Vec<String>,
}

impl CheckStatus {
    /// No sentence: the strip keeps whatever it showed.
    pub fn none() -> Self {
        Self {
            key: "",
            args: Vec::new(),
        }
    }
}

/// Whether a reachability reading means "no network".
///
/// The readings are `QNetworkInformation::Reachability` numbers: 0 Unknown,
/// 1 Disconnected, 2 Local, 3 Site, 4 Online, and `-1` when no backend loaded.
/// Only `Disconnected` is offline: Unknown, no backend, and a network that only
/// reaches an intranet all still deserve a try, because xbps may use a local
/// mirror and a wrong "offline" would block a check that could have worked.
pub fn is_offline(reachability: i32) -> bool {
    reachability == 1
}

/// Whether a local read could not refresh the list, as a key QML words.
pub fn offline_key() -> &'static str {
    "checkOffline"
}

/// The status sentence for one outcome.
///
/// `age` is the wording of the last successful check (`"2 hours"`, or the
/// "not checked yet" sentence); it is never built here, because this module has
/// no plural rules and no translations.
pub fn check_status(outcome: &CheckOutcome, age: &str) -> CheckStatus {
    let arg = |value: &str| vec![value.to_owned()];
    match outcome {
        CheckOutcome::Busy | CheckOutcome::NotDue => CheckStatus::none(),
        CheckOutcome::Offline => CheckStatus {
            key: "checkOffline",
            args: arg(age),
        },
        CheckOutcome::UpToDate => CheckStatus {
            key: "checkUpToDate",
            args: arg(age),
        },
        CheckOutcome::Updates { count } => CheckStatus {
            key: "checkUpdatesFound",
            args: vec![count.to_string(), age.to_owned()],
        },
        CheckOutcome::RepositoriesFailed {
            failed,
            total,
            detail,
        } => CheckStatus {
            key: "checkRepositoriesFailed",
            args: vec![
                failed.to_string(),
                total.to_string(),
                detail.clone(),
                age.to_owned(),
            ],
        },
        CheckOutcome::ReadFailed { key: _ } => CheckStatus {
            key: "checkReadFailed",
            args: arg(age),
        },
    }
}

/// Whether a successful check with `updates` may claim the system is current.
///
/// Only an empty list from a check that actually reached the repositories says
/// that; a list of held packages is still a list of pending updates.
pub fn claims_up_to_date(updates: &[UpdateEntry]) -> bool {
    updates.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xbps::XbpsAction;

    const MINUTE: i64 = 60_000;

    #[test]
    fn a_check_that_never_succeeded_is_due() {
        assert!(is_due(0, 1_700_000_000_000, AUTO_GAP_MS));
    }

    #[test]
    fn a_check_inside_the_gap_is_not_due() {
        let now = 1_700_000_000_000;
        assert!(!is_due(now - MINUTE, now, AUTO_GAP_MS));
        assert!(is_due(now - AUTO_GAP_MS, now, AUTO_GAP_MS));
    }

    #[test]
    fn a_clock_that_went_backwards_makes_the_check_due() {
        let now = 1_700_000_000_000;
        assert!(is_due(now + MINUTE, now, AUTO_GAP_MS));
    }

    #[test]
    fn a_young_remote_list_is_not_replaced_by_a_local_read() {
        let now = 1_700_000_000_000;
        let origin = UpdatesOrigin::Remote { at: now - MINUTE };
        assert!(!local_replaces(origin, now, AUTO_GAP_MS));
    }

    #[test]
    fn a_stale_remote_list_yields_to_the_local_read() {
        let now = 1_700_000_000_000;
        let origin = UpdatesOrigin::Remote {
            at: now - AUTO_GAP_MS,
        };
        assert!(local_replaces(origin, now, AUTO_GAP_MS));
        assert!(local_replaces(UpdatesOrigin::Local, now, AUTO_GAP_MS));
    }

    #[test]
    fn a_remote_list_stamped_in_the_future_yields_to_the_local_read() {
        let now = 1_700_000_000_000;
        let origin = UpdatesOrigin::Remote { at: now + MINUTE };
        assert!(local_replaces(origin, now, AUTO_GAP_MS));
    }

    #[test]
    fn a_failed_check_says_how_old_the_list_is() {
        let status = check_status(&CheckOutcome::Offline, "2 hours");
        assert_eq!(status.key, "checkOffline");
        assert_eq!(status.args, vec![String::from("2 hours")]);
    }

    #[test]
    fn a_partly_failed_check_names_the_count_and_the_first_error_line() {
        let outcome = CheckOutcome::RepositoriesFailed {
            failed: 1,
            total: 2,
            detail: String::from("ERROR: failed to open repository: x: Input/output error"),
        };
        let status = check_status(&outcome, "5 minutes");
        assert_eq!(status.key, "checkRepositoriesFailed");
        assert_eq!(
            status.args,
            vec![
                String::from("1"),
                String::from("2"),
                String::from("ERROR: failed to open repository: x: Input/output error"),
                String::from("5 minutes"),
            ]
        );
    }

    #[test]
    fn a_quiet_outcome_carries_no_sentence() {
        assert_eq!(check_status(&CheckOutcome::Busy, "1 minute").key, "");
        assert_eq!(check_status(&CheckOutcome::NotDue, "1 minute").key, "");
    }

    #[test]
    fn only_an_empty_list_from_a_finished_check_is_up_to_date() {
        assert!(claims_up_to_date(&[]));
        assert!(!claims_up_to_date(&[UpdateEntry {
            name: String::from("vim"),
            version: String::from("9.2.1031_2"),
            pkgver: String::from("vim-9.2.1031_2"),
            action: XbpsAction::Update,
            arch: String::from("x86_64"),
            repository: String::from("https://repo-default.voidlinux.org/current"),
            installed_size: 1,
            download_size: 1,
        }]));
    }

    #[test]
    fn only_disconnected_counts_as_offline() {
        assert!(is_offline(1));
        assert!(!is_offline(0), "unknown still tries");
        assert!(!is_offline(-1), "no backend still tries");
        assert!(!is_offline(2), "local-only network still tries");
        assert!(!is_offline(4));
    }

    #[test]
    fn the_offline_key_is_the_one_qml_words() {
        assert_eq!(offline_key(), "checkOffline");
    }
}
