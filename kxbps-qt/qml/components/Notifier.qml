// The one desktop notification KXbps sends: "n updates are available".
//
// Two layers, because `org.kde.notification` is an optional module:
//
// 1. This file is a plain `QtObject` with no notification import at all, so the
//    `Loader` that creates it in `Main.qml` always succeeds.
// 2. The popup is a separate file, `NotifierPopup.qml`, the only one with the
//    import. When the module is missing that load fails and this object reports
//    `available == false`; the status strip then carries the count.
//
// Whether anything is announced is decided in Rust
// (`crate::notification::updates_payload`), which is what the unit tests pin;
// this file only renders what it is handed. The decision is repeated here
// because a Rust function is not callable from QML — both sides are simple
// enough that the duplication is the cheaper cost.

import QtQuick
import org.kde.ki18n 1.0

QtObject {
    id: notifier

    property var installedBackend
    property var prefsBackend

    // Raised when the user activates the popup; `Main.qml` shows the window on
    // the Updates filter.
    signal openUpdatesRequested()
    // The set was delivered, seen or not. `Main.qml` writes the signature into
    // the dismissed marker so the next start stays quiet.
    signal delivered(string signature)

    // Whether the notification module loaded. False is a supported state, not
    // an error: the strip is the fallback.
    readonly property bool available: popupLoader.status === Loader.Ready
    readonly property bool failed: popupLoader.status === Loader.Error

    // The payload of the last announcement; kept for the popup's handlers.
    property var pendingPayload: null
    property string pendingSignature: ""

    // Offers one announcement to the popup, if the decision table allows it.
    function consider() {
        if (installedBackend === null || prefsBackend === null) {
            return
        }
        if (!shouldNotify()) {
            return
        }
        const signature = installedBackend.pendingSignature
        if (!available) {
            // No module means no service to hand the payload to. The set is
            // still marked delivered: retrying on every start would only teach
            // the user to ignore a popup they never see.
            if (failed) {
                delivered(signature)
            }
            return
        }
        pendingPayload = {
            eventId: "updates-available",
            count: installedBackend.updatesCount
        }
        pendingSignature = signature
        popupLoader.item.show(pendingPayload)
    }

    // The decision table of `notification.rs::updates_payload`, in the same
    // order: no count, the preference off, the desktop switch off, nothing read
    // yet, or this exact set already dismissed.
    function shouldNotify() {
        const count = installedBackend.updatesCount
        const signature = installedBackend.pendingSignature
        return count > 0
            && prefsBackend.notifyOnStart
            && prefsBackend.desktopNotifications
            && signature.length > 0
            && signature !== prefsBackend.dismissedVersion
    }

    // The popup file, the only importer of `org.kde.notification`. The
    // handlers are attached with `target`, because the popup is a separate file
    // and so has no type name this file could write `onActivated:` for.
    property Loader popupLoader: Loader {
        active: true
        source: "NotifierPopup.qml"

        Connections {
            target: notifier.popupLoader.item
            function onActivated() {
                notifier.openUpdatesRequested()
            }
            function onDismissed() {
                notifier.delivered(notifier.pendingSignature)
            }
        }
    }
}
