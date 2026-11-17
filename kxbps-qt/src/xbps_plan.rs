// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Plans read from xbps dry runs.
//!
//! The Plan dialog shows what `xbps-install -n <names>`, `xbps-install -n -Su`
//! or `xbps-remove -n <names>` prints, so the steps are xbps' own answer
//! instead of a second resolver that could disagree with it. A dry run changes
//! nothing: no lock is taken, nothing is downloaded, no file is written, and it
//! never needs root.
//!
//! This module builds the argv list, runs it through the shared [`Invoke`]
//! transport and turns the answer into a [`PlanPayload`]; the Qt side decides
//! when to call it (worker thread, generation guard) and how it is rendered.
//! Every process call is injected, so the suite never starts xbps.

use crate::xbps_runner::{READ_TIMEOUT_MS, REMOTE_TIMEOUT_MS, XPBS_INSTALL, XPBS_REMOVE, argv};
use crate::xbps_snapshot::{Invoke, SnapshotError};
use kxbps_core::{
    DryRun, InstalledMap, Plan, PlanRequest, QueueItem, XbpsError, is_not_installed,
    missing_package, parse_breakers, parse_updates,
};

use crate::plan_backend::{PlanPayload, keys as plan_keys};

/// Why a plan could not be produced: the i18n key plus its `%1…` arguments.
///
/// The same shape as [`SnapshotError`]: QML owns the sentence, and xbps' own
/// output stays untranslated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// The package is not in any repository (`%1` = name).
    UnknownPackage(String),
    /// An update was requested for a package that is not installed (`%1` =
    /// joined names).
    NotInstalled(Vec<String>),
    /// The command could not run, timed out or exited non-zero for a reason
    /// that is not one of the documented answers.
    Xbps(SnapshotError),
}

impl PlanError {
    /// The translation key QML maps to a sentence.
    pub fn key(&self) -> &'static str {
        match self {
            Self::UnknownPackage(_) => plan_keys::ERROR_UNKNOWN_PACKAGE,
            Self::NotInstalled(_) => plan_keys::ERROR_NOT_INSTALLED,
            Self::Xbps(err) => err.key(),
        }
    }

    /// The arguments of that sentence, in `%1…` order.
    pub fn args(&self) -> Vec<String> {
        match self {
            Self::UnknownPackage(name) => vec![name.clone()],
            Self::NotInstalled(names) => vec![names.join(", ")],
            Self::Xbps(err) => err.args(),
        }
    }
}

/// `xbps-install -M -n <names>`: install or update exactly the named packages.
///
/// `-M` is the same flag the update check reads the list with: it fetches the
/// repository indexes into memory instead of trusting the on-disk copy, which
/// may be days old. Without it the plan describes a different index than the
/// list on screen — a queued update whose version the stale index does not
/// know comes back as `already installed`, and the plan is empty while the
/// window shows updates.
pub fn install_argv(names: &[String]) -> Vec<String> {
    let mut args = vec!["-M", "-n"];
    args.extend(names.iter().map(String::as_str));
    argv(XPBS_INSTALL, &args)
}

/// `xbps-install -M -n -Su`: every update the repositories hold.
pub fn system_argv() -> Vec<String> {
    argv(XPBS_INSTALL, &["-M", "-n", "-Su"])
}

/// The timeout a dry run needs: a fetching run reads the network, the rest only
/// the local database.
fn dry_run_timeout(command: &[String]) -> u32 {
    if command.iter().any(|arg| arg == "-M") {
        REMOTE_TIMEOUT_MS
    } else {
        READ_TIMEOUT_MS
    }
}

/// `xbps-remove -n <names>`: remove exactly the named packages.
pub fn remove_argv(names: &[String]) -> Vec<String> {
    let mut args = vec!["-n"];
    args.extend(names.iter().map(String::as_str));
    argv(XPBS_REMOVE, &args)
}

/// The queue split into the two dry runs it needs.
///
/// Installs and updates go to one `xbps-install -n`, removals to one
/// `xbps-remove -n`; a queue with only removals never runs the install half.
pub fn queue_requests(queue: &[QueueItem]) -> (PlanRequest, PlanRequest) {
    (
        PlanRequest::install(kxbps_core::install_names(queue)),
        PlanRequest::remove(kxbps_core::remove_names(queue)),
    )
}

