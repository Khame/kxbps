// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// Shared with the binary, which uses the same hash to tell when it is older
// than the QML on disk.
include!("src/qml_stamp.rs");

/// Every QML file the module carries. A file missing here is never embedded,
/// so the build fails on it instead of the window at runtime.
const QML_FILES: &[&str] = &[
    "qml/Main.qml",
    "qml/views/PackagesView.qml",
    "qml/views/MirrorsView.qml",
    "qml/views/SettingsView.qml",
    "qml/views/HelpView.qml",
    "qml/views/RepositoriesView.qml",
    "qml/components/AboutActionRow.qml",
    "qml/components/KxbpsIcon.qml",
    "qml/components/NavItem.qml",
    "qml/components/PackageDelegate.qml",
    "qml/components/QueueDelegate.qml",
    "qml/components/MirrorDelegate.qml",
    "qml/components/DenseButton.qml",
    "qml/components/DetailsPane.qml",
    "qml/components/PackageFilesDialog.qml",
    "qml/components/StatusStrip.qml",
    "qml/components/LogDrawer.qml",
    "qml/components/PlanDialog.qml",
    "qml/components/BlockedDialog.qml",
    "qml/components/AboutDialog.qml",
    // The notification is the one component with an optional module import;
    // `Main.qml` loads it through a `Loader` so a missing module costs the
    // feature, not the window.
    "qml/components/Notifier.qml",
];

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let qml_dir = std::path::Path::new(&manifest_dir).join("qml");

    let listed: Vec<std::path::PathBuf> = QML_FILES.iter().map(std::path::PathBuf::from).collect();
    let unlisted: Vec<String> = qml_files(&qml_dir)
        .unwrap_or_default()
        .into_iter()
        .map(|file| std::path::Path::new("qml").join(file))
        .filter(|file| !listed.contains(file))
        .map(|file| file.display().to_string())
        .collect();
    if !unlisted.is_empty() {
        panic!("QML files missing from build.rs: {}", unlisted.join(", "));
    }

    // The binary compares this with the tree on disk at startup: `cargo clippy`
    // and `cargo test` re-run this script without linking, so the executable
    // can stay older than the QML it appears to run.
    if let Some((hash, _)) = qml_tree_hash(&qml_dir) {
        println!("cargo:rustc-env=KXBPS_QML_DIR={}", qml_dir.display());
        println!("cargo:rustc-env=KXBPS_QML_HASH={hash}");
    }

    // `qrc` only watches the `.qrc` itself; without these lines a replaced icon
    // would not trigger a rebuild.
    println!("cargo:rerun-if-changed=../packaging/icons/app/kxbps.svg");
    println!("cargo:rerun-if-changed=../packaging/icons/app/kxbps-updates.svg");
    println!("cargo:rerun-if-changed=../packaging/icons/actions");
    // The QML files are compiled into the binary by this script. Cargo watches
    // only the paths named here once any `rerun-if-changed` is emitted, so
    // without the directory a QML edit left the old file in the executable and
    // the change looked like it did nothing.
    println!("cargo:rerun-if-changed=qml");
    println!("cargo:rerun-if-changed=build.rs");

    // Hand-written C++ shim that installs KI18n's `KLocalizedContext` on the QML
    // engine's root context, which is what makes `i18nd()` callable in QML.
    let builder = cxx_qt_build::CxxQtBuilder::new_qml_module(
        cxx_qt_build::QmlModule::new("org.kde.kxbps").qml_files(QML_FILES),
    )
    .qt_module("Network")
    // The notification goes straight to `org.freedesktop.Notifications`; the
    // KNotification module dropped the event without telling anyone.
    .qt_module("DBus")
    // The tray's menu is a `QMenu` owned by KStatusNotifierItem, which exports
    // it over D-Bus and never shows it; the headers come from Qt Widgets.
    .qt_module("Widgets")
    // The application icon, kept in `packaging/` as the single source. The
    // `.qrc` alias puts it under the QML module's own import path, so QML reads
    // it as `qrc:/qt/qml/org/kde/kxbps/icons/kxbps.svg`.
    .qrc("resources.qrc")
    // Every file with a `#[cxx_qt::bridge]` module: the app backend, the
    // package/queue/log models, the plan/job dialogs and the installed backend.
    // All of them must live flat in `src/` (cxx-qt-build 0.10 panics otherwise).
    .files([
        "src/lib.rs",
        "src/package_model.rs",
        "src/query_backend.rs",
        "src/installed_backend.rs",
        "src/plan_backend.rs",
        "src/prefs_backend.rs",
        "src/job_backend.rs",
        "src/queue_model.rs",
        "src/log_model.rs",
        "src/mirror_model.rs",
        "src/mirror_backend.rs",
        "src/xbps_bridge.rs",
        "src/network.rs",
    ])
    // `QFile` access to the protected `QAbstractItemModel` members the Rust
    // model cannot reach.
    .cpp_file("src/k18n_shim.cpp")
    // The About page's Qt/KDE Frameworks versions and platform name.
    .cpp_file("src/about_shim.cpp")
    // The style proxy that keeps the desktop style from underlining button
    // mnemonics.
    .cpp_file("src/style_shim.cpp")
    .cpp_file("src/model_shim.cpp")
    // The mirror model's factory and the `QNetworkAccessManager` probe
    // transport, which `cxx-qt-lib` 0.10 has no binding for.
    .cpp_file("src/mirror_shim.cpp")
    // Child-object factories and the forwarders that let Rust reach a generated
    // method on a `QObject*` it only holds as a pointer.
    .cpp_file("src/backend_shim.cpp")
    // The read-only xbps runner: one `QProcess` per call with an explicit argv
    // list and a hard timeout, driven from a worker thread.
    .cpp_file("src/xbps_shim.cpp")
    // Network reachability for the update check: `QNetworkInformation` has no
    // `cxx-qt-lib` binding, and xbps cannot tell a DNS failure from an empty
    // answer.
    .cpp_file("src/network_shim.cpp")
    // The notification click slot. A header so `cxx-qt-build` runs moc over it;
    // the slot cannot live in Rust, where CXX-Qt 0.10 has no slot support.
    .cpp_file("src/notification_watcher.h")
    // The tray: a StatusNotifierItem with a D-Bus menu. The header carries the
    // Q_OBJECT class, so moc runs over it; `Qt.labs.platform` is no longer used,
    // so no widget popup is created and Wayland has nothing to grab.
    .cpp_file("src/tray_item.h")
    .cpp_file("src/tray_shim.cpp");

    // `cxx-qt-build` has no `include_path` helper, so add the KI18n header
    // directory through the underlying `cc::Build`. The crate's own `src` dir is
    // needed as well: `lib.rs` includes `k18n_shim.h` from there and the
    // generated C++ lives in the build directory, so the relative lookup fails.
    let src_dir = format!("{manifest_dir}/src");
    let builder = unsafe {
        builder.cc_builder(move |cc| {
            cc.include("/usr/include/KF6/KI18n");
            cc.include("/usr/include/KF6/KStatusNotifierItem");
            // `KLocalizedQmlContext` includes `qqmlintegration.h`, which lives in
            // Qt's integration module: `pkg-config Qt6Qml` adds this path, the
            // include list CXX-Qt assembles does not.
            cc.include("/usr/include/qt6/QtQmlIntegration");
            cc.include(&src_dir);
        })
    };

    builder.build();

    // `KLocalizedContext` lives in libKF6I18nQml, `KLocalizedString` in
    // libKF6I18n; neither is pulled in by the Qt module list.
    println!("cargo:rustc-link-lib=KF6I18nQml");
    println!("cargo:rustc-link-lib=KF6I18n");
    // The tray item: KStatusNotifierItem, whose menu is exported over D-Bus.
    println!("cargo:rustc-link-lib=KF6StatusNotifierItem");

    // The C++ archive generated for this crate carries the QML module registration
    // (`cxx_qt_init_qml_module_org_kde_kxbps`) and the `AppBackend` QObject code.
    // Cargo applies the `rustc-link-lib` directive cxx-qt-build emits to the library
    // unit only, so the binary would link without it and fail on the initializer
    // symbol; passing it as a link arg makes sure it reaches the final link too.
    println!("cargo:rustc-link-arg=-lkxbps-qt-cxxqt-generated");
}
