//! A resolved transaction plan, ready for the confirm dialog.
//!
//! The plan is built by [`crate::installed_backend`] with the core's planners and
//! crosses into C++ as one JSON payload, so the dialog shows exactly the steps
//! that will later be applied: [`PlanPayload::items`] is serialised back out as
//! `itemsJson` and is what `applyPlan` replays.
//!
//! **No user-facing text lives here.** Titles, notes, notices and errors are
//! stable translation keys plus their substitution arguments; QML maps them
//! through `i18nd()`. The keys are listed in [`keys`].

use crate::xbps_plan::PlanError;
use cxx_qt_lib::{
    QList, QMap, QMapPair_QString_QVariant, QObjectMutPtr, QString, QStringList, QVariant,
    QVariantValue,
};
use kxbps_core::{Blocked, Plan, PlanItem, RepoId, action_key, plan_totals, reason_key};
use serde::{Deserialize, Serialize};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/core/qlist/qlist_QVariant.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;

        include!("cxx-qt-lib/qobjectmutptr.h");
        type QObjectMutPtr = cxx_qt_lib::QObjectMutPtr;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_uncreatable]
        #[qproperty(QList_QVariant, items, READ, CONSTANT)]
        #[qproperty(QList_QVariant, blocked, READ, CONSTANT)]
        #[qproperty(QStringList, names, READ, CONSTANT)]
        #[qproperty(i32, count, READ, CONSTANT)]
        #[qproperty(i64, download_total, READ, CONSTANT)]
        #[qproperty(i64, disk_delta, READ, CONSTANT)]
        #[qproperty(QString, download_text, READ, CONSTANT)]
        #[qproperty(QString, disk_delta_text, READ, CONSTANT)]
        #[qproperty(QString, title_key, READ, CONSTANT)]
        #[qproperty(QStringList, title_args, READ, CONSTANT)]
        #[qproperty(QString, note_key, READ, CONSTANT)]
        #[qproperty(QStringList, note_args, READ, CONSTANT)]
        #[qproperty(QString, error_key, READ, CONSTANT)]
        #[qproperty(QStringList, error_args, READ, CONSTANT)]
        #[qproperty(QString, notice_key, READ, CONSTANT)]
        #[qproperty(bool, is_blocked, READ, CONSTANT)]
        #[qproperty(QString, items_json, READ, CONSTANT)]
        type PlanBackend = super::PlanBackendRust;
    }

    impl
        cxx_qt::Constructor<
            (QString, QObjectMutPtr),
            BaseArguments = (QObjectMutPtr,),
            NewArguments = (QString,),
        > for PlanBackend
    {
    }
}

/// Translation keys the Qt layer emits instead of sentences.
///
/// QML owns the mapping (`i18nd("kxbps", …)`); Task 11 renders them. The same
/// bare-camelCase style as [`kxbps_core::probe::ProbeError::key`].
pub mod keys {
    /// Title of the queue plan.
    pub const PLAN_QUEUE: &str = "planQueue";
    /// Title of the system upgrade plan.
    pub const PLAN_SYSTEM: &str = "planSystem";
    /// Title of a caller-named install plan.
    pub const PLAN_INSTALL: &str = "planInstall";
    /// Title of `xbps-remove <arg>`.
    pub const PLAN_REMOVE: &str = "planRemove";
    /// Title of the removal with dependents.
    pub const PLAN_CASCADE: &str = "planCascade";
    /// Note under a plan that skipped held packages (`%1` = joined names).
    pub const NOTE_HELD: &str = "noteHeld";
    /// The queue does not change the system.
    pub const NOTICE_EMPTY_QUEUE: &str = "emptyQueue";
    /// Nothing to install.
    pub const NOTICE_EMPTY_INSTALL: &str = "emptyInstall";
    /// The system is up to date.
    pub const NOTICE_EMPTY_SYSTEM: &str = "emptySystem";
    /// The selection is not installed, so there is nothing to remove.
    pub const NOTICE_EMPTY_REMOVE: &str = "emptyRemove";
    /// The catalog does not know the package (`%1` = name).
    pub const ERROR_UNKNOWN_PACKAGE: &str = "errorUnknownPackage";
    /// The package's repository is disabled (`%1` = name, `%2` = repo key).
    pub const ERROR_REPO_DISABLED: &str = "errorRepoDisabled";
    /// A dependency sits in a disabled repository (`%1` = name, `%2` =
    /// dependency, `%3` = repo key).
    pub const ERROR_DEPENDENCY_REPO_DISABLED: &str = "errorDependencyRepoDisabled";
    /// The update targets are not installed (`%1` = joined names).
    pub const ERROR_NOT_INSTALLED: &str = "errorNotInstalled";
}