/// What one dry run answered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DryOutcome {
    /// The steps, in xbps' order.
    pub items: Vec<kxbps_core::PlanItem>,
    /// Packages xbps refused to remove, for the Blocked dialog.
    pub blocked: Vec<kxbps_core::Blocked>,
    /// `true` when the run printed no rows and exited 0.
    pub empty: bool,
    /// Names xbps called already installed: the queue entry has nothing to do.
    pub satisfied: Vec<String>,
}

impl DryOutcome {
    /// The steps and the blockers as one plan.
    pub fn plan(&self) -> Plan {
        Plan {
            items: self.items.clone(),
            blocked: self.blocked.clone(),
        }
    }
}

/// Runs one dry run through `invoke` and parses its answer.
///
/// The answers, in the order xbps produces them:
///
/// - exit **19** on a removal, with `<pkgver> in transaction breaks installed
///   pkg '<name>'` lines on stderr: the transaction is blocked and lists what
///   still needs the package. This is the only source of `blocked`;
/// - exit **2** with ``Package `x' not found``: a real failure;
/// - exit **2** with ``Package `x' is not currently installed``: nothing to
///   remove, a notice;
/// - exit **0** with ``ERROR: Package `x' already installed.`` on stderr: the
///   transaction is empty, a notice, never a failure;
/// - exit **0** with no rows: empty, a notice.
pub fn dry_run(
    request: &PlanRequest,
    installed: &InstalledMap,
    invoke: &Invoke,
) -> Result<DryOutcome, PlanError> {
    let Some(mode) = request.dry_run() else {
        return Ok(DryOutcome {
            empty: true,
            ..DryOutcome::default()
        });
    };
    let command = match mode {
        DryRun::Install => install_argv(&request.names),
        DryRun::System => system_argv(),
        DryRun::Remove => remove_argv(&request.names),
    };
    let program = command.first().cloned().unwrap_or_default();
    let output =
        crate::xbps_runner::run(&command, dry_run_timeout(&command), invoke).map_err(|err| {
            PlanError::Xbps(SnapshotError::Reply {
                reason: err.to_string(),
            })
        })?;

    if output.timed_out {
        return Err(PlanError::Xbps(SnapshotError::Timeout { program }));
    }
    if output.exit_code == -1 {
        return Err(PlanError::Xbps(SnapshotError::Start {
            program,
            reason: output.stderr.trim().to_owned(),
        }));
    }

    match output.exit_code {
        0 => {
            let entries = parse_table(program.clone(), &output.stdout)?;
            let items = kxbps_core::items_from(mode, installed, &request.names, &entries);
            Ok(DryOutcome {
                empty: entries.is_empty(),
                satisfied: if mode.removes() {
                    Vec::new()
                } else {
                    satisfied_names(&output.stderr, &items)
                },
                items,
                blocked: Vec::new(),
            })
        }
        kxbps_core::EXIT_UNRESOLVED if mode.removes() => {
            let blocked = kxbps_core::DryRunOutcome {
                items: Vec::new(),
                blockers: parse_breakers(&output.stderr),
            }
            .blocked();
            if blocked.is_empty() {
                return Err(PlanError::Xbps(SnapshotError::Exit {
                    program,
                    code: output.exit_code,
                    stderr: output.stderr.trim().to_owned(),
                }));
            }
            Ok(DryOutcome {
                items: Vec::new(),
                blocked,
                empty: false,
                satisfied: Vec::new(),
            })
        }
        kxbps_core::EXIT_NOT_INSTALLED => {
            if let Some(name) = missing_package(&output.stderr) {
                return Err(PlanError::UnknownPackage(name));
            }
            if is_not_installed(&output.stderr) {
                return Ok(DryOutcome {
                    empty: true,
                    ..DryOutcome::default()
                });
            }
            Err(PlanError::Xbps(SnapshotError::Exit {
                program,
                code: output.exit_code,
                stderr: output.stderr.trim().to_owned(),
            }))
        }
        code => {
            // The `already installed` notice is worded like an error but is
            // not one; a build that exits non-zero for it is still a notice. An
            // aborted transaction carries the same sentence, so only a stderr
            // that says nothing else counts: anything more is a failure that
            // must not be reported as "nothing to do".
            if kxbps_core::is_only_already_installed(&output.stderr) {
                return Ok(DryOutcome {
                    empty: true,
                    satisfied: satisfied_names(&output.stderr, &[]),
                    ..DryOutcome::default()
                });
            }
            Err(PlanError::Xbps(SnapshotError::Exit {
                program,
                code,
                stderr: output.stderr.trim().to_owned(),
            }))
        }
    }
}

