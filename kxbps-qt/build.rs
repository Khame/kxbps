// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// Shared with the binary, which uses the same hash to tell when it is older
// than the QML on disk.
include!("src/qml_stamp.rs");
// Shared with the binary's tests: where KF6 keeps its headers next to Qt's.
include!("src/qt_headers.rs");

/// Every QML file the module carries. A missing entry fails the build instead
/// of the window.
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

/// Qt's header directory as this build sees it: `/usr/include/qt6` natively,
/// `<sysroot>/usr/include/qt6` when `QMAKE` points at a cross wrapper.
fn qt_headers() -> Option<std::path::PathBuf> {
    let qmake = std::env::var("QMAKE").unwrap_or_else(|_| "/usr/lib/qt6/bin/qmake6".to_owned());
    let output = std::process::Command::new(qmake)
        .args(["-query", "QT_INSTALL_HEADERS"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    Some(std::path::PathBuf::from(path.trim()))
}

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

    // clippy and test re-run this script without relinking, so the binary can
    // carry QML older than the tree it appears to run.
    if let Some((hash, _)) = qml_tree_hash(&qml_dir) {
        println!("cargo:rustc-env=KXBPS_QML_DIR={}", qml_dir.display());
        println!("cargo:rustc-env=KXBPS_QML_HASH={hash}");
    }

    // `qrc` only watches the `.qrc` itself; without these lines a replaced icon
    // would not trigger a rebuild.
    println!("cargo:rerun-if-changed=../packaging/icons/app/kxbps.svg");
    println!("cargo:rerun-if-changed=../packaging/icons/app/kxbps-updates.svg");
    println!("cargo:rerun-if-changed=../packaging/icons/actions");
    // Cargo watches only the paths named below once any rerun-if-changed is
    // emitted; without the directory a QML edit changed nothing in the binary.
    println!("cargo:rerun-if-changed=qml");
    println!("cargo:rerun-if-changed=build.rs");

    // Installs KI18n's `KLocalizedContext` on the engine root context, which is
    // what makes `i18nd()` callable in QML.
    let builder = cxx_qt_build::CxxQtBuilder::new_qml_module(
        cxx_qt_build::QmlModule::new("org.kde.kxbps").qml_files(QML_FILES),
    )
    .qt_module("Network")
    // Notifications go straight to `org.freedesktop.Notifications`; the
    // KNotification module dropped the event silently.
    .qt_module("DBus")
    // KStatusNotifierItem owns a `QMenu` it exports over D-Bus and never shows.
    .qt_module("Widgets")
    // The icon's single source is `packaging/`; the alias puts it under the QML
    // module path, `qrc:/qt/qml/org/kde/kxbps/icons/kxbps.svg`.
    .qrc("resources.qrc")
    // Every file with a `#[cxx_qt::bridge]` module. They must stay flat in
    // `src/` (cxx-qt-build 0.10 panics otherwise).
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
    // `QFile` access to the protected `QAbstractItemModel` members.
    .cpp_file("src/k18n_shim.cpp")
    // The About page's Qt/KDE versions and platform name.
    .cpp_file("src/about_shim.cpp")
    // Keeps the desktop style from underlining button mnemonics.
    .cpp_file("src/style_shim.cpp")
    .cpp_file("src/model_shim.cpp")
    // The mirror model's factory and its `QNetworkAccessManager` transport,
    // which cxx-qt-lib 0.10 does not bind.
    .cpp_file("src/mirror_shim.cpp")
    // Child-object factories and the forwarders Rust needs to call a generated
    // method through a `QObject*`.
    .cpp_file("src/backend_shim.cpp")
    // The read-only xbps runner: one `QProcess` per call, explicit argv, hard
    // timeout, driven from a worker thread.
    .cpp_file("src/xbps_shim.cpp")
    // `QNetworkInformation` has no cxx-qt-lib binding, and xbps cannot tell a
    // DNS failure from an empty answer.
    .cpp_file("src/network_shim.cpp")
    // The notification click slot; CXX-Qt 0.10 cannot declare one in Rust.
    .cpp_file("src/notification_watcher.h")
    // The tray: StatusNotifierItem with a D-Bus menu, no widget popup, so
    // Wayland has nothing to grab.
    .cpp_file("src/tray_item.h")
    .cpp_file("src/tray_shim.cpp");

    // cxx-qt-build has no include helper, so the paths go through `cc::Build`.
    // `src/` is needed too: `lib.rs` includes `k18n_shim.h` and the generated
    // C++ sits in the build directory.
    let src_dir = format!("{manifest_dir}/src");
    let qt_include = qt_headers().unwrap_or_else(|| std::path::PathBuf::from("/usr/include/qt6"));
    let kf6_root = include_root(&qt_include);
    let builder = unsafe {
        builder.cc_builder(move |cc| {
            cc.include(kf6_root.join("KF6/KI18n"));
            cc.include(kf6_root.join("KF6/KStatusNotifierItem"));
            // `qqmlintegration.h` lives in Qt's integration module; the include
            // list CXX-Qt assembles does not carry it.
            cc.include(qt_include.join("QtQmlIntegration"));
            cc.include(&src_dir);
        })
    };

    builder.build();

    // Neither library comes in through the Qt module list.
    println!("cargo:rustc-link-lib=KF6I18nQml");
    println!("cargo:rustc-link-lib=KF6I18n");
    println!("cargo:rustc-link-lib=KF6StatusNotifierItem");

    // Cargo applies cxx-qt-build's `rustc-link-lib` to the library unit only, so
    // the binary would miss the module initializer without this link arg.
    println!("cargo:rustc-link-arg=-lkxbps-qt-cxxqt-generated");
}
