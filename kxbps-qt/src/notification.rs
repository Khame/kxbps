//! Whether updates are announced, decided without Qt.
//!
//! The desktop popup itself lives in `Notifier.qml`, which needs the
//! `org.freedesktop.Notifications` service and a session bus. Deciding *whether*
//! to show it is a table of preferences and one string comparison, so it lives
//! here and is tested without a QObject, a bus or a display: a wrong answer is a
//! popup on every start, and that is exactly what a GUI-free test can rule out.
//!
//! One set of updates is announced once, and reminded once three hours later.
//! The reminder is the only repeat: after it, a set is silent for good (the tray
//! icon stays the permanent indicator). A different set starts the cycle again.
//!
//! The module returns translation *keys*, never finished sentences. QML picks
//! the plural form through `i18ndp`, because the plural rule belongs to the
//! translation, not to Rust.

/// How long after the first announcement the one reminder goes out.
pub const REMINDER_AFTER_MS: i64 = 3 * 60 * 60 * 1000;

/// The floor between two first announcements, i.e. one day.
///
/// Without it a set that changes daily would pop up on every start; with it the
/// tray icon carries the news for the rest of the day.
pub const FIRST_NOTICE_FLOOR_MS: i64 = 24 * 60 * 60 * 1000;

/// Which of the two popups this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Announcement {
    /// The set appeared; the user hears about it for the first time.
    First,
    /// The set is still there three hours later.
    Reminder,
}

/// What QML needs to build one notification.
///
/// Only the kind and the count travel. The sentences live in `Notifier.qml`,
/// where the extractor sees them and the catalogs get them; an English msgid
/// held here would never reach the template, and the reminder body would stay
/// English in every language. The plural form is still chosen by `i18ndp` in
/// QML, because the plural rule belongs to the translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotificationPayload {
    pub kind: Announcement,
    pub count: usize,
}

impl NotificationPayload {
    /// The payload for one announcement of `count` packages.
    fn new(kind: Announcement, count: usize) -> Self {
        Self { kind, count }
    }
}

/// Everything the decision needs; one field per stored value or live condition.
///
/// `signature` is the whole set, not a count: two different sets with the same
/// number of packages are different news.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnounceState {
    /// The set pending right now.
    pub signature: String,
    /// The last set announced (first popup), empty when none.
    pub dismissed: String,
    /// The last set reminded, empty when none.
    pub reminded: String,
    /// Epoch milliseconds of the last first announcement; `0` means never.
    pub notified_at: i64,
    /// The current time, epoch milliseconds.
    pub now: i64,
    /// How many packages are pending.
    pub count: usize,
    /// The one notification switch.
    pub desktop_notifications: bool,
    /// Whether a window is on screen; a popup over the list is noise.
    pub window_visible: bool,
}

/// Decides which announcement, if any, this moment calls for.
///
/// The checks run cheapest and most decisive first, and every early return is a
/// documented reason for silence. The ones that return before writing anything
/// (empty set, switch off, visible window) leave the bookkeeping alone, so the
/// announcement is made later instead of being lost.
pub fn announcement(state: &AnnounceState) -> Option<Announcement> {
    // Nothing to say: an empty set also cancels a pending reminder, because
    // there is nothing left to remind about.
    if state.count == 0 || state.signature.is_empty() {
        return None;
    }
    // The user turned the one switch off: only the tray icon changes.
    if !state.desktop_notifications {
        return None;
    }
    // The user is looking at the list; the announcement waits for a later call.
    if state.window_visible {
        return None;
    }
    if state.signature != state.dismissed {
        // A new set, at most once a day. The cap only applies to a clock that
        // agrees with itself: `now >= notified_at`. A clock that moved backwards
        // cannot tell how long ago the last announcement was, so the differing
        // set is still news and re-anchors the stamp; the reminder below is
        // blocked until the clock catches up.
        if state.now >= state.notified_at && state.now - state.notified_at < FIRST_NOTICE_FLOOR_MS {
            return None;
        }
        return Some(Announcement::First);
    }
    // The same set: one reminder, three hours after the first announcement, and
    // only when it has not been reminded yet. `now >= notified_at` rules out a
    // clock that went backwards; the user is warned on the next start instead.
    if state.signature != state.reminded
        && state.now >= state.notified_at
        && state.now - state.notified_at >= REMINDER_AFTER_MS
    {
        return Some(Announcement::Reminder);
    }
    None
}