/// Everything the dialog needs, built in Rust and handed over as JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanPayload {
    pub title_key: String,
    pub title_args: Vec<String>,
    pub note_key: String,
    pub note_args: Vec<String>,
    pub error_key: String,
    pub error_args: Vec<String>,
    pub notice_key: String,
    pub names: Vec<String>,
    pub items: Vec<PlanItem>,
    pub blocked: Vec<Blocked>,
}

impl PlanPayload {
    /// A payload that only carries a "nothing to do" notice.
    pub fn notice(key: &str) -> Self {
        Self {
            notice_key: key.to_string(),
            ..Self::default()
        }
    }

    /// A payload that only carries a planning failure as a translation key.
    ///
    /// The key and its arguments come from [`PlanError`], which turns a refused
    /// dry run (a package xbps does not know, a disabled repository, a missing
    /// xbps) into the same shape QML already renders.
    pub fn from_error(err: &PlanError) -> Self {
        Self {
            error_key: err.key().to_string(),
            error_args: err.args(),
            ..Self::default()
        }
    }

    /// A payload for a plan the user can apply (or that is blocked).
    pub fn from_plan(
        plan: Plan,
        title_key: &str,
        title_args: Vec<String>,
        names: Vec<String>,
    ) -> Self {
        Self {
            title_key: title_key.to_string(),
            title_args,
            names,
            items: plan.items,
            blocked: plan.blocked,
            ..Self::default()
        }
    }
}

/// Rust side of [`qobject::PlanBackend`]; every field is prebuilt at
/// construction because the dialog never changes once it is open.
pub struct PlanBackendRust {
    items: QList<QVariant>,
    blocked: QList<QVariant>,
    names: QStringList,
    count: i32,
    download_total: i64,
    disk_delta: i64,
    download_text: QString,
    disk_delta_text: QString,
    title_key: QString,
    title_args: QStringList,
    note_key: QString,
    note_args: QStringList,
    error_key: QString,
    error_args: QStringList,
    notice_key: QString,
    is_blocked: bool,
    items_json: QString,
}

impl PlanBackendRust {
    /// Turns a payload into the values QML reads.
    pub fn from_payload(payload: &PlanPayload) -> Self {
        let totals = plan_totals(&payload.items);
        let download_total = i64::try_from(totals.download).unwrap_or(i64::MAX);
        let added = i64::try_from(totals.added).unwrap_or(i64::MAX);
        let removed = i64::try_from(totals.removed).unwrap_or(i64::MAX);

        Self {
            items: QList::from(payload.items.iter().map(item_value).collect::<Vec<_>>()),
            blocked: QList::from(
                payload
                    .blocked
                    .iter()
                    .map(blocked_value)
                    .collect::<Vec<_>>(),
            ),
            names: string_list(&payload.names),
            count: i32::try_from(payload.items.len()).unwrap_or(i32::MAX),
            download_total,
            disk_delta: added.saturating_sub(removed),
            download_text: QString::from(crate::package_model::format_size(totals.download)),
            // Unsigned on purpose: the sign glyph belongs to the dialog, which
            // can read `diskDelta` to decide between `+` and `−`.
            disk_delta_text: QString::from(crate::package_model::format_size(
                totals.added.abs_diff(totals.removed),
            )),
            title_key: QString::from(payload.title_key.as_str()),
            title_args: string_list(&payload.title_args),
            note_key: QString::from(payload.note_key.as_str()),
            note_args: string_list(&payload.note_args),
            error_key: QString::from(payload.error_key.as_str()),
            error_args: string_list(&payload.error_args),
            notice_key: QString::from(payload.notice_key.as_str()),
            is_blocked: !payload.blocked.is_empty(),
            items_json: QString::from(
                serde_json::to_string(&payload.items)
                    .unwrap_or_else(|_| String::from("[]"))
                    .as_str(),
            ),
        }
    }
}

