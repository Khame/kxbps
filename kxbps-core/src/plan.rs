//! What a `-n` dry run reported, turned into plan steps.
//!
//! xbps already resolves the whole transaction: `xbps-install -n` and
//! `xbps-remove -n` print the same one-line-per-package table the Updates list
//! reads, and they print it without touching a single file. This module parses
//! that table (through [`crate::xbps::parse_updates`]) plus the stderr lines
//! that say why a transaction was refused, and nothing else: the process itself
//! is started by the Qt layer, which owns the transport.
//!
//! Two answers are not rows on stdout:
//!
//! - **blocked** removals. `xbps-remove -n` exits 19 after printing
//!   `<pkgver> in transaction breaks installed pkg `<name>'' once per package
//!   that still needs the target, then `ERROR: Transaction aborted …`. The
//!   breaking names are the ones the Blocked dialog lists.
//! - **nothing to do**. A dry run that prints no rows and exits 0 has a valid
//!   empty plan; it is a notice, never an error.
//!
//! Nothing here is a solver: the order, the versions and the sizes are xbps'
//! own answer, which is the whole point of asking it.

use crate::types::{Blocked, PlanAction, PlanItem, PlanReason};
use crate::xbps::{UpdateEntry, XbpsAction, XbpsError, parse_updates, split_pkgver};

use crate::request::DryRun;

/// Exit code xbps uses for a transaction it refused because of dependencies.
pub const EXIT_UNRESOLVED: i32 = 19;

/// Exit code of `xbps-remove -n` for a package that is not installed.
pub const EXIT_NOT_INSTALLED: i32 = 2;

/// One package xbps refused to break.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Breaker {
    /// The package the user wants out (`acpid` for `acpid-2.0.34_4`).
    pub name: String,
    /// The installed packages whose removal of `name` would break, in the order
    /// xbps printed them.
    pub needed_by: Vec<String>,
}

/// The outcome of a dry run that did not fail outright.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DryRunOutcome {
    /// The steps in the order xbps would apply them.
    pub items: Vec<PlanItem>,
    /// Packages xbps refused to remove; empty for an install run.
    pub blockers: Vec<Breaker>,
}

impl DryRunOutcome {
    /// The blockers as the shared [`Blocked`] entries the dialog renders.
    ///
    /// Grouped by name in first-seen order and deduplicated, because one
    /// package can appear in more than one refusal line.
    pub fn blocked(&self) -> Vec<Blocked> {
        let mut out: Vec<Blocked> = Vec::new();
        for breaker in &self.blockers {
            let entry = match out.iter_mut().find(|entry| entry.name == breaker.name) {
                Some(entry) => entry,
                None => {
                    out.push(Blocked {
                        name: breaker.name.clone(),
                        needed_by: Vec::new(),
                    });
                    out.last_mut().expect("just pushed")
                }
            };
            for name in &breaker.needed_by {
                if !entry.needed_by.contains(name) {
                    entry.needed_by.push(name.clone());
                }
            }
        }
        out
    }
}

/// Turns one dry run's output into plan steps and, for a removal, blockers.
///
/// `stdout` is the plan table and is parsed strictly; `stderr` is only read for
/// the phrases xbps uses to explain a refusal, so an unrecognised line in it is
/// ignored rather than a failure. `requested` is what the user asked for, which
/// is why it is not asked of xbps: the names the dry run reports and the queue
/// asked for become [`PlanReason::Requested`], everything else the dependencies
/// xbps pulled in.
pub fn read_dry_run(
    run: DryRun,
    installed: &crate::types::InstalledMap,
    requested: &[String],
    stdout: &str,
    stderr: &str,
) -> Result<DryRunOutcome, XbpsError> {
    let entries = parse_updates(stdout)?;
    let blockers = if run.removes() {
        parse_breakers(stderr)
    } else {
        Vec::new()
    };
    Ok(DryRunOutcome {
        items: items_from(run, installed, requested, &entries),
        blockers,
    })
}

