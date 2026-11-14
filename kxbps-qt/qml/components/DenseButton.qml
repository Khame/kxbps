// A button that hugs its label.
//
// The Breeze style sizes a button from the grid unit — about two of them — and
// not from the text inside. With the application font capped at 10 pt that left
// a tall empty box around every label, which is what made the toolbars and the
// details pane read as oversized. This keeps the height close to the text and
// the padding small; everything else still comes from the instance.

import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

QQC2.Button {
    implicitHeight: Kirigami.Units.gridUnit * 1.6
    padding: Kirigami.Units.smallSpacing
}