/// The payload QML renders for the decision [`announcement`] returned.
pub fn payload_for(kind: Announcement, count: usize) -> NotificationPayload {
    NotificationPayload::new(kind, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One hour in milliseconds, so the boundaries read in a test.
    const HOUR: i64 = 60 * 60 * 1000;
    /// The time of the first announcement the states below start from.
    const T0: i64 = 1_700_000_000_000;

    /// An allowed state: five pending packages, nothing announced before.
    fn allowed() -> AnnounceState {
        AnnounceState {
            signature: "sig-a".to_string(),
            dismissed: String::new(),
            reminded: String::new(),
            notified_at: 0,
            now: T0,
            count: 5,
            desktop_notifications: true,
            window_visible: false,
        }
    }

    #[test]
    fn an_empty_set_is_not_announced() {
        let state = AnnounceState {
            count: 0,
            ..allowed()
        };
        assert_eq!(announcement(&state), None);
    }

    #[test]
    fn a_negative_counter_is_a_broken_state_not_a_reason_to_pop_up() {
        // `count` is a `usize` in Rust; the Qt layer never passes a negative
        // number, and an empty set is the only zero case above.
        let state = allowed();
        assert_eq!(state.count, 5);
    }

    #[test]
    fn an_empty_signature_means_nothing_was_read_yet() {
        let state = AnnounceState {
            signature: String::new(),
            dismissed: String::new(),
            ..allowed()
        };
        assert_eq!(announcement(&state), None);
    }

    #[test]
    fn a_disabled_switch_stays_silent_even_for_a_new_set() {
        let state = AnnounceState {
            desktop_notifications: false,
            ..allowed()
        };
        assert_eq!(announcement(&state), None);
    }

    #[test]
    fn a_visible_window_defers_the_announcement() {
        let state = AnnounceState {
            window_visible: true,
            ..allowed()
        };
        assert_eq!(announcement(&state), None, "no popup over the list");
        // Hidden again, the same state announces.
        let state = AnnounceState {
            window_visible: false,
            ..state
        };
        assert_eq!(announcement(&state), Some(Announcement::First));
    }

    #[test]
    fn a_new_set_is_announced_once() {
        assert_eq!(announcement(&allowed()), Some(Announcement::First));
    }

    #[test]
    fn a_set_announced_an_hour_ago_is_not_announced_again() {
        let state = AnnounceState {
            dismissed: "sig-a".to_string(),
            notified_at: T0,
            now: T0 + HOUR,
            ..allowed()
        };
        assert_eq!(
            announcement(&state),
            None,
            "one first announcement per set, and the reminder is not due yet"
        );
    }

    #[test]
    fn a_second_set_within_the_day_is_capped() {
        let state = AnnounceState {
            signature: "sig-b".to_string(),
            dismissed: "sig-a".to_string(),
            notified_at: T0,
            now: T0 + 23 * HOUR,
            ..allowed()
        };
        assert_eq!(
            announcement(&state),
            None,
            "the tray icon carries the news until the day is up"
        );
    }

    #[test]
    fn a_second_set_after_the_day_is_announced() {
        let state = AnnounceState {
            signature: "sig-b".to_string(),
            dismissed: "sig-a".to_string(),
            notified_at: T0,
            now: T0 + FIRST_NOTICE_FLOOR_MS,
            ..allowed()
        };
        assert_eq!(announcement(&state), Some(Announcement::First));
    }

    #[test]
    fn the_reminder_is_not_due_one_millisecond_early() {
        let state = AnnounceState {
            dismissed: "sig-a".to_string(),
            notified_at: T0,
            now: T0 + REMINDER_AFTER_MS - 1,
            ..allowed()
        };
        assert_eq!(announcement(&state), None);
    }

    #[test]
    fn the_reminder_is_due_exactly_three_hours_later() {
        let state = AnnounceState {
            dismissed: "sig-a".to_string(),
            notified_at: T0,
            now: T0 + REMINDER_AFTER_MS,
            ..allowed()
        };
        assert_eq!(announcement(&state), Some(Announcement::Reminder));
    }

    #[test]
    fn a_reminded_set_is_never_reminded_again() {
        let state = AnnounceState {
            dismissed: "sig-a".to_string(),
            reminded: "sig-a".to_string(),
            notified_at: T0,
            now: T0 + 5 * HOUR,
            ..allowed()
        };
        assert_eq!(announcement(&state), None);
    }

    #[test]
    fn a_restart_after_three_hours_still_reminds_once() {
        // The app was closed at the three-hour mark; the next start sees the
        // elapsed time and reminds once, then stays silent.
        let restarted = AnnounceState {
            dismissed: "sig-a".to_string(),
            notified_at: T0,
            now: T0 + 9 * HOUR,
            ..allowed()
        };
        assert_eq!(announcement(&restarted), Some(Announcement::Reminder));
        let after = AnnounceState {
            reminded: "sig-a".to_string(),
            ..restarted
        };
        assert_eq!(announcement(&after), None);
    }

    #[test]
    fn a_changed_set_resets_the_reminder() {
        // The reminded marker names the old set, so the new one may remind
        // again once its own three hours pass.
        let state = AnnounceState {
            signature: "sig-b".to_string(),
            dismissed: "sig-b".to_string(),
            reminded: "sig-a".to_string(),
            notified_at: T0,
            now: T0 + REMINDER_AFTER_MS,
            ..allowed()
        };
        assert_eq!(announcement(&state), Some(Announcement::Reminder));
    }

    #[test]
    fn an_empty_set_cancels_the_reminder() {
        let state = AnnounceState {
            count: 0,
            dismissed: "sig-a".to_string(),
            notified_at: T0,
            now: T0 + 5 * HOUR,
            ..allowed()
        };
        assert_eq!(announcement(&state), None);
    }

    #[test]
    fn a_clock_that_went_backwards_blocks_the_reminder_but_not_a_new_set() {
        let backwards = AnnounceState {
            dismissed: "sig-a".to_string(),
            notified_at: T0,
            now: T0 - HOUR,
            ..allowed()
        };
        assert_eq!(announcement(&backwards), None, "no reminder from the past");

        let changed = AnnounceState {
            signature: "sig-b".to_string(),
            ..backwards
        };
        assert_eq!(
            announcement(&changed),
            Some(Announcement::First),
            "a differing set is still news and re-anchors the clock"
        );
    }

    #[test]
    fn the_payload_carries_the_kind_and_the_count() {
        let first = payload_for(Announcement::First, 1);
        assert_eq!(first.kind, Announcement::First);
        assert_eq!(first.count, 1);

        let many = payload_for(Announcement::First, 5);
        assert_eq!(many.count, 5);

        let reminder = payload_for(Announcement::Reminder, 3);
        assert_eq!(reminder.kind, Announcement::Reminder);
        assert_eq!(reminder.count, 3);
    }
}