/// The steps of an install/update dry run, in xbps' order.
///
/// A dependency xbps reports with action `update` is still an update, but its
/// reason is [`PlanReason::Dependency`] unless the user named it.
pub fn items_from(
    run: DryRun,
    installed: &crate::types::InstalledMap,
    requested: &[String],
    entries: &[UpdateEntry],
) -> Vec<PlanItem> {
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        let (action, reason) = match (run, entry.action) {
            (DryRun::Remove, _) => (
                PlanAction::Remove,
                removal_reason(installed, requested, entry),
            ),
            (_, XbpsAction::Remove) => (PlanAction::Remove, PlanReason::Dependency),
            (_, XbpsAction::Update) => (PlanAction::Update, install_reason(requested, entry)),
            (_, XbpsAction::Reinstall) => (PlanAction::Reinstall, install_reason(requested, entry)),
            (_, XbpsAction::Install | XbpsAction::Configure) => {
                (PlanAction::Install, install_reason(requested, entry))
            }
        };
        items.push(PlanItem {
            name: entry.name.clone(),
            action,
            // `xbps-install -n` names the version it will write and nothing
            // about the old one; the removal dry run is the only place xbps
            // echoes the installed version, and it is `from` there.
            from: match action {
                PlanAction::Remove => entry.version.clone().into(),
                PlanAction::Update => installed
                    .get(&entry.name)
                    .map(|inst| inst.version.clone())
                    .filter(|version| *version != entry.version),
                PlanAction::Install | PlanAction::Reinstall => None,
            },
            to: match action {
                PlanAction::Remove => None,
                _ => Some(entry.version.clone()),
            },
            reason,
            download: entry.download_size,
            size: entry.installed_size,
        });
    }
    items
}

fn install_reason(requested: &[String], entry: &UpdateEntry) -> PlanReason {
    if requested.iter().any(|name| name == &entry.name) {
        PlanReason::Requested
    } else {
        PlanReason::Dependency
    }
}

/// A removal is either the package the user named, a package xbps keeps only
/// because something needs it, or — for `xbps-remove -n` on an orphan — a name
/// nothing depends on any more.
fn removal_reason(
    installed: &crate::types::InstalledMap,
    requested: &[String],
    entry: &UpdateEntry,
) -> PlanReason {
    if requested.iter().any(|name| name == &entry.name) {
        return PlanReason::Requested;
    }
    match installed.get(&entry.name) {
        Some(inst) if inst.explicit => PlanReason::Reverse,
        _ => PlanReason::Orphan,
    }
}

/// The refusal lines of `xbps-remove -n`.
///
/// Each line reads `<pkgver> in transaction breaks installed pkg `<name>''.
/// The pkgver needs no quoting in practice, but xbps brackets the *installed*
/// name with backticks and a straight quote, so the name is taken up to the
/// first of those. A line that does not match is skipped: stderr also carries
/// the `ERROR:` summary and, on a failure, whatever else xbps thought worth
/// saying.
pub fn parse_breakers(stderr: &str) -> Vec<Breaker> {
    const MARKER: &str = " in transaction breaks installed pkg ";
    let mut out: Vec<Breaker> = Vec::new();
    for line in stderr.lines() {
        let Some((pkgver, rest)) = line.split_once(MARKER) else {
            continue;
        };
        let Some((name, _)) = split_pkgver(pkgver.trim()) else {
            continue;
        };
        let needed = rest
            .trim_start_matches(['`', '\'', '"'])
            .split(['`', '\'', '"'])
            .next()
            .unwrap_or("")
            .trim();
        // `needed_by` is a package name, like the catalog path's `Blocked`;
        // xbps prints the full pkgver here. A bare name (no version) is kept
        // as it is.
        let needed = match split_pkgver(needed) {
            Some((name, _)) => name,
            None => needed,
        };
        if needed.is_empty() {
            continue;
        }
        match out.iter_mut().find(|breaker| breaker.name == name) {
            Some(breaker) => {
                if !breaker.needed_by.iter().any(|other| other == needed) {
                    breaker.needed_by.push(needed.to_owned());
                }
            }
            None => out.push(Breaker {
                name: name.to_owned(),
                needed_by: vec![needed.to_owned()],
            }),
        }
    }
    out
}

