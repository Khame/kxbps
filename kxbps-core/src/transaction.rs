// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The privileged transaction: what to run and what its exit code means.
//!
//! One transaction is a sequence of `pkexec /usr/bin/xbps-install -S -y <pkgver>…`
//! and `pkexec /usr/bin/xbps-remove -y <pkgver>…` commands: installations,
//! updates and reinstalls first, removals after. The program and every package
//! are separate argv elements, and no `sudo` ever appears: polkit is the only
//! escalation, and it matches the executable against the action ids in
//! `packaging/io.github.khame.kxbps.policy`.
//!
//! Exit codes are pkexec's contract, not a guess:
//!
//! - `0` the transaction ran and succeeded;
//! - `126` pkexec could not be started by polkit or the authentication dialog
//!   was dismissed;
//! - `127` the authentication dialog was not shown or the user was not
//!   authorized;
//! - anything else is xbps' own exit code, passed through untranslated.

use crate::types::{PlanAction, PlanItem};

/// Where pkexec lives; polkit matches this exact path from the policy file.
pub const PKEXEC: &str = "pkexec";

/// The absolute path of `xbps-install`, as the install action annotates it.
pub const XPBS_INSTALL_PATH: &str = "/usr/bin/xbps-install";

/// The absolute path of `xbps-remove`, as the remove action annotates it.
pub const XPBS_REMOVE_PATH: &str = "/usr/bin/xbps-remove";

/// Where the privileged mirror helper lives; the `mirror` policy action
/// annotates this exact path, and it is the one binary that runs as root.
pub const MIRROR_APPLY_PATH: &str = "/usr/libexec/kxbps-mirror-apply";

/// The repository helper polkit runs as root.
pub const REPO_APPLY_PATH: &str = "/usr/libexec/kxbps-repo-apply";

/// The polkit action that authorizes the repository helper.
pub const REPO_ACTION_ID: &str = "io.github.khame.kxbps.repo";

/// The polkit action id of the mirror helper.
pub const MIRROR_ACTION_ID: &str = "io.github.khame.kxbps.mirror";

/// The absolute path of `xbps-pkgdb`, the tool that pins a package version.
pub const XPBS_PKGDB_PATH: &str = "/usr/bin/xbps-pkgdb";

/// The polkit action that authorizes holding or unholding one package.
pub const HOLD_ACTION_ID: &str = "io.github.khame.kxbps.hold";

/// Which of the two privileged tools a transaction runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transaction {
    /// `xbps-install -y`: install, update or reinstall.
    Install,
    /// `xbps-remove -y`: remove.
    Remove,
}

impl Transaction {
    /// The transaction a uniform plan performs.
    ///
    /// A plan that holds any install, update or reinstall is an install
    /// transaction; only an all-remove plan is a removal, and an empty plan has
    /// nothing to run. [`TransactionPlan`] uses this per group of items, so a
    /// mixed plan becomes one install command and one remove command.
    pub fn for_items(items: &[PlanItem]) -> Option<Self> {
        if items.iter().any(|item| item.action != PlanAction::Remove) {
            Some(Self::Install)
        } else if items.is_empty() {
            None
        } else {
            Some(Self::Remove)
        }
    }

