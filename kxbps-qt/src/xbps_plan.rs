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

use crate::xbps_runner::{READ_TIMEOUT_MS, XPBS_INSTALL, XPBS_REMOVE, argv};
use crate::xbps_snapshot::{Invoke, SnapshotError};
use kxbps_core::{
    Catalog, DryRun, InstalledMap, Plan, PlanRequest, QueueItem, Repos, XbpsError,
    is_already_installed, is_not_installed, missing_package, parse_breakers, parse_updates,
};

use crate::plan_backend::{PlanPayload, keys as plan_keys, repo_key};

/// Why a plan could not be produced: the i18n key plus its `%1…` arguments.
///
/// The same shape as [`SnapshotError`]: QML owns the sentence, and xbps' own
/// output stays untranslated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// The package is not in any repository (`%1` = name).
    UnknownPackage(String),
    /// The package only exists in a repository the user left disabled
    /// (`%1` = name, `%2` = repo key).
    RepoDisabled { package: String, repo: String },
    /// A dependency only exists in a disabled repository (`%1` = name,
    /// `%2` = dependency, `%3` = repo key).
    DependencyRepoDisabled {
        package: String,
        dependency: String,
        repo: String,
    },
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
            Self::RepoDisabled { .. } => plan_keys::ERROR_REPO_DISABLED,
            Self::DependencyRepoDisabled { .. } => plan_keys::ERROR_DEPENDENCY_REPO_DISABLED,
            Self::NotInstalled(_) => plan_keys::ERROR_NOT_INSTALLED,
            Self::Xbps(err) => err.key(),
        }
    }

    /// The arguments of that sentence, in `%1…` order.
    pub fn args(&self) -> Vec<String> {
        match self {
            Self::UnknownPackage(name) => vec![name.clone()],
            Self::RepoDisabled { package, repo } => vec![package.clone(), repo.clone()],
            Self::DependencyRepoDisabled {
                package,
                dependency,
                repo,
            } => vec![package.clone(), dependency.clone(), repo.clone()],
            Self::NotInstalled(names) => vec![names.join(", ")],
            Self::Xbps(err) => err.args(),
        }
    }
}

/// `xbps-install -n <names>`: install or update exactly the named packages.
pub fn install_argv(names: &[String]) -> Vec<String> {
    let mut args = vec!["-n"];
    args.extend(names.iter().map(String::as_str));
    argv(XPBS_INSTALL, &args)
}

/// `xbps-install -n -Su`: every update the repositories hold.
pub fn system_argv() -> Vec<String> {
    argv(XPBS_INSTALL, &["-n", "-Su"])
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
    let output = crate::xbps_runner::run(&command, READ_TIMEOUT_MS, invoke).map_err(|err| {
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
            Ok(DryOutcome {
                empty: entries.is_empty(),
                items: kxbps_core::items_from(mode, installed, &request.names, &entries),
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
            // not one; a build that exits non-zero for it is still a notice.
            if is_already_installed(&output.stderr) {
                return Ok(DryOutcome {
                    empty: true,
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
    repos: &Repos,
    catalog: &Catalog,
    queue: &[QueueItem],
    invoke: &Invoke,
) -> PlanPayload {
    if queue.is_empty() {
        return PlanPayload::notice(plan_keys::NOTICE_EMPTY_QUEUE);
    }
    let (install, remove) = queue_requests(queue);
    let mut items = Vec::new();
    if !install.names.is_empty() {
        let install = match precheck(&install, installed, repos, catalog) {
            Ok(request) => match dry_run(&request, installed, invoke) {
                Ok(outcome) => outcome,
                Err(err) => return PlanPayload::from_error(&err),
            },
            Err(err) => return PlanPayload::from_error(&err),
        };
        if !install.empty {
            items.extend(install.items);
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
        return PlanPayload::notice(plan_keys::NOTICE_EMPTY_QUEUE);
    }
    PlanPayload::from_plan(
        Plan {
            items,
            blocked: Vec::new(),
        },
        plan_keys::PLAN_QUEUE,
        Vec::new(),
        Vec::new(),
    )
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
    repos: &Repos,
    catalog: &Catalog,
    names: &[String],
    title_key: &str,
    invoke: &Invoke,
) -> PlanPayload {
    let request = PlanRequest::install(names.to_vec());
    let request = match precheck(&request, installed, repos, catalog) {
        Ok(request) => request,
        Err(err) => return PlanPayload::from_error(&err),
    };
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

/// The repository checks that run before the dry run.
///
/// xbps only knows its own configuration. A package that exists solely in a
/// repository the user switched off would fail as "not found", so the catalog
/// answers first and the dry run is what decides everything else.
fn precheck(
    request: &PlanRequest,
    _installed: &InstalledMap,
    repos: &Repos,
    catalog: &Catalog,
) -> Result<PlanRequest, PlanError> {
    let mut request = request.clone();
    for name in &request.names {
        if let Some(repo) = catalog.disabled_repo_of(name, repos) {
            return Err(PlanError::RepoDisabled {
                package: name.clone(),
                repo: repo_key(repo).to_owned(),
            });
        }
        if let Some((dependency, repo)) = catalog.disabled_dependency_of(name, repos) {
            return Err(PlanError::DependencyRepoDisabled {
                package: name.clone(),
                dependency,
                repo: repo_key(repo).to_owned(),
            });
        }
    }
    request.names.dedup();
    Ok(request)
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

    #[test]
    fn the_dry_run_argv_lists_never_ask_for_root() {
        let names = vec!["hello".to_owned(), "vim".to_owned()];
        assert_eq!(install_argv(&names), ["xbps-install", "-n", "hello", "vim"]);
        assert_eq!(system_argv(), ["xbps-install", "-n", "-Su"]);
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
        assert_eq!(
            outcome.blocked[0].needed_by,
            vec!["base-system-0.114_2".to_owned()]
        );
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
            ["xbps-install", "-n", "-Su"]
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
