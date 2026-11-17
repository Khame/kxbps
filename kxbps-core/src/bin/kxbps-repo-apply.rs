// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `kxbps-repo-apply`: the privileged helper behind the Repositories page.
//!
//! polkit runs this binary as root through the `io.github.khame.kxbps.repo`
//! action. It takes one verb and one value:
//!
//! - `add <https://…>` writes `20-kxbps-<slug>.conf` with one `repository=`
//!   line;
//! - `remove <file.conf>` deletes a file this helper wrote.
//!
//! After a successful `add` it also runs `xbps-install -S`, because a
//! repository without an index is skipped by `xbps-query` and would not appear
//! in any package list. A failed sync exits 6 and keeps the file.
//!
//! Everything it accepts is decided by [`kxbps_core::repository_edit`]; this
//! file only parses the argv, prints what happened on stderr and exits with the
//! documented code. A value that is not one of those verbs is refused before
//! any file is touched.

use kxbps_core::mirror_apply::sync_indexes;
use kxbps_core::repository_edit::{self, CONFIG_DIR, RepoError, SYNC_FAILED};

fn main() {
    let mut args = std::env::args().skip(1);
    let verb = args.next().unwrap_or_default();
    let value = args.next().unwrap_or_default();
    if args.next().is_some() {
        eprintln!("kxbps-repo-apply: expected one verb and one value");
        std::process::exit(RepoError::UrlRefused.code());
    }
    let dir = std::path::Path::new(CONFIG_DIR);
    let result = match verb.as_str() {
        "add" => repository_edit::add(dir, &value).map(|_| ()),
        "remove" => repository_edit::remove(dir, &value),
        _ => {
            eprintln!("usage: kxbps-repo-apply add <https://…> | remove <file.conf>");
            std::process::exit(RepoError::UrlRefused.code());
        }
    };
    match result {
        Ok(()) => {
            // A new repository has no index until `xbps-install -S` fetches
            // one, and `xbps-query` skips it without a word: the package list
            // would not show the repository the user just added. Same
            // authorization, so one password does both.
            if verb == "add"
                && let Err(detail) = sync_indexes()
            {
                eprintln!("kxbps-repo-apply: index sync failed: {detail}");
                std::process::exit(SYNC_FAILED);
            }
        }
        Err(error) => {
            eprintln!("kxbps-repo-apply: {}", error.key());
            std::process::exit(error.code());
        }
    }
}