    /// The absolute path polkit authorizes.
    pub fn program(self) -> &'static str {
        match self {
            Self::Install => XPBS_INSTALL_PATH,
            Self::Remove => XPBS_REMOVE_PATH,
        }
    }

    /// The polkit action id the policy file declares for this tool.
    pub fn action_id(self) -> &'static str {
        match self {
            Self::Install => "io.github.khame.kxbps.install",
            Self::Remove => "io.github.khame.kxbps.remove",
        }
    }

    /// The argv list of the transaction, without pkexec.
    ///
    /// The package versions stay one element each even when a name contains a
    /// space or a quote, because there is no shell anywhere in this path.
    ///
    /// An install syncs the repository indexes first (`-S`): the plan and the
    /// update list are read from indexes fetched into memory, so the on-disk
    /// copy may be older than the versions this transaction names, and without
    /// the sync xbps answers "not found in repository pool" for a package the
    /// window just showed. A removal has no index to sync.
    pub fn xbps_argv(self, pkgvers: &[String]) -> Vec<String> {
        let mut list = Vec::with_capacity(pkgvers.len() + 3);
        list.push(self.program().to_owned());
        if self == Self::Install {
            list.push(String::from("-S"));
        }
        list.push(String::from("-y"));
        list.extend(pkgvers.iter().cloned());
        list
    }

    /// The complete argv list, with pkexec in front.
    pub fn argv(self, pkgvers: &[String]) -> Vec<String> {
        let mut list = Vec::with_capacity(pkgvers.len() + 3);
        list.push(String::from(PKEXEC));
        list.extend(self.xbps_argv(pkgvers));
        list
    }

    /// The same list, signed with the stable string `kxbps --action` needs.
    pub fn command_line(self, pkgvers: &[String]) -> String {
        crate::format::command_line(&self.argv(pkgvers))
    }
}

/// What one privileged process may do: run an xbps transaction, or hand one
/// base URL to the mirror helper.
///
/// The two arms share only the shape — an argv with `pkexec` first and a polkit
/// action id — because they answer to different policy actions and different
/// exit codes. Every argv element stays its own argument and no shell is
/// involved, so a URL with a quote or a space can never become a second
/// command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Privileged {
    /// `pkexec /usr/bin/xbps-install -S -y …`: one transaction tool.
    Package(Transaction),
    /// `pkexec /usr/libexec/kxbps-mirror-apply <base>`: one base URL.
    Mirror(String),
    /// `pkexec /usr/libexec/kxbps-repo-apply <verb> <value>`: one repository
    /// file, added or removed.
    Repo { verb: String, value: String },
    /// `pkexec /usr/bin/xbps-pkgdb -m hold|unhold <name>`: one package pinned
    /// to its current version, or released again.
    Hold { name: String, held: bool },
    /// `pkexec /usr/bin/xbps-install -S`: fetch the repository indexes and
    /// write them to disk. The only privileged run with no arguments.
    Sync,
}

impl Privileged {
    /// The argv polkit runs, `pkexec` first.
    ///
    /// The package arm names the tool but no packages: a plan's argv is built
    /// by [`Transaction::argv`] once the items exist, while this value answers
    /// "which process would this be" before that. A mirror is exactly its one
    /// argument.
    pub fn argv(&self) -> Vec<String> {
        match self {
            Self::Package(transaction) => {
                vec![String::from(PKEXEC), String::from(transaction.program())]
            }
            Self::Mirror(base) => vec![
                String::from(PKEXEC),
                String::from(MIRROR_APPLY_PATH),
                base.clone(),
            ],
            Self::Repo { verb, value } => vec![
                String::from(PKEXEC),
                String::from(REPO_APPLY_PATH),
                verb.clone(),
                value.clone(),
            ],
            Self::Hold { name, held } => vec![
                String::from(PKEXEC),
                String::from(XPBS_PKGDB_PATH),
                String::from("-m"),
                String::from(if *held { "hold" } else { "unhold" }),
                name.clone(),
            ],
            Self::Sync => vec![
                String::from(PKEXEC),
                String::from(XPBS_INSTALL_PATH),
                String::from("-S"),
            ],
        }
    }

    /// The base URL of a mirror action; `None` for a package transaction.
    pub fn mirror_base(&self) -> Option<&str> {
        match self {
            Self::Package(_) | Self::Repo { .. } | Self::Hold { .. } | Self::Sync => None,
            Self::Mirror(base) => Some(base),
        }
    }