/// Whether xbps reported a lock held by another process.
///
/// Taken from libxbps' own format strings (`strings /usr/lib/libxbps.so`), not
/// from a guess: a second xbps run prints `package database locked, waiting...`
/// or `repository locked: <url>: waiting...` and then waits for the holder
/// instead of failing. So a lock is a slow start, never an error, and the UI
/// must say so rather than look hung.
///
/// The holder can be any process of any user: a root transaction, a terminal
/// `xbps-install`, or, during the outer loop, an `xbps-remote`. Nothing in the
/// text says whose process it is, which is why the sentence names the lock and
/// not the caller.
pub fn lock_waited_on(stderr: &str) -> bool {
    stderr.lines().any(|line| {
        let line = line.trim();
        if line.contains("package database locked, waiting") {
            return true;
        }
        // `repository locked: <url>: waiting...` — the URL sits between the two
        // fixed halves, so both are required.
        line.contains("repository locked: ") && line.contains(": waiting")
    })
}

/// Whether stderr is xbps saying the package is not installed at all.
///
/// `xbps-remove -n` exits 2 with ``Package `zramen' is not currently
/// installed.``; that is a notice about an empty removal, not a failure.
pub fn is_not_installed(stderr: &str) -> bool {
    stderr
        .lines()
        .any(|line| line.contains("is not currently installed"))
}

/// Whether stderr is xbps saying the package is already installed.
///
/// `xbps-install -n bash` exits **0** with ``ERROR: Package `bash' already
/// installed.``; the `ERROR:` prefix makes it look like a failure, and it is
/// not: the transaction is simply empty.
pub fn is_already_installed(stderr: &str) -> bool {
    stderr
        .lines()
        .any(|line| line.contains("already installed"))
}

/// Whether every non-empty stderr line is the already-installed notice.
///
/// `xbps-install` prints ``Package `x' already installed.`` for a name it has
/// already put in the transaction — and an aborted transaction carries the
/// sentence too. A non-zero exit is a notice only when the stderr says nothing
/// else; anything more is a failure, and hiding it would drop the queue entries
/// the sentence names.
pub fn is_only_already_installed(stderr: &str) -> bool {
    let mut noticed = false;
    for line in stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if !is_already_installed(line) {
            return false;
        }
        noticed = true;
    }
    noticed
}

/// The names xbps reported as already installed, in output order.
///
/// `xbps-install -n <names>` answers ``ERROR: Package `<name>' already
/// installed.`` per name the machine already holds at the newest version the
/// repositories offer. Those queue entries have nothing left to do: the
/// transaction installs nothing for them, so they leave the queue instead of
/// staying there as rows that look queued and change nothing.
pub fn already_installed_names(stderr: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in stderr.lines() {
        let line = line.trim();
        let Some(rest) = line
            .strip_prefix("ERROR: Package `")
            .or_else(|| line.strip_prefix("Package `"))
        else {
            continue;
        };
        let Some((name, _)) = rest.split_once("' already installed") else {
            continue;
        };
        let name = name.trim();
        if !name.is_empty() && !names.iter().any(|seen| seen == name) {
            names.push(name.to_owned());
        }
    }
    names
}

