// SPDX-License-Identifier: GPL-2.0-or-later
//
// Buttons without an underlined mnemonic.
//
// The desktop QML style paints a button's label through `QStyle`: its
// background style item passes `Kirigami.MnemonicData.mnemonicLabel` to
// `QStyle::drawControl(CE_PushButton)`, and that label carries `&` before the
// letter the Alt shortcut activates. The style lays the text out with
// `Qt::TextShowMnemonic`, which underlines the letter — in every state, on
// every button, and regardless of the button font.
//
// Two settings that look like the fix are not:
//
//   * `font.underline: false` — the underline is not a font attribute, it is
//     added while the mnemonic text is laid out.
//   * `QStyle::SH_UnderlineShortcut` — the style never asks; the hint exists
//     for widget code that draws labels itself.
//
// So the proxy rewrites what the style is asked to paint: the same option, with
// the mnemonic markers dropped from the label. Kirigami keeps its own copy of
// the label and derives the `Shortcut` from it, so Alt+letter still activates
// the button; only the drawn text changes.

#include "style_shim.h"

#include <QApplication>
#include <QProxyStyle>
#include <QStyle>
#include <QStyleOption>
#include <QStyleOptionButton>
#include <QString>

namespace {

// Drops the `&` mnemonic markers: `&Save` and `Save` both read `Save`, and a
// doubled `&&`, which Qt shows as a literal ampersand, becomes one `&`.
QString labelWithoutMnemonics(const QString &text)
{
    QString plain;
    plain.reserve(text.size());
    for (qsizetype i = 0; i < text.size(); ++i) {
        if (text.at(i) != QLatin1Char('&')) {
            plain.append(text.at(i));
        } else if (i + 1 < text.size() && text.at(i + 1) == QLatin1Char('&')) {
            plain.append(QLatin1Char('&'));
            ++i;
        }
    }
    return plain;
}

class PlainButtonStyle : public QProxyStyle
{
public:
    explicit PlainButtonStyle(const QString &key)
        : QProxyStyle(key)
    {
    }

    void drawControl(ControlElement element, const QStyleOption *option, QPainter *painter,
                     const QWidget *widget) const override
    {
        // The bevel and the label are separate elements, and a style may paint
        // both in one call, so both are rewritten.
        if (element == CE_PushButton || element == CE_PushButtonLabel) {
            if (const auto *button = qstyleoption_cast<const QStyleOptionButton *>(option)) {
                if (button->text.contains(QLatin1Char('&'))) {
                    QStyleOptionButton plain(*button);
                    plain.text = labelWithoutMnemonics(button->text);
                    QProxyStyle::drawControl(element, &plain, painter, widget);
                    return;
                }
            }
        }
        QProxyStyle::drawControl(element, option, painter, widget);
    }
};

} // namespace

void kxbps_set_plain_buttons()
{
    const QStyle *current = QApplication::style();
    if (!current) {
        return;
    }

    // A fresh style of the same kind, not the live one: `QProxyStyle` takes
    // ownership of the style it wraps, and handing it the application style
    // would leave the two objects owning the same instance. `QProxyStyle`
    // falls back to the desktop style when the key is unknown.
    QApplication::setStyle(new PlainButtonStyle(current->objectName()));
}
