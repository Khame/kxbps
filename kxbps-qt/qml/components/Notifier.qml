// The one desktop notification KXbps sends: "n updates are available".
//
// The decision of *whether* to announce is Rust's
// (`crate::notification::updates_payload`), mirrored here because a Rust
// function is not callable from QML; both sides are small enough that the
// duplication is the cheaper cost.
//
// The transport is `appBackend.notify`, which calls
// `org.freedesktop.Notifications.Notify` on the session bus. The earlier
// version used `org.kde.notification`, whose `sendEvent()` silently drops an
// event whose component it cannot resolve and answers with an immediate
// `closed` — the popup never appeared, and the set was marked delivered anyway.
// The bus does not have that failure mode.

import QtQuick
import org.kde.ki18n 1.0

QtObject {
    id: notifier

    property var installedBackend
    property var prefsBackend
    property var appBackend

    // The set was announced. `Main.qml` writes the signature into the
    // dismissed marker so the next start stays quiet.
    signal delivered(string signature)

    // Offers one announcement to the desktop service, if the decision table
    // allows it.
    function consider() {
        if (installedBackend === null || prefsBackend === null || appBackend === null) {
            return
        }
        if (!shouldNotify()) {
            return
        }
        const count = installedBackend.updatesCount
        const signature = installedBackend.pendingSignature
        appBackend.notify(i18nd("kxbps", "Updates available"),
                          i18ndp("kxbps",
                                 "%1 package can be updated.",
                                 "%1 packages can be updated.",
                                 count),
                          "kxbps-updates")
        delivered(signature)
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
}
