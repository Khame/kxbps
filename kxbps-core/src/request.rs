// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! What should be planned, as one dry run asks for it.
//!
//! The Qt layer builds a [`PlanRequest`], runs it and parses the answer; the
//! core only names the shapes so both sides agree on what a request is. The
//! argv lists themselves live next to the process runner.

use crate::types::{InstalledMap, QueueAction, QueueItem};

/// Which dry run answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DryRun {
    /// `xbps-install -n <packages>`: install or update the named packages.
    Install,
    /// `xbps-install -n -Su`: every available update.
    System,
    /// `xbps-remove -n <packages>`: remove the named packages.
    Remove,
}

impl DryRun {
    /// Whether the run asks xbps to remove packages, which is where a refused
    /// transaction is the answer rather than an error.
    pub fn removes(self) -> bool {
        self == Self::Remove
    }
}

/// What one dry run should plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanRequest {
    /// The names the user asks for, in queue order.
    pub names: Vec<String>,
    /// Install or update exactly `names`.
    pub install: bool,
    /// Update everything xbps has a newer version of.
    pub system: bool,
    /// Remove `names`.
    pub remove: bool,
}

impl PlanRequest {
    /// `xbps-install -n <names>`.
    pub fn install(names: Vec<String>) -> Self {
        Self {
            names,
            install: true,
            ..Self::default()
        }
    }

    /// `xbps-install -n -Su`.
    pub fn system() -> Self {
        Self {
            system: true,
            ..Self::default()
        }
    }

    /// `xbps-remove -n <names>`.
    pub fn remove(names: Vec<String>) -> Self {
        Self {
            names,
            remove: true,
            ..Self::default()
        }
    }

    /// The dry run this request asks for, or `None` when it asks for neither
    /// installing nor removing anything.
    pub fn dry_run(&self) -> Option<DryRun> {
        if self.remove {
            Some(DryRun::Remove)
        } else if self.system {
            Some(DryRun::System)
        } else if self.install {
            Some(DryRun::Install)
        } else {
            None
        }
    }

    /// Whether xbps would do nothing: an empty name list for an install, or a
    /// removal with nothing to remove.
    pub fn is_empty(&self) -> bool {
        !self.system && self.names.is_empty()
    }
}

/// The names a queue asks to install or update.
pub fn install_names(queue: &[QueueItem]) -> Vec<String> {
    queue
        .iter()
        .filter(|item| item.action != QueueAction::Remove)
        .map(|item| item.name.clone())
        .collect()
}

/// The names a queue asks to remove.
pub fn remove_names(queue: &[QueueItem]) -> Vec<String> {
    queue
        .iter()
        .filter(|item| item.action == QueueAction::Remove)
        .map(|item| item.name.clone())
        .collect()
}

/// Update entries the queue asks for packages that are not installed.
///
/// xbps has nothing to say about this: an install of an installed package is
/// an empty run, so the queue's own entries are checked against the installed
/// set, and every missing name is reported at once.
pub fn not_installed(queue: &[QueueItem], installed: &InstalledMap) -> Vec<String> {
    queue
        .iter()
        .filter(|item| item.action == QueueAction::Update && !installed.contains_key(&item.name))
        .map(|item| item.name.clone())
        .collect()
}

/// The queue with every install of an installed package turned into an update.
///
/// `xbps-install` treats a name the machine already holds as an upgrade, so the
/// two actions are the same command with a different label — but the label is
/// what the plan and the log say. The rows queue an update directly; this is
/// the second net, for a queue file written by an older build. A package with
/// nothing newer answers "already installed", which the dry run turns into a
/// notice instead of a password prompt.
pub fn installs_as_updates(queue: &[QueueItem], installed: &InstalledMap) -> Vec<QueueItem> {
    queue
        .iter()
        .map(|item| {
            if item.action == QueueAction::Install && installed.contains_key(&item.name) {
                QueueItem {
                    name: item.name.clone(),
                    action: QueueAction::Update,
                }
            } else {
                item.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Inst;

    fn queue(name: &str, action: QueueAction) -> QueueItem {
        QueueItem {
            name: name.to_owned(),
            action,
        }
    }

    #[test]
    fn a_queue_splits_into_installs_and_removals() {
        let queue = vec![
            queue("vim", QueueAction::Install),
            queue("git", QueueAction::Update),
            queue("zramen", QueueAction::Remove),
        ];
        assert_eq!(install_names(&queue), ["vim", "git"]);
        assert_eq!(remove_names(&queue), ["zramen"]);
    }

    #[test]
    fn a_request_names_its_dry_run() {
        assert_eq!(PlanRequest::system().dry_run(), Some(DryRun::System));
        assert_eq!(
            PlanRequest::install(vec!["vim".into()]).dry_run(),
            Some(DryRun::Install)
        );
        assert_eq!(
            PlanRequest::remove(vec!["vim".into()]).dry_run(),
            Some(DryRun::Remove)
        );
        assert_eq!(PlanRequest::default().dry_run(), None);
        assert!(PlanRequest::install(Vec::new()).is_empty());
        assert!(!PlanRequest::system().is_empty());
    }

    #[test]
    fn an_update_of_an_absent_package_is_named() {
        let installed: InstalledMap = [(
            "vim".to_owned(),
            Inst {
                version: "9.2_1".into(),
                explicit: true,
                held: false,
                at: 0,
            },
        )]
        .into_iter()
        .collect();
        let queue = vec![
            queue("vim", QueueAction::Update),
            queue("ghost", QueueAction::Update),
        ];
        assert_eq!(not_installed(&queue, &installed), ["ghost"]);
    }

    #[test]
    fn an_install_of_an_installed_package_becomes_an_update() {
        let installed: InstalledMap = [(
            "vim".to_owned(),
            Inst {
                version: "9.2_1".into(),
                explicit: true,
                held: false,
                at: 0,
            },
        )]
        .into_iter()
        .collect();
        let items = vec![
            queue("vim", QueueAction::Install),
            queue("git", QueueAction::Install),
            queue("vim", QueueAction::Update),
        ];
        assert_eq!(
            installs_as_updates(&items, &installed),
            [
                queue("vim", QueueAction::Update),
                queue("git", QueueAction::Install),
                queue("vim", QueueAction::Update)
            ],
            "the installed install is relabelled, the absent one is not"
        );
    }
}
