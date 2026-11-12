//! Whether updates are announced, decided without Qt.
//!
//! The desktop popup itself lives in `Notifier.qml`, which needs the
//! `org.kde.notification` module and a session bus. Deciding *whether* to show
//! it is a table of preferences and one string comparison, so it lives here and
//! is tested without a QObject, a bus or a display: a wrong answer is a popup
//! on every start, and that is exactly what a GUI-free test can rule out.
//!
//! The module returns translation *keys*, never finished sentences. QML picks
//! the plural form through `i18ndp`, because the plural rule belongs to the
//! translation, not to Rust.

/// Wire id of the announcement; KNotification's event id, not translated.
pub const UPDATES_EVENT: &str = "updates-available";

/// What QML needs to build one notification.
///
/// `title_key` and `body_key` are English msgids for `i18nd`/`i18ndp`;
/// `body_args` carries the count as its only element. Keys are `&'static str`
/// because they are compile-time msgids, while the args are owned so the QML
/// side can take them by value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationPayload {
    pub event_id: &'static str,
    pub title_key: &'static str,
    pub body_key: &'static str,
    pub body_args: Vec<String>,
}

/// The title of the update announcement, in both plural forms.
pub const UPDATES_TITLE: &str = "Updates available";
/// The body, singular; QML's `i18ndp` chooses between it and [`UPDATES_BODY_MANY`].
pub const UPDATES_BODY_ONE: &str = "%1 package can be updated.";
/// The body, plural.
pub const UPDATES_BODY_MANY: &str = "%1 packages can be updated.";

/// Decides whether this start should announce the pending updates.
///
/// The checks run cheapest and most decisive first, and every early return is a
/// documented reason not to notify:
///
/// 1. `count <= 0` — nothing to say, and the status strip already shows zero.
/// 2. `!notify_on_start` — the user turned the announcement off.
/// 3. `!desktop` — the user wants the count in the window only.
/// 4. an empty `signature` — nothing has been read from xbps yet, so there is no
///    set to compare against and a popup would be a guess.
/// 5. `signature == dismissed` — this exact set was already delivered. A
///    notification that was never looked at is still delivered, so the next
///    start does not repeat it.
///
/// Anything else is one notification for the set `signature` describes.
pub fn updates_payload(
    notify_on_start: bool,
    desktop: bool,
    count: i32,
    signature: &str,
    dismissed: &str,
) -> Option<NotificationPayload> {
    if count <= 0 || !notify_on_start || !desktop || signature.is_empty() || signature == dismissed
    {
        return None;
    }
    Some(NotificationPayload {
        event_id: UPDATES_EVENT,
        title_key: UPDATES_TITLE,
        body_key: if count == 1 {
            UPDATES_BODY_ONE
        } else {
            UPDATES_BODY_MANY
        },
        body_args: vec![count.to_string()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The full "tell the user" case: two prefs on, five updates, nothing
    /// dismissed yet.
    const ALLOWED: (bool, bool, i32, &str, &str) = (true, true, 5, "sig-a", "");

    #[test]
    fn nothing_is_announced_without_updates() {
        let (notify_on_start, desktop, _, signature, dismissed) = ALLOWED;
        assert_eq!(
            updates_payload(notify_on_start, desktop, 0, signature, dismissed),
            None
        );
        // A negative count is a broken counter, not a reason to pop up.
        assert_eq!(
            updates_payload(notify_on_start, desktop, -1, signature, dismissed),
            None
        );
    }

    #[test]
    fn one_update_is_announced_with_the_count_as_its_only_argument() {
        let (notify_on_start, desktop, _, signature, dismissed) = ALLOWED;
        let payload =
            updates_payload(notify_on_start, desktop, 1, signature, dismissed).expect("payload");
        assert_eq!(payload.body_args, ["1"]);
        assert_eq!(payload.body_key, UPDATES_BODY_ONE);
    }

    #[test]
    fn five_updates_carry_five_and_the_plural_msgid() {
        let (notify_on_start, desktop, count, signature, dismissed) = ALLOWED;
        let payload = updates_payload(notify_on_start, desktop, count, signature, dismissed)
            .expect("payload");
        assert_eq!(payload.body_args, ["5"]);
        assert_eq!(payload.body_key, UPDATES_BODY_MANY);
    }

    #[test]
    fn the_announcement_uses_the_wire_event_id() {
        let (notify_on_start, desktop, count, signature, dismissed) = ALLOWED;
        let payload = updates_payload(notify_on_start, desktop, count, signature, dismissed)
            .expect("payload");
        assert_eq!(payload.event_id, "updates-available");
        assert_eq!(payload.event_id, UPDATES_EVENT);
        assert_eq!(payload.title_key, "Updates available");
    }

    #[test]
    fn a_disabled_start_announcement_stays_silent() {
        let (_, desktop, count, signature, dismissed) = ALLOWED;
        assert_eq!(
            updates_payload(false, desktop, count, signature, dismissed),
            None
        );
    }

    #[test]
    fn a_profile_without_desktop_notifications_stays_silent() {
        // `notifyOnStart` is on, so the window still shows the count; only the
        // desktop popup is switched off.
        let (notify_on_start, _, count, signature, dismissed) = ALLOWED;
        assert_eq!(
            updates_payload(notify_on_start, false, count, signature, dismissed),
            None
        );
    }

    #[test]
    fn a_dismissed_signature_is_not_repeated() {
        let (notify_on_start, desktop, count, signature, _) = ALLOWED;
        assert_eq!(
            updates_payload(notify_on_start, desktop, count, signature, signature),
            None
        );
    }

    #[test]
    fn a_different_dismissed_signature_does_not_block_a_new_set() {
        let (notify_on_start, desktop, count, signature, _) = ALLOWED;
        assert!(
            updates_payload(notify_on_start, desktop, count, signature, "sig-b").is_some(),
            "a new set deserves a new announcement"
        );
    }

    #[test]
    fn an_empty_signature_means_nothing_was_read_yet() {
        let (notify_on_start, desktop, count, _, dismissed) = ALLOWED;
        assert_eq!(
            updates_payload(notify_on_start, desktop, count, "", dismissed),
            None
        );
        // Even when the dismissed marker is empty too, there is no set to
        // compare: an empty signature must not match an empty marker.
        assert_eq!(
            updates_payload(notify_on_start, desktop, count, "", ""),
            None
        );
    }
}