    /// The polkit action id the policy file declares for this process.
    pub fn action_id(&self) -> &'static str {
        match self {
            Self::Package(transaction) => transaction.action_id(),
            Self::Mirror(_) => MIRROR_ACTION_ID,
            Self::Repo { .. } => REPO_ACTION_ID,
            Self::Hold { .. } => HOLD_ACTION_ID,
            // The index sync writes `/var/db/xbps` with the tool the install
            // action already authorizes, so it needs no action of its own.
            Self::Sync => "io.github.khame.kxbps.install",
        }
    }

    /// The same argv, signed with the stable string the job log shows.
    pub fn command_line(&self) -> String {
        crate::format::command_line(&self.argv())
    }
}

/// The argv of the mirror helper: `pkexec /usr/libexec/kxbps-mirror-apply <base>`.
///
/// The base URL is one element, exactly as the user's selection spells it; the
/// helper validates and normalises it, and a refused URL never reaches a
/// write.
pub fn mirror_argv(base: &str) -> Vec<String> {
    Privileged::Mirror(base.to_owned()).argv()
}

/// The argv of one repository edit.
pub fn repo_argv(verb: &str, value: &str) -> Vec<String> {
    Privileged::Repo {
        verb: verb.to_owned(),
        value: value.to_owned(),
    }
    .argv()
}

/// The argv of one hold change: `pkexec /usr/bin/xbps-pkgdb -m hold|unhold`.
///
/// `held` is the state the package should end in, not the state it is in, so
/// the caller cannot turn a hold into an unhold by toggling twice.
pub fn hold_argv(name: &str, held: bool) -> Vec<String> {
    Privileged::Hold {
        name: name.to_owned(),
        held,
    }
    .argv()
}

/// The argv of an index sync: `pkexec /usr/bin/xbps-install -S`.
///
/// The read paths fetch indexes into memory (`-M`) and write nothing, so a
/// package added to a repository stays invisible to `xbps-query -Rs` until
/// something syncs the disk copy. This is that something; it needs the install
/// action's authorization because it writes `/var/db/xbps`, and it takes no
/// `-y` because there is nothing to confirm.
pub fn sync_argv() -> Vec<String> {
    Privileged::Sync.argv()
}

/// The commands a plan runs, in order: `pkexec xbps-install` first, then
/// `pkexec xbps-remove` for the removals.
///
/// Each command carries only the package versions its own tool owns. A mixed
/// plan is reachable — xbps' own dry run reports `remove` rows for packages an
/// install replaces — so a removal must never be passed to `xbps-install` as an
/// explicit version, which would reinstall what the plan asked to drop.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransactionPlan {
    pub commands: Vec<Vec<String>>,
}

impl TransactionPlan {
    /// `None` for an empty plan, which has nothing to run.
    pub fn for_items(items: &[PlanItem]) -> Option<Self> {
        let installs: Vec<&PlanItem> = items
            .iter()
            .filter(|item| item.action != PlanAction::Remove)
            .collect();
        let removes: Vec<&PlanItem> = items
            .iter()
            .filter(|item| item.action == PlanAction::Remove)
            .collect();
        let mut commands = Vec::with_capacity(2);
        if !installs.is_empty() {
            commands.push(Transaction::Install.argv(&pkgvers_of(&installs)));
        }
        if !removes.is_empty() {
            commands.push(Transaction::Remove.argv(&pkgvers_of(&removes)));
        }
        if commands.is_empty() {
            None
        } else {
            Some(Self { commands })
        }
    }

    /// The complete argv lists, each with pkexec in front, run in this order.
    pub fn commands(&self) -> Vec<Vec<String>> {
        self.commands.clone()
    }
}

/// The package versions a plan hands to xbps.
///
/// An install step carries its target version, a removal the installed one;
/// a step with neither falls back to the bare name, which xbps accepts as a
/// package expression.
pub fn pkgvers(items: &[PlanItem]) -> Vec<String> {
    let borrowed: Vec<&PlanItem> = items.iter().collect();
    pkgvers_of(&borrowed)
}