/// Whether stderr is xbps saying the package is not in any repository.
///
/// `xbps-install -n nosuch` exits 2 with `ERROR: Package <name> not found in
/// repository pool.`, which is a real failure: the user named something that
/// does not exist.
pub fn missing_package(stderr: &str) -> Option<String> {
    for line in stderr.lines() {
        // A line that does not carry the prefix says nothing about a missing
        // package; it must not stop the search, because xbps writes warnings
        // before the answer.
        let Some(rest) = line
            .strip_prefix("ERROR: Package ")
            .or_else(|| line.strip_prefix("Package "))
        else {
            continue;
        };
        // The notice about an installed package is also prefixed `ERROR:
        // Package`; only the "not found" wording means the name is unknown.
        let name = rest
            .split(" not found")
            .next()
            .filter(|_| rest.contains(" not found"))
            .unwrap_or("")
            .trim()
            .trim_matches(['`', '\'', '"']);
        if !name.is_empty() {
            return Some(name.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Inst;

    fn inst(version: &str, explicit: bool) -> Inst {
        Inst {
            version: version.to_owned(),
            explicit,
            held: false,
            at: 0,
        }
    }

    fn installed(entries: &[(&str, &str, bool)]) -> crate::types::InstalledMap {
        entries
            .iter()
            .map(|(name, version, explicit)| ((*name).to_owned(), inst(version, *explicit)))
            .collect()
    }

    /// Captured on Void with xbps 0.60.7.
    const INSTALL_HELLO: &str =
        "hello-2.12.3_1 install x86_64 https://repo-de.voidlinux.org/current 220488 73301\n";

    #[test]
    fn an_install_dry_run_becomes_install_steps() {
        let outcome = read_dry_run(
            DryRun::Install,
            &installed(&[]),
            &["hello".to_owned()],
            INSTALL_HELLO,
            "",
        )
        .expect("the table parses");
        assert_eq!(outcome.blockers, Vec::new());
        assert_eq!(
            outcome.items,
            vec![PlanItem {
                name: "hello".into(),
                action: PlanAction::Install,
                from: None,
                to: Some("2.12.3_1".into()),
                reason: PlanReason::Requested,
                download: 73301,
                size: 220488,
            }]
        );
    }

    #[test]
    fn dependencies_are_reasons_not_requested_packages() {
        let table = "\
bash-5.2.037_1 install x86_64 https://repo-de.voidlinux.org/current 8100000 1800000
vim-9.2.1031_1 install x86_64 https://repo-de.voidlinux.org/current 4194304 1468006
";
        let items = read_dry_run(
            DryRun::Install,
            &installed(&[]),
            &["vim".to_owned()],
            table,
            "",
        )
        .expect("parses")
        .items;
        assert_eq!(items[0].reason, PlanReason::Dependency);
        assert_eq!(items[1].reason, PlanReason::Requested);
    }

    #[test]
    fn an_update_keeps_the_installed_version_as_from() {
        let table = "chromium-157.0.8103.50_1 update x86_64 https://repo-de.voidlinux.org/current 426243959 153176879\n";
        let items = read_dry_run(
            DryRun::System,
            &installed(&[("chromium", "156.0_1", true)]),
            &[],
            table,
            "",
        )
        .expect("parses")
        .items;
        assert_eq!(items[0].action, PlanAction::Update);
        assert_eq!(items[0].from.as_deref(), Some("156.0_1"));
        assert_eq!(items[0].to.as_deref(), Some("157.0.8103.50_1"));
        assert_eq!(items[0].reason, PlanReason::Dependency);
    }

    #[test]
    fn an_empty_dry_run_is_a_valid_empty_plan() {
        let outcome =
            read_dry_run(DryRun::System, &installed(&[]), &[], "", "").expect("empty is valid");
        assert_eq!(outcome, DryRunOutcome::default());
    }

    /// Captured on Void with xbps 0.60.7.
    const REMOVE_BLOCKED_STDERR: &str = "\
acpid-2.0.34_4 in transaction breaks installed pkg 'base-system-0.114_2'
ERROR: Transaction aborted due to unresolved dependencies.
";

    #[test]
    fn a_blocked_removal_is_read_from_stderr() {
        let blockers = parse_breakers(REMOVE_BLOCKED_STDERR);
        assert_eq!(
            blockers,
            vec![Breaker {
                name: "acpid".into(),
                needed_by: vec!["base-system".into()],
            }]
        );
        let blocked = DryRunOutcome {
            items: Vec::new(),
            blockers,
        }
        .blocked();
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].name, "acpid");
        assert_eq!(blocked[0].needed_by, vec!["base-system".to_owned()]);
    }

    #[test]
    fn several_refusals_of_one_package_keep_every_dependent() {
        let stderr = "\
glibc-2.41_1 in transaction breaks installed pkg 'bash-5.2_1'
glibc-2.41_1 in transaction breaks installed pkg 'vim-9.2_1'
ERROR: Transaction aborted due to unresolved dependencies.
";
        let blocked = DryRunOutcome {
            items: Vec::new(),
            blockers: parse_breakers(stderr),
        }
        .blocked();
        assert_eq!(blocked[0].needed_by, vec!["bash", "vim"]);
    }

    #[test]
    fn an_install_run_never_reads_blockers() {
        let outcome = read_dry_run(
            DryRun::Install,
            &installed(&[]),
            &["hello".to_owned()],
            INSTALL_HELLO,
            REMOVE_BLOCKED_STDERR,
        )
        .expect("parses");
        assert!(outcome.blockers.is_empty());
    }

    #[test]
    fn a_removal_dry_run_lists_removals_without_a_target_version() {
        let table = "zramen-0.2.4_1 remove x86_64 https://repo-de.voidlinux.org/current 0 848\n";
        let items = read_dry_run(
            DryRun::Remove,
            &installed(&[("zramen", "0.2.4_1", true)]),
            &["zramen".to_owned()],
            table,
            "",
        )
        .expect("parses")
        .items;
        assert_eq!(items[0].action, PlanAction::Remove);
        assert_eq!(items[0].from.as_deref(), Some("0.2.4_1"));
        assert_eq!(items[0].to, None);
        assert_eq!(items[0].size, 0);
        assert_eq!(items[0].reason, PlanReason::Requested);
    }

    #[test]
    fn a_not_installed_removal_is_a_notice_not_an_error() {
        assert!(is_not_installed(
            "Package `zramen' is not currently installed."
        ));
        assert!(!is_not_installed(
            "ERROR: Transaction aborted due to unresolved dependencies."
        ));
    }

    /// Pins the one thing that looks like an error and is not: xbps prefixes
    /// its already-installed notice with `ERROR:` and still exits 0.
    #[test]
    fn already_installed_is_worded_like_an_error_and_is_not_one() {
        let stderr = "ERROR: Package `bash' already installed.";
        assert!(is_already_installed(stderr));
        assert!(!is_not_installed(stderr));
        assert_eq!(missing_package(stderr), None);
    }

    #[test]
    fn the_already_installed_names_are_read_back() {
        let stderr = "ERROR: Package `autofs' already installed.\n\
                      ERROR: Package `libvips' already installed.\n\
                      Package `vim' is not currently installed.";
        assert_eq!(
            already_installed_names(stderr),
            ["autofs".to_owned(), "libvips".to_owned()]
        );
        assert!(already_installed_names("").is_empty());
        assert!(
            already_installed_names("ERROR: Package 'nosuchpkg' not found in repository pool.")
                .is_empty(),
            "a missing package is not an installed one"
        );
    }

    /// A non-zero exit is a notice only when the notice is all xbps said. An
    /// abort carries it too, and reading that as "nothing to do" hides the
    /// failure.
    #[test]
    fn only_a_stderr_of_notices_is_a_notice() {
        assert!(is_only_already_installed(
            "ERROR: Package `bash' already installed."
        ));
        assert!(is_only_already_installed(
            "ERROR: Package `bash' already installed.\nPackage `vim' already installed.\n"
        ));
        assert!(!is_only_already_installed(""));
        assert!(
            !is_only_already_installed(
                "ERROR: Package `mpv' already installed.\n\
                 gst-plugins-base1-1.28.9_1: broken, unresolvable shlib `libtheoradec.so.1'\n\
                 ERROR: Transaction aborted due to unresolved shlibs."
            ),
            "an abort is not a notice"
        );
    }

    #[test]
    fn a_missing_package_is_named() {
        assert_eq!(
            missing_package("ERROR: Package 'nosuchpkg-zzz' not found in repository pool."),
            Some("nosuchpkg-zzz".to_owned())
        );
        assert_eq!(missing_package(""), None);
    }

    /// xbps may write a warning before the answer; a line without the prefix
    /// must not end the search.
    #[test]
    fn a_missing_package_is_found_after_an_unrelated_line() {
        let stderr = "\
WARNING: something else happened
ERROR: Package 'nosuchpkg-zzz' not found in repository pool.
";
        assert_eq!(missing_package(stderr), Some("nosuchpkg-zzz".to_owned()));
    }

    #[test]
    fn a_line_that_is_not_a_refusal_is_skipped() {
        assert_eq!(
            parse_breakers("ERROR: Transaction aborted\nnot a breaker\n"),
            []
        );
    }
}