impl cxx_qt::Constructor<(QString, QObjectMutPtr)> for qobject::PlanBackend {
    type BaseArguments = (QObjectMutPtr,);
    type NewArguments = (QString,);
    type InitializeArguments = ();

    fn route_arguments(
        arguments: (QString, QObjectMutPtr),
    ) -> (
        Self::NewArguments,
        Self::BaseArguments,
        Self::InitializeArguments,
    ) {
        let (payload_json, parent) = arguments;
        ((payload_json,), (parent,), ())
    }

    fn new((payload_json,): Self::NewArguments) -> PlanBackendRust {
        let payload: PlanPayload =
            serde_json::from_str(&payload_json.to_string()).unwrap_or_default();
        PlanBackendRust::from_payload(&payload)
    }
}

fn string_list(values: &[String]) -> QStringList {
    values
        .iter()
        .map(|value| QString::from(value.as_str()))
        .collect()
}

/// One plan step as a `QVariantMap`, keyed for QML delegates.
fn item_value(item: &PlanItem) -> QVariant {
    let mut map = QMap::default();
    insert(&mut map, "name", &QString::from(item.name.as_str()));
    insert(&mut map, "action", &QString::from(action_key(item.action)));
    insert(
        &mut map,
        "from",
        &QString::from(item.from.as_deref().unwrap_or("")),
    );
    insert(
        &mut map,
        "to",
        &QString::from(item.to.as_deref().unwrap_or("")),
    );
    insert(&mut map, "reason", &QString::from(reason_key(item.reason)));
    insert(
        &mut map,
        "download",
        &i64::try_from(item.download).unwrap_or(i64::MAX),
    );
    insert(
        &mut map,
        "size",
        &i64::try_from(item.size).unwrap_or(i64::MAX),
    );
    insert(
        &mut map,
        "downloadText",
        &QString::from(crate::package_model::format_size(item.download)),
    );
    insert(
        &mut map,
        "sizeText",
        &QString::from(crate::package_model::format_size(item.size)),
    );
    QVariant::from(&map)
}

/// One blocked package as `{name, neededBy: [names]}`.
fn blocked_value(blocked: &Blocked) -> QVariant {
    let mut map = QMap::default();
    insert(&mut map, "name", &QString::from(blocked.name.as_str()));
    insert(&mut map, "neededBy", &string_list(&blocked.needed_by));
    QVariant::from(&map)
}

fn insert<T: QVariantValue>(map: &mut QMap<QMapPair_QString_QVariant>, key: &str, value: &T) {
    map.insert_clone(&QString::from(key), &QVariant::from(value));
}