/// The same list for a group of borrowed items, as the command builder holds
/// them.
fn pkgvers_of(items: &[&PlanItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| {
            let version = match item.action {
                PlanAction::Remove => item.from.as_deref(),
                _ => item.to.as_deref().or(item.from.as_deref()),
            }
            .unwrap_or("");
            if version.is_empty() {
                item.name.clone()
            } else {
                crate::xbps::pkgver(&item.name, version)
            }
        })
        .collect()
}

/// What a finished process means, as the UI reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Exit code 0.
    Success,
    /// Exit code 126: the dialog was dismissed or polkit could not start the
    /// program.
    DialogDismissed,
    /// Exit code 127: the user is not authorized.
    Denied,
    /// Exit code -1: pkexec is missing or not executable.
    PkexecMissing,
    /// Any other code: xbps' own, passed through.
    XbpsFailed(i32),
}

impl Outcome {
    /// Maps a process exit code; the code itself is never changed here.
    pub fn from_exit_code(code: i32) -> Self {
        match code {
            0 => Self::Success,
            126 => Self::DialogDismissed,
            127 => Self::Denied,
            -1 => Self::PkexecMissing,
            other => Self::XbpsFailed(other),
        }
    }

    pub fn success(self) -> bool {
        self == Self::Success
    }

