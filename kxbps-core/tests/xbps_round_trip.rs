// SPDX-License-Identifier: GPL-3.0-or-later
//! What the window shows, driven end to end from captured xbps output.
//!
//! Every fixture under `tests/fixtures/xbps/` was captured from xbps 0.60.7 on
//! Void Linux. This file feeds that text through the real parsers and the real
//! domain functions, then checks the values the user reads: names, versions,
//! counts, sizes and plan steps. Nothing here asserts on a private struct, so a
//! refactor inside a module may not silently change what the window displays.
//!
//! The failure side of the same text is here too: an unparsable line, an empty
//! listing and a lock held by another xbps process. Each must reach the stable
//! answer the UI knows, not a panic and not a guess.
//!
//! No process is started and nothing privileged is touched: the transport lives
//! in `kxbps-qt`, which is where a start failure is tested.

use kxbps_core::xbps::{parse_property, parse_repos};
use kxbps_core::{
    DryRun, InstalledMap, PlanAction, PlanReason, XbpsError, effective_updates, held_updates_in,
    installed_map, items_from, lock_waited_on, parse_installed, parse_pkgver_list, parse_search,
    parse_updates, plan_totals, read_dry_run, split_pkgver,
};

const INSTALLED: &str = include_str!("fixtures/xbps/installed.txt");
const MANUAL: &str = include_str!("fixtures/xbps/manual.txt");
const UPDATES: &str = include_str!("fixtures/xbps/updates.txt");
const SEARCH: &str = include_str!("fixtures/xbps/search.txt");
const REPOS: &str = include_str!("fixtures/xbps/repos.txt");
const REPOS_0607: &str = include_str!("fixtures/xbps/repos-0.60.7.txt");

/// The store's installed map for the captured `-l` and `-m` answers, with no
/// install dates: xbps has no bulk source for them, so the store keeps zero.
fn store_of_fixtures() -> InstalledMap {
    let records = parse_installed(INSTALLED).expect("the captured -l listing parses");
    let explicit = parse_pkgver_list(MANUAL).expect("the captured -m listing parses");
    installed_map(&records, &explicit, &[], &|_| 0)
}

#[test]
fn the_installed_list_becomes_the_store_the_window_opens_with() {
    let store = store_of_fixtures();

    assert_eq!(store.len(), 6, "every package xbps listed is in the store");
    // xbps splits `name-version` at the last hyphen, and the version may start
    // with a digit or a letter; a split at the last dash before a digit would
    // read this one as name `linux6.18` + version `headers-6.18.62_1`.
    assert_eq!(
        store
            .get("linux6.18-headers")
            .map(|inst| inst.version.as_str()),
        Some("6.18.62_1"),
        "a hyphenated name keeps its full name"
    );
    assert_eq!(
        store
            .get("python3-Jinja2")
            .map(|inst| inst.version.as_str()),
        Some("3.1.6_2")
    );
    assert_eq!(
        store.get("AppStream-qt").map(|inst| inst.version.as_str()),
        Some("1.1.2_1")
    );

    // `-m` is the only source of "the user asked for this". The capture has no
    // overlap with the installed list, which is the honest case: a package the
    // user asked for may have been removed since, and the store must not invent
    // an entry for it.
    let explicit: Vec<&str> = store
        .iter()
        .filter(|(_, inst)| inst.explicit)
        .map(|(name, _)| name.as_str())
        .collect();
    assert!(
        explicit.is_empty(),
        "no installed package is in the -m capture, so none is explicit"
    );
    assert!(
        store.values().all(|inst| !inst.held),
        "no capture had a -H answer, so nothing is held"
    );
}

