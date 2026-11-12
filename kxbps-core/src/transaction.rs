//! The privileged transaction: what to run and what its exit code means.
//!
//! One transaction is `pkexec /usr/bin/xbps-install -y <pkgver>…` or
//! `pkexec /usr/bin/xbps-remove -y <pkgver>…`. The program and every package
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

/// Which of the two privileged tools a transaction runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transaction {
    /// `xbps-install -y`: install, update or reinstall.
    Install,
    /// `xbps-remove -y`: remove.
    Remove,
}

impl Transaction {
    /// The transaction a plan performs.
    ///
    /// A plan that holds any install, update or reinstall is one transaction:
    /// `xbps-install` also handles the removals a plan never mixes in there.
    /// Only an all-remove plan goes through `xbps-remove`, otherwise there is
    /// nothing to run.
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
    pub fn xbps_argv(self, pkgvers: &[String]) -> Vec<String> {
        let mut list = Vec::with_capacity(pkgvers.len() + 2);
        list.push(self.program().to_owned());
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
        self.argv(pkgvers).join(" ")
    }
}

/// The package versions a plan hands to xbps.
///
/// An install step carries its target version, a removal the installed one;
/// a step with neither falls back to the bare name, which xbps accepts as a
/// package expression.
pub fn pkgvers(items: &[PlanItem]) -> Vec<String> {
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
                format!("{}-{version}", item.name)
            }
        })
        .collect()
}

/// Everything the argv list of a plan needs, computed once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionPlan {
    pub transaction: Transaction,
    pub pkgvers: Vec<String>,
}

impl TransactionPlan {
    /// `None` for an empty plan, which has nothing to run.
    pub fn for_items(items: &[PlanItem]) -> Option<Self> {
        Transaction::for_items(items).map(|transaction| Self {
            transaction,
            pkgvers: pkgvers(items),
        })
    }

    /// `pkexec /usr/bin/xbps-install -y <pkgver>…`.
    pub fn argv(&self) -> Vec<String> {
        self.transaction.argv(&self.pkgvers)
    }
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

    #[test]
    fn install_runs_xbps_install_through_pkexec_after_yes() {
        let argv = Transaction::Install.argv(&v(&["vim-9.2.1031_1", "git-2.51.0_1"]));
        assert_eq!(
            argv,
            [
                "pkexec",
                "/usr/bin/xbps-install",
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
        assert_eq!(argv.len(), 4);
        assert_eq!(argv[3], "my package-1.0_1");
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
        assert_eq!(plan.transaction, Transaction::Install);
        assert_eq!(
            plan.argv(),
            ["pkexec", "/usr/bin/xbps-install", "-y", "vim-9.2.1031_1"]
        );
        assert_eq!(
            plan.transaction.command_line(&plan.pkgvers),
            "pkexec /usr/bin/xbps-install -y vim-9.2.1031_1"
        );
        assert_eq!(TransactionPlan::for_items(&[]), None);
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
}
