// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! A failed save reaches the window as a translation key, not as English.
//!
//! `StoreError`'s `Display` names paths and OS errors, which no translation
//! catalog can carry. The UI therefore gets [`StoreError::key`] plus
//! [`StoreError::args`] and builds the sentence itself; `Display` stays the
//! technical form for the log.

use std::path::PathBuf;

use kxbps_core::StoreError;

fn io_error() -> std::io::Error {
    std::io::Error::other("Read-only file system")
}

/// Each variant names a key QML knows; the key is one camelCase word.
#[test]
fn every_store_error_variant_has_its_own_key() {
    let cases = [
        (StoreError::NoDataDir, "errorStoreNoDataDir"),
        (
            StoreError::Io {
                path: PathBuf::from("/home/u/.local/share/kxbps/queue.json"),
                source: io_error(),
            },
            "errorStoreWrite",
        ),
        (
            StoreError::Json {
                path: PathBuf::from("/home/u/.local/share/kxbps/prefs.json"),
                source: serde_json::from_str::<u8>("nope").expect_err("not a u8"),
            },
            "errorStoreFormat",
        ),
    ];
    for (error, key) in cases {
        assert_eq!(error.key(), key, "{error} should map to {key}");
        assert!(
            !error.key().is_empty() && !error.key().contains(' '),
            "{} must be a single identifier, not a sentence",
            error.key()
        );
    }
}

/// The path and the underlying error travel as arguments, in `%1…` order.
#[test]
fn a_write_error_carries_the_path_and_the_os_reason() {
    let error = StoreError::Io {
        path: PathBuf::from("/home/u/.local/share/kxbps/queue.json"),
        source: io_error(),
    };
    assert_eq!(
        error.args(),
        vec![
            String::from("/home/u/.local/share/kxbps/queue.json"),
            String::from("Read-only file system"),
        ]
    );
}

/// The format error names the file; the parser's own wording stays technical.
#[test]
fn a_format_error_carries_the_path_and_the_parser_reason() {
    let parse = serde_json::from_str::<u8>("nope").expect_err("not a u8");
    let reason = parse.to_string();
    let error = StoreError::Json {
        path: PathBuf::from("/home/u/.local/share/kxbps/prefs.json"),
        source: parse,
    };
    assert_eq!(
        error.args(),
        vec![
            String::from("/home/u/.local/share/kxbps/prefs.json"),
            reason
        ]
    );
}

/// A machine with no data directory has nothing to name.
#[test]
fn a_missing_data_directory_carries_no_arguments() {
    assert!(StoreError::NoDataDir.args().is_empty());
}

/// `Display` keeps the full technical text: it is what the log shows.
#[test]
fn display_still_carries_the_path_and_the_reason() {
    let error = StoreError::Io {
        path: PathBuf::from("/x/queue.json"),
        source: io_error(),
    };
    let text = error.to_string();
    assert!(text.contains("/x/queue.json"), "{text}");
    assert!(text.contains("Read-only file system"), "{text}");
}