#[test]
fn the_updates_list_shows_the_version_the_dry_run_writes() {
    let store = store_of_fixtures();
    let pending = parse_updates(UPDATES).expect("the captured -un listing parses");

    assert_eq!(pending.len(), 7);
    let chromium = pending
        .iter()
        .find(|entry| entry.name == "chromium")
        .expect("chromium is pending");
    assert_eq!(chromium.version, "157.0.8103.50_1");
    assert_eq!(chromium.installed_size, 426_243_959);
    assert_eq!(chromium.download_size, 153_176_879);

    // The plan row reads `installed → repo`: the new version is xbps' answer,
    // the old one is the only thing the store can supply.
    let items = items_from(DryRun::System, &store, &[], &pending);
    let headers = items
        .iter()
        .find(|item| item.name == "linux6.18-headers")
        .expect("the headers update is planned");
    assert_eq!(headers.action, PlanAction::Update);
    assert_eq!(headers.from.as_deref(), Some("6.18.62_1"));
    assert_eq!(headers.to.as_deref(), Some("6.18.63_1"));
    assert_eq!(
        headers.reason,
        PlanReason::Dependency,
        "the user named nothing, so every row is a dependency"
    );

    // The status strip's numbers are the sum of the rows it shows.
    let totals = plan_totals(&items);
    assert_eq!(totals.added, 1_024_149_048);
    assert_eq!(totals.download, 569_992_657);
    assert_eq!(totals.removed, 0, "an update removes nothing");
}

#[test]
fn a_held_package_drops_out_of_the_updates_the_window_shows() {
    let mut store = store_of_fixtures();
    store.get_mut("linux6.18-headers").expect("installed").held = true;
    let pending = parse_updates(UPDATES).expect("parses");

    let shown = effective_updates(&pending, &store);
    assert_eq!(shown.len(), 6, "the held row is not offered");
    assert!(
        shown.iter().all(|entry| entry.name != "linux6.18-headers"),
        "a hold hides exactly its own package"
    );
    assert_eq!(held_updates_in(&pending, &store), ["linux6.18-headers"]);
}

#[test]
fn a_search_row_separates_the_tag_the_name_and_the_version() {
    let hits = parse_search(SEARCH).expect("the captured -Rs listing parses");

    assert_eq!(hits.len(), 9);
    let installed: Vec<&str> = hits
        .iter()
        .filter(|hit| hit.installed)
        .map(|hit| hit.name.as_str())
        .collect();
    assert_eq!(installed, ["AppStream"], "only `[*]` means installed");

    let kakoune = hits
        .iter()
        .find(|hit| hit.name == "kakoune")
        .expect("kakoune is a hit");
    assert_eq!(kakoune.version, "2026.05.21_1");
    assert_eq!(
        kakoune.description,
        "Selection-based vim-like editor with less keystrokes"
    );
    // The row shows name and version separately; the raw field is kept for the
    // queue, which asks xbps for a pkgver, not for a name.
    assert_eq!(kakoune.pkgver, "kakoune-2026.05.21_1");
    assert!(
        hits.iter().all(|hit| hit.name != "gvim-huge-extra"),
        "no row is invented from the description"
    );
}

#[test]
fn the_repository_list_marks_an_unavailable_index_as_unavailable() {
    let repos = parse_repos(REPOS).expect("the captured -L listing parses");
    assert_eq!(repos.len(), 5);
    assert!(
        repos.iter().all(|repo| repo.available()),
        "the current-system capture has no unavailable index"
    );
    let local = repos
        .iter()
        .find(|repo| repo.url.contains("khame.github.io"))
        .expect("the local repository is listed");
    assert_eq!(local.count, 5);
    assert!(
        local.signed,
        "every readable index in this capture is signed"
    );

    // The 0.60.7 capture is the one with a `-1` row: xbps could not read that
    // index at all, which is neither "empty" nor "verified".
    let old = parse_repos(REPOS_0607).expect("the older -L listing parses");
    let unavailable: Vec<&str> = old
        .iter()
        .filter(|repo| !repo.available())
        .map(|repo| repo.url.as_str())
        .collect();
    assert_eq!(
        unavailable,
        ["https://repo-fi.voidlinux.org/current"],
        "a `-1` count is the unavailable index and nothing else"
    );
    let maybe = old
        .iter()
        .find(|repo| repo.url.contains("repo-fi"))
        .expect("listed");
    assert!(
        !maybe.signed,
        "a maybe-signed index is not shown as verified"
    );
}