/// The names xbps called already installed that the table does not carry.
///
/// `xbps-install -M -n a b` prints ``Package `b' already installed.`` when `b`
/// entered the transaction as a dependency of `a`, and prints `b` on the table
/// as an `update` row at the same time: the sentence means "already in this
/// transaction", not "nothing to do". Only a name the plan leaves out has
/// nothing left to do, so only that one may leave the queue.
fn satisfied_names(stderr: &str, items: &[kxbps_core::PlanItem]) -> Vec<String> {
    kxbps_core::already_installed_names(stderr)
        .into_iter()
        .filter(|name| !items.iter().any(|item| &item.name == name))
        .collect()
}

/// Parses the plan table, logging the unreadable line the way the read layer
/// does.
fn parse_table(program: String, stdout: &str) -> Result<Vec<kxbps_core::UpdateEntry>, PlanError> {
    parse_updates(stdout).map_err(|err| match err {
        XbpsError::MalformedLine { line, text, .. } => {
            eprintln!("kxbps: {program} line {line} was not understood: {text}");
            PlanError::Xbps(SnapshotError::Output {
                program: program.clone(),
                line,
                text,
            })
        }
        // An empty table is a valid empty list, so this arm is unreachable;
        // mapping it keeps every `?` honest.
        XbpsError::Empty => PlanError::Xbps(SnapshotError::Empty { program }),
    })
}

/// Plan payload for the transaction queue.
///
/// Installs and updates come from one `xbps-install -n`, removals from one
/// `xbps-remove -n`; a blocked removal wins and discards the steps, exactly
/// like the dialog refuses to apply anything it may not.
pub fn queue_payload(
    installed: &InstalledMap,
    queue: &[QueueItem],
    invoke: &Invoke,
) -> PlanPayload {
    if queue.is_empty() {
        return PlanPayload::notice(plan_keys::NOTICE_EMPTY_QUEUE);
    }
    let (install, remove) = queue_requests(queue);
    let mut items = Vec::new();
    let mut satisfied = Vec::new();
    if !install.names.is_empty() {
        let install = precheck(&install);
        match dry_run(&install, installed, invoke) {
            Ok(outcome) => {
                satisfied = outcome.satisfied;
                if !outcome.empty {
                    items.extend(outcome.items);
                }
            }
            Err(err) => return PlanPayload::from_error(&err),
        }
    }
    if !remove.names.is_empty() {
        match dry_run(&remove, installed, invoke) {
            Ok(outcome) if !outcome.blocked.is_empty() => {
                return PlanPayload::from_plan(
                    outcome.plan(),
                    plan_keys::PLAN_QUEUE,
                    Vec::new(),
                    remove.names,
                );
            }
            Ok(outcome) => items.extend(outcome.items),
            Err(err) => return PlanPayload::from_error(&err),
        }
    }
    if items.is_empty() {
        let mut payload = PlanPayload::notice(plan_keys::NOTICE_EMPTY_QUEUE);
        payload.satisfied = satisfied;
        return payload;
    }
    let mut payload = PlanPayload::from_plan(
        Plan {
            items,
            blocked: Vec::new(),
        },
        plan_keys::PLAN_QUEUE,
        Vec::new(),
        Vec::new(),
    );
    payload.satisfied = satisfied;
    payload
}

