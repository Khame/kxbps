// SPDX-License-Identifier: GPL-2.0-or-later
//
// Declaration for the style proxy in `style_shim.cpp`. The CXX bridge in
// `src/lib.rs` includes this header so the code cxx generates has the
// declaration of `kxbps_set_plain_buttons` in scope.

#pragma once

/// Stops the desktop style from underlining the keyboard mnemonic in a button
/// label.
///
/// The Breeze QML style draws a button's label inside its background style item
/// and hands it `Kirigami.MnemonicData.mnemonicLabel`, a string with `&` before
/// the letter the Alt shortcut activates. The style draws that text with
/// `Qt::TextShowMnemonic`, so the letter comes out underlined. Neither
/// `font.underline: false` nor `QStyle::SH_UnderlineShortcut` changes that,
/// because the underline is decided while the text is laid out, not by the font
/// or by that style hint.
///
/// The proxy rewrites the option the style paints, dropping the marker and
/// keeping the letter. The shortcut itself is a `Shortcut` item Kirigami
/// derives from the same label, so it keeps working.
///
/// Must run after the `QApplication` exists and before the QML engine loads:
/// every button reads the application style when it is drawn.
void kxbps_set_plain_buttons();