#[test]
fn a_property_take_shows_the_value_xbps_printed() {
    let value = parse_property(include_str!("fixtures/xbps/property.txt")).expect("parses");
    assert_eq!(value, "AppStream-1.1.2_1");
    assert_eq!(
        split_pkgver(&value),
        Some(("AppStream", "1.1.2_1")),
        "the details pane splits the same way the list does"
    );
}

#[test]
fn a_dry_run_that_prints_nothing_is_an_empty_plan_not_an_error() {
    let store = store_of_fixtures();
    let outcome =
        read_dry_run(DryRun::Install, &store, &["vim".to_owned()], "", "").expect("valid empty");
    assert!(outcome.items.is_empty());
    assert!(outcome.blockers.is_empty());
    assert_eq!(plan_totals(&outcome.items), plan_totals(&[]));
}

#[test]
fn a_refused_removal_names_the_packages_that_need_the_target() {
    let store = store_of_fixtures();
    // Captured wording of xbps 0.60.7: one refusal line per installed package,
    // then xbps' own summary. `acpid` is not in the captured store, which is the
    // real case too: the user removed something the store never listed.
    let stderr = "\
acpid-2.0.34_4 in transaction breaks installed pkg `base-system-0.114_2'
acpid-2.0.34_4 in transaction breaks installed pkg `NetworkManager-1.58.1_2'
ERROR: Transaction aborted due to unresolved dependencies.
";
    let outcome = read_dry_run(DryRun::Remove, &store, &["acpid".to_owned()], "", stderr)
        .expect("stderr is read");

    assert!(
        outcome.items.is_empty(),
        "a refused transaction has no rows"
    );
    let blocked = outcome.blocked();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].name, "acpid");
    assert_eq!(
        blocked[0].needed_by,
        ["base-system", "NetworkManager"],
        "the dialog lists every package that would break, in xbps' order"
    );
}

#[test]
fn a_line_this_version_does_not_understand_is_reported_with_its_text() {
    // The window shows `(line N): <text>`, so the number and the raw line have
    // to survive the parser untouched.
    let text = "ii vim-9.2.1031_1 Editor\nnot a package line\n";
    let error = parse_installed(text).expect_err("the second line is not a package");
    match error {
        XbpsError::MalformedLine { line, text, .. } => {
            assert_eq!(line, 2);
            assert_eq!(text, "not a package line");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}

#[test]
fn an_empty_installed_list_is_a_failure_not_an_empty_system() {
    // `xbps-query -l` prints at least one line on any working system, so no
    // output is xbps being broken, and an empty list would be a lie.
    assert_eq!(parse_installed(""), Err(XbpsError::Empty));
    assert_eq!(parse_repos("\n"), Err(XbpsError::Empty));
    // A search that matches nothing is a real answer and stays one.
    assert_eq!(parse_search(""), Ok(Vec::new()));
    // No updates is a real answer too.
    assert_eq!(parse_updates(""), Ok(Vec::new()));
}

#[test]
fn a_lock_waited_on_is_recognised_as_a_lock_and_not_as_a_failure() {
    // The real wording, from libxbps' format strings: a second xbps run waits
    // for the holder instead of failing, so its output still carries a plan and
    // the UI has to be able to say why the start was slow.
    let database = "package database locked, waiting...\n";
    let repository = "repository locked: https://repo-de.voidlinux.org/current: waiting...\n";
    assert!(lock_waited_on(database), "the database lock is recognised");
    assert!(
        lock_waited_on(repository),
        "the repository lock is recognised"
    );
    assert!(!lock_waited_on("ERROR: Package `bash' already installed."));
    assert!(!lock_waited_on("repository locked: but nobody is waiting"));

    // The holder can be another user's process: nothing in the text names the
    // caller, so recognition must not depend on who owns the lock.
    assert!(lock_waited_on(database.trim()));

    // A lock line is not a plan row and not a refusal: the window stays honest
    // with an empty plan rather than inventing steps from the message.
    let store = store_of_fixtures();
    let outcome =
        read_dry_run(DryRun::Install, &store, &["vim".to_owned()], "", database).expect("read");
    assert!(outcome.items.is_empty(), "a lock line is not a plan");
    assert!(outcome.blockers.is_empty(), "a lock line blocks nothing");
}
