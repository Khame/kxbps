// The actual notification, and the only file that imports the module.
//
// `Notifier.qml` loads this through a `Loader`, so a missing
// `org.kde.notification` fails this file's compilation and stops there: the
// rest of the application never sees the error.
//
// A `Notification` is created per announcement and kept until it closes, so
// `closed` and `ignored` can still be answered — that is what marks the set
// delivered. `autoDelete` is off for the same reason: the runtime must not free
// the object while its handlers are the only way to learn the outcome.
//
// The objects are created with `Qt.createQmlObject` rather than declared
// inline: `Notification` has no default property, so an object literal inside
// its body is a parse error, and `defaultAction` is a value that has to be
// assigned. Building both from source keeps the module's own shape visible.

import QtQuick
import org.kde.notification 1.0
import org.kde.ki18n 1.0

QtObject {
    id: popup

    // The popup was activated: the window should open on the Updates filter.
    signal activated()
    // It was closed, timed out or ignored: the set is delivered either way
    // (spec §3.4, "do not loop").
    signal dismissed()

    // The live notification, or null before the first `show`.
    property var current: null

    // Hands one announcement to the desktop service.
    //
    // A second `show` while the first popup is still on screen means the first
    // one describes a set that no longer exists; it is closed first, and its
    // `closed` handler marks the old set delivered. That is the honest answer:
    // the user was told about it and did not act on it.
    function show(payload) {
        if (current !== null) {
            const previous = current
            current = null
            previous.close()
        }
        const notification = createNotification(payload)
        if (notification === null) {
            return
        }
        current = notification
        notification.sendEvent()
    }

    // Builds the `Notification` and its default action, or `null` when the
    // module cannot provide the type after all.
    function createNotification(payload) {
        const notification = Qt.createQmlObject(
            'import QtQml; import org.kde.notification 1.0; '
            + 'Notification { componentName: "kxbps"; '
            + 'iconName: "system-software-update"; '
            + 'urgency: Notification.NormalUrgency; autoDelete: false }',
            popup)
        if (notification === null) {
            return null
        }
        // The runtime's helper reports an error as `null`; a missing module is
        // the only realistic cause, and `Notifier.qml` already handles it.
        notification.eventId = payload.eventId
        notification.title = i18nd("kxbps", "Updates available")
        notification.text = i18ndp("kxbps",
                                   "%1 package can be updated.",
                                   "%1 packages can be updated.",
                                   payload.count)
        notification.closed.connect(dismissed)
        notification.ignored.connect(dismissed)

        // The action is a value on the notification, so it is built the same
        // way and assigned: `defaultAction: NotificationAction { … }` inside
        // the created object would need the type in this file's scope too.
        const action = Qt.createQmlObject(
            'import org.kde.notification 1.0; NotificationAction {}',
            notification)
        if (action !== null) {
            action.label = i18nd("kxbps", "Show updates")
            action.activated.connect(activated)
            notification.defaultAction = action
        }
        return notification
    }
}