/// Plan payload for the system upgrade: `xbps-install -n -Su`.
pub fn system_payload(installed: &InstalledMap, invoke: &Invoke) -> PlanPayload {
    match dry_run(&PlanRequest::system(), installed, invoke) {
        Ok(outcome) if outcome.empty => PlanPayload::notice(plan_keys::NOTICE_EMPTY_SYSTEM),
        Ok(outcome) => PlanPayload::from_plan(
            outcome.plan(),
            plan_keys::PLAN_SYSTEM,
            Vec::new(),
            Vec::new(),
        ),
        Err(err) => PlanPayload::from_error(&err),
    }
}

/// Plan payload for "install exactly these packages".
pub fn named_payload(
    installed: &InstalledMap,
    names: &[String],
    title_key: &str,
    invoke: &Invoke,
) -> PlanPayload {
    let request = precheck(&PlanRequest::install(names.to_vec()));
    match dry_run(&request, installed, invoke) {
        Ok(outcome) if outcome.empty => PlanPayload::notice(plan_keys::NOTICE_EMPTY_INSTALL),
        Ok(outcome) => {
            PlanPayload::from_plan(outcome.plan(), title_key, Vec::new(), names.to_vec())
        }
        Err(err) => PlanPayload::from_error(&err),
    }
}

/// Plan payload for a removal, blocked list included.
///
/// `cascade` only changes the dialog's title: `xbps-remove -n` takes the whole
/// list the user approved, and the dependents are whatever xbps refuses.
pub fn remove_payload(
    installed: &InstalledMap,
    names: &[String],
    cascade: bool,
    invoke: &Invoke,
) -> PlanPayload {
    let title_key = if cascade {
        plan_keys::PLAN_CASCADE
    } else {
        plan_keys::PLAN_REMOVE
    };
    let title_args = vec![names.join(" ")];
    match dry_run(&PlanRequest::remove(names.to_vec()), installed, invoke) {
        Ok(outcome) if !outcome.blocked.is_empty() => {
            PlanPayload::from_plan(outcome.plan(), title_key, title_args, names.to_vec())
        }
        Ok(outcome) if outcome.empty => PlanPayload::notice(plan_keys::NOTICE_EMPTY_REMOVE),
        Ok(outcome) => {
            PlanPayload::from_plan(outcome.plan(), title_key, title_args, names.to_vec())
        }
        Err(err) => PlanPayload::from_error(&err),
    }
}

