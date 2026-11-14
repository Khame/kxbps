// The one desktop notification KXbps sends.
//
// The decision of *whether* to announce, and the bookkeeping that makes it one
// popup plus one reminder per set, live in Rust
// (`installedBackend.takeAnnouncement`), where they are tested without a bus or
// a display. This file only renders what Rust offered: it translates the texts,
// sends them, and tells the caller whether a popup went out.
//
// The transport is `appBackend.notify`, which calls
// `org.freedesktop.Notifications.Notify` on the session bus. The earlier
// version used `org.kde.notification`, whose `sendEvent()` silently drops an
// event whose component it cannot resolve and answers with an immediate
// `closed`; the bus does not have that failure mode.
//
// Clicking the popup brings the window back through the `default` action; the
// backend sends `notificationActivated` for it, and `Main.qml` shows the
// window. Nothing is sent here for that: the bus delivers it.

import QtQuick
import org.kde.ki18n 1.0

QtObject {
    id: notifier

    property var installedBackend
    property var appBackend

    // Offers one announcement to the desktop service, if the backend decides
    // one is due. `visible` is the window's visibility: a popup over the list
    // the user is reading is noise, so the backend defers instead.
    //
    // Returns `first`, `reminder`, or an empty string when nothing was sent, so
    // the caller knows whether to arm the reminder timer or try again later.
    function consider(visible) {
        if (installedBackend === null || appBackend === null) {
            return ""
        }
        const raw = installedBackend.takeAnnouncement(visible)
        if (raw.length === 0) {
            return ""
        }
        const offer = JSON.parse(raw)
        appBackend.notify(i18nd("kxbps", offer.summaryKey),
                          i18ndp("kxbps", offer.bodyOneKey, offer.bodyManyKey,
                                 Number(offer.bodyArg)),
                          "kxbps-updates")
        return offer.kind
    }
}