/// Wire key of a repository, the same spelling the package model uses.
pub fn repo_key(repo: RepoId) -> &'static str {
    match repo {
        RepoId::Current => "current",
        RepoId::Nonfree => "nonfree",
        RepoId::Multilib => "multilib",
        RepoId::Debug => "debug",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kxbps_core::{PlanAction, PlanReason};

    fn item(name: &str, action: PlanAction, download: u64, size: u64) -> PlanItem {
        PlanItem {
            name: name.into(),
            action,
            from: Some("1.0_1".into()),
            to: Some("1.1_1".into()),
            reason: PlanReason::Requested,
            download,
            size,
        }
    }

    fn plan() -> Plan {
        Plan {
            items: vec![
                item("vim", PlanAction::Install, 1_468_006, 4_194_304),
                item("tzdata", PlanAction::Remove, 0, 2_097_152),
            ],
            blocked: Vec::new(),
        }
    }

    fn map_of(value: &QVariant) -> QMap<QMapPair_QString_QVariant> {
        value.value::<QMap<QMapPair_QString_QVariant>>().unwrap()
    }

    fn text_of(map: &QMap<QMapPair_QString_QVariant>, key: &str) -> String {
        map.get(&QString::from(key))
            .and_then(|value| value.value::<QString>())
            .map(|value| value.to_string())
            .unwrap_or_default()
    }

    #[test]
    fn payload_totals_and_texts_describe_the_plan() {
        let backend = PlanBackendRust::from_payload(&PlanPayload::from_plan(
            plan(),
            keys::PLAN_QUEUE,
            Vec::new(),
            vec!["vim".into()],
        ));
        assert_eq!(backend.count, 2);
        assert_eq!(backend.download_total, 1_468_006);
        assert_eq!(backend.disk_delta, 4_194_304 - 2_097_152);
        assert_eq!(backend.download_text.to_string(), "1.4 MiB");
        assert_eq!(backend.disk_delta_text.to_string(), "2.0 MiB");
        assert_eq!(backend.title_key.to_string(), "planQueue");
        assert!(!backend.is_blocked);
        assert_eq!(backend.names.len(), 1);
        assert_eq!(backend.names.get(0).unwrap().to_string(), "vim");
    }

    #[test]
    fn items_json_round_trips_to_the_plan_items() {
        let backend = PlanBackendRust::from_payload(&PlanPayload::from_plan(
            plan(),
            keys::PLAN_QUEUE,
            Vec::new(),
            Vec::new(),
        ));
        let items: Vec<PlanItem> =
            serde_json::from_str(&backend.items_json.to_string()).expect("items json");
        assert_eq!(items, plan().items);
    }

    #[test]
    fn item_maps_carry_the_core_keys() {
        let value = item_value(&item("vim", PlanAction::Update, 10, 20));
        let map = map_of(&value);
        assert_eq!(text_of(&map, "name"), "vim");
        assert_eq!(text_of(&map, "action"), "update");
        assert_eq!(text_of(&map, "reason"), "requested");
        assert_eq!(text_of(&map, "from"), "1.0_1");
        assert_eq!(text_of(&map, "to"), "1.1_1");
        assert_eq!(text_of(&map, "downloadText"), "10 B");
    }

    #[test]
    fn blocked_plans_report_the_dependents() {
        let plan = Plan {
            items: Vec::new(),
            blocked: vec![Blocked {
                name: "glibc".into(),
                needed_by: vec!["bash".into(), "vim".into()],
            }],
        };
        let backend = PlanBackendRust::from_payload(&PlanPayload::from_plan(
            plan,
            keys::PLAN_REMOVE,
            vec!["glibc".into()],
            vec!["glibc".into()],
        ));
        assert!(backend.is_blocked);
        assert_eq!(backend.count, 0);
        assert_eq!(backend.blocked.len(), 1);
        let map = map_of(backend.blocked.get(0).unwrap());
        assert_eq!(text_of(&map, "name"), "glibc");
        let needed_by = map
            .get(&QString::from("neededBy"))
            .and_then(|value| value.value::<QStringList>())
            .unwrap();
        assert_eq!(needed_by.len(), 2);
    }

    #[test]
    fn plan_errors_become_keys_not_sentences() {
        let payload = PlanPayload::from_error(&PlanError::UnknownPackage("nope".into()));
        assert_eq!(payload.error_key, "errorUnknownPackage");
        assert_eq!(payload.error_args, vec!["nope".to_string()]);

        let payload = PlanPayload::from_error(&PlanError::RepoDisabled {
            package: "steam".into(),
            repo: "nonfree".into(),
        });
        assert_eq!(payload.error_key, "errorRepoDisabled");
        assert_eq!(
            payload.error_args,
            vec!["steam".to_string(), "nonfree".to_string()]
        );

        let payload =
            PlanPayload::from_error(&PlanError::NotInstalled(vec!["a".into(), "b".into()]));
        assert_eq!(payload.error_key, "errorNotInstalled");
        assert_eq!(payload.error_args, vec!["a, b".to_string()]);
    }

    #[test]
    fn notice_payloads_carry_no_items() {
        let backend =
            PlanBackendRust::from_payload(&PlanPayload::notice(keys::NOTICE_EMPTY_SYSTEM));
        assert_eq!(backend.notice_key.to_string(), "emptySystem");
        assert_eq!(backend.count, 0);
        assert_eq!(backend.items_json.to_string(), "[]");
        assert_eq!(backend.download_text.to_string(), "0 B");
    }

    #[test]
    fn format_size_matches_the_package_model() {
        use crate::package_model::format_size;
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(4_194_304), "4.0 MiB");
        assert_eq!(format_size(1_468_006), "1.4 MiB");
    }
}