/// Keeps the first mention of each name and its position.
///
/// A queue may hold the same package twice without the two entries being
/// adjacent; xbps is idempotent about it, but asking once keeps the argv and
/// the plan free of duplicates.
fn precheck(request: &PlanRequest) -> PlanRequest {
    let mut request = request.clone();
    let mut seen = std::collections::HashSet::new();
    request.names.retain(|name| seen.insert(name.clone()));
    request
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xbps_snapshot::SharedInvoke;
    use kxbps_core::{Inst, PlanAction, PlanReason, QueueAction};
    use std::sync::{Arc, Mutex};

    fn inst(version: &str, explicit: bool) -> Inst {
        Inst {
            version: version.to_owned(),
            explicit,
            held: false,
            at: 0,
        }
    }

    fn installed(entries: &[(&str, &str)]) -> InstalledMap {
        entries
            .iter()
            .map(|(name, version)| ((*name).to_owned(), inst(version, true)))
            .collect()
    }

    fn reply(exit_code: i32, stdout: &str, stderr: &str) -> String {
        serde_json::json!({
            "exitCode": exit_code,
            "stdout": stdout,
            "stderr": stderr,
            "timedOut": false,
        })
        .to_string()
    }

    /// A transport answering one canned reply, remembering every argv.
    fn one_answer(reply: &str) -> (SharedInvoke, Arc<Mutex<Vec<Vec<String>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        let reply = reply.to_owned();
        let invoke: SharedInvoke = Arc::new(move |request: &str, _: u32| {
            let argv: Vec<String> = serde_json::from_str(request).expect("request decodes");
            recorder.lock().map(|mut seen| seen.push(argv)).ok();
            reply.clone()
        });
        (invoke, seen)
    }

    /// The update list is read with `-M`, which fetches the indexes into
    /// memory. A plan read from the on-disk index would then describe a
    /// different, older set: every queued update comes back as "already
    /// installed" and the plan is empty while the list shows updates. The dry
    /// runs must ask the same index the check asked.
    #[test]
    fn the_install_dry_runs_read_the_index_the_update_check_reads() {
        let names = vec!["hello".to_owned(), "vim".to_owned()];
        assert_eq!(
            install_argv(&names),
            ["xbps-install", "-M", "-n", "hello", "vim"]
        );
        assert_eq!(system_argv(), ["xbps-install", "-M", "-n", "-Su"]);
        assert_eq!(remove_argv(&names), ["xbps-remove", "-n", "hello", "vim"]);
    }

    /// A dry run that fetches the indexes is a network read, so it gets the
    /// network timeout instead of the local one.
    #[test]
    fn a_fetching_dry_run_uses_the_remote_timeout() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = std::sync::Arc::clone(&seen);
        let invoke: SharedInvoke = std::sync::Arc::new(move |request: &str, timeout: u32| {
            recorder.lock().map(|mut seen| seen.push(timeout)).ok();
            let _ = request;
            reply(0, "", "")
        });
        let names = vec!["hello".to_owned()];
        dry_run(
            &PlanRequest::install(names.clone()),
            &installed(&[]),
            &*invoke,
        )
        .expect("plans");
        dry_run(&PlanRequest::remove(names), &installed(&[]), &*invoke).expect("plans");
        let seen = seen.lock().expect("recorded");
        assert_eq!(seen[0], crate::xbps_runner::REMOTE_TIMEOUT_MS);
        assert_eq!(seen[1], READ_TIMEOUT_MS);
    }

    #[test]
    fn the_dry_run_argv_lists_never_ask_for_root() {
        let names = vec!["hello".to_owned(), "vim".to_owned()];
        assert_eq!(
            install_argv(&names),
            ["xbps-install", "-M", "-n", "hello", "vim"]
        );
        assert_eq!(system_argv(), ["xbps-install", "-M", "-n", "-Su"]);
        assert_eq!(remove_argv(&names), ["xbps-remove", "-n", "hello", "vim"]);
        for argv in [install_argv(&names), system_argv(), remove_argv(&names)] {
            assert!(
                !argv
                    .iter()
                    .any(|arg| arg.contains("sudo") || arg.contains("pkexec")),
                "a dry run is never privileged: {argv:?}"
            );
        }
    }

    #[test]
    fn an_install_plan_comes_from_the_dry_run_table() {
        let (invoke, seen) = one_answer(&reply(
            0,
            "hello-2.12.3_1 install x86_64 https://repo-de.voidlinux.org/current 220488 73301\n",
            "",
        ));
        let request = PlanRequest::install(vec!["hello".to_owned()]);
        let outcome = dry_run(&request, &installed(&[]), &*invoke).expect("plans");
        assert!(!outcome.empty);
        assert_eq!(outcome.items.len(), 1);
        assert_eq!(outcome.items[0].name, "hello");
        assert_eq!(outcome.items[0].action, PlanAction::Install);
        assert_eq!(outcome.items[0].download, 73301);
        assert_eq!(
            seen.lock().map(|seen| seen[0].clone()).unwrap(),
            install_argv(&request.names)
        );
    }

    /// The one case that reads like a failure and is not: xbps exits 0 and
    /// prints `ERROR: Package `bash' already installed.`
    #[test]
    fn already_installed_is_an_empty_plan_not_a_failure() {
        let (invoke, _) = one_answer(&reply(0, "", "ERROR: Package `bash' already installed."));
        let request = PlanRequest::install(vec!["bash".to_owned()]);
        let outcome = dry_run(&request, &installed(&[("bash", "5.2.037_1")]), &*invoke)
            .expect("an installed package is not a failure");
        assert!(outcome.empty);
        assert!(outcome.items.is_empty());
        assert!(outcome.blocked.is_empty());
    }

    /// xbps says `already installed` for a name it has already put in the
    /// transaction as a dependency of an earlier argument, and prints the same
    /// name as an `update` row on the table:
    ///
    /// ```text
    /// $ xbps-install -M -n fribidi fribidi-devel
    /// ERROR: Package `fribidi-devel' already installed.
    /// fribidi-devel-1.0.18_1 update …      <- the table carries it anyway
    /// ```
    ///
    /// Such a name is not satisfied: dropping it from the queue leaves the
    /// transaction short of a package the plan installs.
    #[test]
    fn a_name_the_table_carries_is_not_satisfied() {
        let (invoke, _) = one_answer(&reply(
            0,
            "fribidi-devel-1.0.18_1 update x86_64 https://r/current 148027 23328\n\
             fribidi-1.0.18_1 update x86_64 https://r/current 159864 33456\n",
            "ERROR: Package `fribidi-devel' already installed.",
        ));
        let installed = installed(&[("fribidi", "1.0.16_1"), ("fribidi-devel", "1.0.16_1")]);
        let request = PlanRequest::install(vec!["fribidi".to_owned(), "fribidi-devel".to_owned()]);
        let outcome = dry_run(&request, &installed, &*invoke).expect("plans");
        assert_eq!(outcome.items.len(), 2, "both rows are the transaction");
        assert!(
            outcome.satisfied.is_empty(),
            "a name on the table has something to do: {:?}",
            outcome.satisfied
        );
    }

    /// An aborted transaction is a failure even when it carries the notice:
    /// xbps exits 8 with `Transaction aborted due to unresolved shlibs` and
    /// still names the package it did not have to add again. Reading that as
    /// "nothing to do" hides the failure and drops the queue entry.
    #[test]
    fn an_aborted_transaction_is_a_failure_not_a_notice() {
        let (invoke, _) = one_answer(&reply(
            8,
            "",
            "ERROR: Package `mpv' already installed.\n\
             gst-plugins-base1-1.28.9_1: broken, unresolvable shlib `libtheoradec.so.1'\n\
             ERROR: Transaction aborted due to unresolved shlibs.",
        ));
        let request = PlanRequest::install(vec!["mpv".to_owned()]);
        let error = dry_run(&request, &installed(&[("mpv", "0.42.0_1")]), &*invoke)
            .expect_err("an abort is a failure");
        assert_eq!(error.key(), "errorXbpsExit");
    }

    /// A build that exits non-zero for the notice alone is still a notice.
    #[test]
    fn a_non_zero_exit_that_says_nothing_else_is_still_a_notice() {
        let (invoke, _) = one_answer(&reply(1, "", "ERROR: Package `bash' already installed."));
        let request = PlanRequest::install(vec!["bash".to_owned()]);
        let outcome = dry_run(&request, &installed(&[("bash", "5.2.037_1")]), &*invoke)
            .expect("the notice is the whole answer");
        assert!(outcome.empty);
        assert_eq!(outcome.satisfied, ["bash".to_owned()]);
    }

    /// The queue's Apply drops the names the plan called satisfied. A mixed
    /// queue keeps the steps and only the name xbps left out of the table is
    /// dropped.
    #[test]
    fn a_mixed_queue_keeps_its_steps_and_drops_only_the_left_out_name() {
        let (invoke, _) = one_answer(&reply(
            0,
            "vim-9.2.1031_1 update x86_64 https://r/current 4194304 1468006\n",
            "ERROR: Package `bash' already installed.",
        ));
        let queue = vec![
            QueueItem {
                name: "vim".into(),
                action: QueueAction::Update,
            },
            QueueItem {
                name: "bash".into(),
                action: QueueAction::Update,
            },
        ];
        let payload = queue_payload(
            &installed(&[("vim", "9.2.1000_1"), ("bash", "5.2.037_1")]),
            &queue,
            &*invoke,
        );
        assert_eq!(payload.items.len(), 1);
        assert_eq!(payload.items[0].name, "vim");
        assert_eq!(payload.satisfied, ["bash".to_owned()]);
    }

    #[test]
    fn a_missing_package_is_an_error_with_its_name() {
        let (invoke, _) = one_answer(&reply(
            2,
            "",
            "ERROR: Package 'nosuchpkg' not found in repository pool.",
        ));
        let request = PlanRequest::install(vec!["nosuchpkg".to_owned()]);
        let error = dry_run(&request, &installed(&[]), &*invoke).expect_err("nothing to plan");
        assert_eq!(error.key(), "errorUnknownPackage");
        assert_eq!(error.args(), vec!["nosuchpkg".to_owned()]);
    }

    /// Blocked comes from the removal dry run's exit code and stderr, and from
    /// nowhere else.
    #[test]
    fn a_blocked_removal_lists_the_dependents_and_offers_no_steps() {
        let (invoke, _) = one_answer(&reply(
            19,
            "",
            "acpid-2.0.34_4 in transaction breaks installed pkg 'base-system-0.114_2'\n\
             ERROR: Transaction aborted due to unresolved dependencies.\n",
        ));
        let request = PlanRequest::remove(vec!["acpid".to_owned()]);
        let outcome = dry_run(&request, &installed(&[("acpid", "2.0.34_4")]), &*invoke)
            .expect("a refusal is an answer");
        assert!(outcome.items.is_empty());
        assert_eq!(outcome.blocked.len(), 1);
        assert_eq!(outcome.blocked[0].name, "acpid");
        assert_eq!(outcome.blocked[0].needed_by, vec!["base-system".to_owned()]);
    }

    #[test]
    fn a_removal_of_an_uninstalled_package_is_an_empty_plan() {
        let (invoke, _) = one_answer(&reply(
            2,
            "",
            "Package `zramen' is not currently installed.",
        ));
        let outcome = dry_run(
            &PlanRequest::remove(vec!["zramen".to_owned()]),
            &installed(&[]),
            &*invoke,
        )
        .expect("a notice, not a failure");
        assert!(outcome.empty);
    }

    #[test]
    fn an_up_to_date_system_is_an_empty_plan() {
        let (invoke, seen) = one_answer(&reply(0, "", ""));
        let outcome = dry_run(&PlanRequest::system(), &installed(&[]), &*invoke).expect("empty");
        assert!(outcome.empty);
        assert_eq!(
            seen.lock().map(|seen| seen[0].clone()).unwrap(),
            ["xbps-install", "-M", "-n", "-Su"]
        );
    }

    #[test]
    fn a_missing_xbps_is_a_start_error() {
        let (invoke, _) = one_answer(&reply(-1, "", "No such file or directory"));
        let error =
            dry_run(&PlanRequest::system(), &installed(&[]), &*invoke).expect_err("xbps is gone");
        assert_eq!(error.key(), "errorXbpsStart");
        assert_eq!(error.args()[0], "xbps-install");
    }

    #[test]
    fn a_timeout_is_reported_as_one() {
        let (invoke, _) = one_answer(r#"{"exitCode":-1,"stdout":"","stderr":"","timedOut":true}"#);
        let error =
            dry_run(&PlanRequest::system(), &installed(&[]), &*invoke).expect_err("timed out");
        assert_eq!(error.key(), "errorXbpsTimeout");
    }

    #[test]
    fn an_unparsable_line_is_an_output_error() {
        let (invoke, _) = one_answer(&reply(0, "this is not a plan line\n", ""));
        let error =
            dry_run(&PlanRequest::system(), &installed(&[]), &*invoke).expect_err("not a plan");
        assert_eq!(error.key(), "errorXbpsOutput");
    }

    #[test]
    fn a_queue_splits_into_one_install_and_one_remove_run() {
        let queue = vec![
            QueueItem {
                name: "vim".into(),
                action: QueueAction::Install,
            },
            QueueItem {
                name: "git".into(),
                action: QueueAction::Update,
            },
            QueueItem {
                name: "zramen".into(),
                action: QueueAction::Remove,
            },
        ];
        let (install, remove) = queue_requests(&queue);
        assert_eq!(install.names, ["vim", "git"]);
        assert_eq!(remove.names, ["zramen"]);
    }

    #[test]
    fn a_reason_column_separates_requested_from_dependencies() {
        let (invoke, _) = one_answer(&reply(
            0,
            "bash-5.2.037_1 install x86_64 https://r/current 8100000 1800000\n\
             vim-9.2.1031_1 install x86_64 https://r/current 4194304 1468006\n",
            "",
        ));
        let request = PlanRequest::install(vec!["vim".to_owned()]);
        let outcome = dry_run(&request, &installed(&[]), &*invoke).expect("plans");
        assert_eq!(outcome.items[0].reason, PlanReason::Dependency);
        assert_eq!(outcome.items[1].reason, PlanReason::Requested);
    }
}