    /// The translation key the job label shows; empty on success.
    pub fn key(self) -> &'static str {
        match self {
            Self::Success => "",
            Self::DialogDismissed => "jobAuthDismissed",
            Self::Denied => "jobAuthDenied",
            Self::PkexecMissing => "jobPkexecMissing",
            Self::XbpsFailed(_) => "jobXbpsFailed",
        }
    }

    /// The arguments for [`Self::key`]: none, or the xbps exit code.
    pub fn args(self) -> Vec<String> {
        match self {
            Self::XbpsFailed(code) => vec![code.to_string()],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PlanReason;

    fn item(name: &str, action: PlanAction, from: Option<&str>, to: Option<&str>) -> PlanItem {
        PlanItem {
            name: name.into(),
            action,
            from: from.map(str::to_string),
            to: to.map(str::to_string),
            reason: PlanReason::Requested,
            download: 0,
            size: 0,
        }
    }

    fn v(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    /// The plan was read from freshly fetched indexes (`-M`), but the
    /// transaction reads the on-disk ones. `-S` syncs them first, so a package
    /// version the plan just showed is not "not found in repository pool" a
    /// second later. A removal has no index to sync and never gets it.
    #[test]
    fn an_install_transaction_syncs_the_indexes_before_it_installs() {
        let install = Transaction::Install.argv(&v(&["vim-9.2.1031_1"]));
        assert_eq!(install[1], "/usr/bin/xbps-install");
        assert_eq!(install[2], "-S");
        let remove = Transaction::Remove.argv(&v(&["tzdata-2025.1_1"]));
        assert_eq!(
            remove,
            ["pkexec", "/usr/bin/xbps-remove", "-y", "tzdata-2025.1_1"]
        );
    }

    #[test]
    fn install_runs_xbps_install_through_pkexec_after_yes() {
        let argv = Transaction::Install.argv(&v(&["vim-9.2.1031_1", "git-2.51.0_1"]));
        assert_eq!(
            argv,
            [
                "pkexec",
                "/usr/bin/xbps-install",
                "-S",
                "-y",
                "vim-9.2.1031_1",
                "git-2.51.0_1"
            ]
        );
    }

    #[test]
    fn remove_runs_xbps_remove_through_pkexec_after_yes() {
        let argv = Transaction::Remove.argv(&v(&["tzdata-2025.1_1"]));
        assert_eq!(
            argv,
            ["pkexec", "/usr/bin/xbps-remove", "-y", "tzdata-2025.1_1"]
        );
    }

    #[test]
    fn no_argv_ever_mentions_sudo() {
        for transaction in [Transaction::Install, Transaction::Remove] {
            let argv = transaction.argv(&v(&["a-1_1", "b-2_1"]));
            assert_eq!(argv.first().map(String::as_str), Some("pkexec"));
            assert!(
                !argv.iter().any(|arg| arg.contains("sudo")),
                "{argv:?} escalates through polkit only"
            );
            assert!(
                argv.iter().all(|arg| !arg.contains(' ')),
                "each package stays its own argument: {argv:?}"
            );
        }
    }

    #[test]
    fn a_package_with_spaces_stays_one_argument() {
        let argv = Transaction::Install.argv(&v(&["my package-1.0_1"]));
        assert_eq!(argv.len(), 5);
        assert_eq!(argv[4], "my package-1.0_1");
    }

    #[test]
    fn the_tool_paths_are_the_ones_the_policy_annotates() {
        assert_eq!(Transaction::Install.program(), "/usr/bin/xbps-install");
        assert_eq!(Transaction::Remove.program(), "/usr/bin/xbps-remove");
        assert_eq!(
            Transaction::Install.action_id(),
            "io.github.khame.kxbps.install"
        );
        assert_eq!(
            Transaction::Remove.action_id(),
            "io.github.khame.kxbps.remove"
        );
    }

    #[test]
    fn a_plan_with_any_install_step_installs_and_an_all_remove_plan_removes() {
        let mixed = [
            item("vim", PlanAction::Install, None, Some("1_1")),
            item("tzdata", PlanAction::Remove, Some("2_1"), None),
        ];
        assert_eq!(
            Transaction::for_items(&mixed),
            Some(Transaction::Install),
            "installs and updates go through xbps-install"
        );
        let updates = [item("linux", PlanAction::Update, Some("1_1"), Some("2_1"))];
        assert_eq!(Transaction::for_items(&updates), Some(Transaction::Install));
        let removes = [item("tzdata", PlanAction::Remove, Some("2_1"), None)];
        assert_eq!(Transaction::for_items(&removes), Some(Transaction::Remove));
        assert_eq!(Transaction::for_items(&[]), None);
    }

    #[test]
    fn pkgvers_use_the_target_version_to_install_and_the_installed_one_to_remove() {
        let items = [
            item("vim", PlanAction::Install, None, Some("9.2.1031_1")),
            item(
                "git",
                PlanAction::Update,
                Some("2.50.0_1"),
                Some("2.51.0_1"),
            ),
            item("tzdata", PlanAction::Remove, Some("2025.1_1"), None),
            item("bash", PlanAction::Reinstall, Some("5.2_1"), Some("5.2_1")),
        ];
        assert_eq!(
            pkgvers(&items),
            v(&[
                "vim-9.2.1031_1",
                "git-2.51.0_1",
                "tzdata-2025.1_1",
                "bash-5.2_1"
            ])
        );
    }

    #[test]
    fn a_step_without_a_version_falls_back_to_its_name() {
        assert_eq!(
            pkgvers(&[item("vim", PlanAction::Install, None, None)]),
            v(&["vim"])
        );
    }

    #[test]
    fn a_transaction_plan_is_the_argv_qml_would_read() {
        let items = [item("vim", PlanAction::Install, None, Some("9.2.1031_1"))];
        let plan = TransactionPlan::for_items(&items).expect("a plan has a transaction");
        assert_eq!(
            plan.commands(),
            [v(&[
                "pkexec",
                "/usr/bin/xbps-install",
                "-S",
                "-y",
                "vim-9.2.1031_1"
            ])]
        );
        assert_eq!(
            Transaction::Install.command_line(&v(&["vim-9.2.1031_1"])),
            "pkexec /usr/bin/xbps-install -S -y vim-9.2.1031_1"
        );
        assert_eq!(TransactionPlan::for_items(&[]), None);
    }

    #[test]
    fn a_mixed_plan_installs_first_and_removes_afterwards() {
        let items = [
            item("vim", PlanAction::Install, None, Some("9.2.1031_1")),
            item("tzdata", PlanAction::Remove, Some("2025.1_1"), None),
            item(
                "git",
                PlanAction::Update,
                Some("2.50.0_1"),
                Some("2.51.0_1"),
            ),
        ];
        let plan = TransactionPlan::for_items(&items).expect("a mixed plan has commands");
        assert_eq!(
            plan.commands(),
            [
                v(&[
                    "pkexec",
                    "/usr/bin/xbps-install",
                    "-S",
                    "-y",
                    "vim-9.2.1031_1",
                    "git-2.51.0_1"
                ]),
                v(&["pkexec", "/usr/bin/xbps-remove", "-y", "tzdata-2025.1_1"]),
            ],
            "installs and updates go to xbps-install, removals to xbps-remove"
        );
        let install = &plan.commands()[0];
        assert!(
            !install.iter().any(|arg| arg == "tzdata-2025.1_1"),
            "a removal never reaches the install argv: {install:?}"
        );
    }

    #[test]
    fn an_all_remove_plan_is_one_remove_command() {
        let items = [
            item("tzdata", PlanAction::Remove, Some("2025.1_1"), None),
            item("nano", PlanAction::Remove, Some("8.0_1"), None),
        ];
        let plan = TransactionPlan::for_items(&items).expect("a removal plan has commands");
        assert_eq!(
            plan.commands(),
            [v(&[
                "pkexec",
                "/usr/bin/xbps-remove",
                "-y",
                "tzdata-2025.1_1",
                "nano-8.0_1"
            ])]
        );
    }

    #[test]
    fn an_all_install_plan_is_one_install_command() {
        let items = [
            item("vim", PlanAction::Install, None, Some("9.2.1031_1")),
            item(
                "git",
                PlanAction::Reinstall,
                Some("2.51.0_1"),
                Some("2.51.0_1"),
            ),
        ];
        let plan = TransactionPlan::for_items(&items).expect("a plan has commands");
        assert_eq!(
            plan.commands(),
            [v(&[
                "pkexec",
                "/usr/bin/xbps-install",
                "-S",
                "-y",
                "vim-9.2.1031_1",
                "git-2.51.0_1"
            ])]
        );
    }

    #[test]
    fn an_empty_plan_has_no_commands() {
        assert_eq!(TransactionPlan::for_items(&[]), None);
        assert!(TransactionPlan::default().commands().is_empty());
    }

    #[test]
    fn a_mirror_is_one_base_url_after_pkexec() {
        let privileged = Privileged::Mirror(String::from("https://repo-default.voidlinux.org/"));
        assert_eq!(
            privileged.argv(),
            [
                "pkexec",
                "/usr/libexec/kxbps-mirror-apply",
                "https://repo-default.voidlinux.org/"
            ],
            "the base URL is one element; nothing re-splits it"
        );
        assert_eq!(
            privileged.argv(),
            mirror_argv("https://repo-default.voidlinux.org/")
        );
        assert_eq!(privileged.action_id(), "io.github.khame.kxbps.mirror");
        assert_eq!(
            privileged.mirror_base(),
            Some("https://repo-default.voidlinux.org/")
        );
        assert_eq!(
            privileged.command_line(),
            "pkexec /usr/libexec/kxbps-mirror-apply https://repo-default.voidlinux.org/"
        );
        assert!(
            !privileged.argv().iter().any(|arg| arg == "sudo"),
            "escalation is polkit only"
        );
    }

    #[test]
    fn a_mirror_url_with_spaces_or_quotes_stays_one_argument() {
        let base = "https://mirror.example/with space/?q=\"x\";rm -rf /";
        let argv = Privileged::Mirror(String::from(base)).argv();
        assert_eq!(argv.len(), 3, "{argv:?}");
        assert_eq!(argv[2], base, "no shell ever sees this string");
    }

    #[test]
    fn a_package_transaction_keeps_its_own_action_and_argv_shape() {
        let privileged = Privileged::Package(Transaction::Install);
        assert_eq!(
            privileged.argv(),
            ["pkexec", "/usr/bin/xbps-install"],
            "the tool first; the packages come from the plan"
        );
        assert_eq!(privileged.action_id(), "io.github.khame.kxbps.install");
        assert_eq!(privileged.mirror_base(), None);
    }

    #[test]
    fn the_mirror_path_and_action_are_the_ones_the_policy_authorizes() {
        assert_eq!(MIRROR_APPLY_PATH, "/usr/libexec/kxbps-mirror-apply");
        assert_eq!(MIRROR_ACTION_ID, "io.github.khame.kxbps.mirror");
        assert_eq!(mirror_argv("https://a/")[1], MIRROR_APPLY_PATH);
    }

    #[test]
    fn exit_codes_map_to_the_documented_outcomes() {
        assert_eq!(Outcome::from_exit_code(0), Outcome::Success);
        assert_eq!(Outcome::from_exit_code(126), Outcome::DialogDismissed);
        assert_eq!(Outcome::from_exit_code(127), Outcome::Denied);
        assert_eq!(Outcome::from_exit_code(-1), Outcome::PkexecMissing);
        assert_eq!(Outcome::from_exit_code(19), Outcome::XbpsFailed(19));
        assert_eq!(Outcome::from_exit_code(1), Outcome::XbpsFailed(1));
    }

    #[test]
    fn only_zero_is_success() {
        assert!(Outcome::from_exit_code(0).success());
        for code in [1, 19, 126, 127, -1] {
            assert!(!Outcome::from_exit_code(code).success(), "{code}");
        }
    }

    #[test]
    fn every_failure_has_a_key_and_the_xbps_code_travels_as_its_argument() {
        assert_eq!(Outcome::Success.key(), "");
        assert!(Outcome::Success.args().is_empty());
        assert_eq!(Outcome::DialogDismissed.key(), "jobAuthDismissed");
        assert!(Outcome::DialogDismissed.args().is_empty());
        assert_eq!(Outcome::Denied.key(), "jobAuthDenied");
        assert_eq!(Outcome::PkexecMissing.key(), "jobPkexecMissing");
        assert_eq!(Outcome::XbpsFailed(19).key(), "jobXbpsFailed");
        assert_eq!(Outcome::XbpsFailed(19).args(), v(&["19"]));
    }

    #[test]
    fn a_hold_is_one_pkgdb_command_in_the_direction_asked() {
        assert_eq!(
            hold_argv("vim", true),
            v(&["pkexec", "/usr/bin/xbps-pkgdb", "-m", "hold", "vim"])
        );
        assert_eq!(
            hold_argv("vim", false),
            v(&["pkexec", "/usr/bin/xbps-pkgdb", "-m", "unhold", "vim"])
        );
    }

    #[test]
    fn a_sync_is_one_install_command_with_no_packages_and_no_yes() {
        assert_eq!(
            sync_argv(),
            v(&["pkexec", "/usr/bin/xbps-install", "-S"]),
            "no -y: there is nothing to confirm, the indexes are the whole job"
        );
    }

    #[test]
    fn the_sync_rides_on_the_install_action() {
        let sync = Privileged::Sync;
        assert_eq!(sync.action_id(), "io.github.khame.kxbps.install");
        assert_eq!(sync.mirror_base(), None);
        assert_eq!(sync.command_line(), "pkexec /usr/bin/xbps-install -S");
    }

    #[test]
    fn the_hold_has_its_own_policy_action() {
        let hold = Privileged::Hold {
            name: String::from("vim"),
            held: true,
        };
        assert_eq!(hold.action_id(), "io.github.khame.kxbps.hold");
        assert_eq!(hold.mirror_base(), None);
    }
}
