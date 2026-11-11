fn main() {
    // Hand-written C++ shim that installs KI18n's `KLocalizedContext` on the QML
    // engine's root context, which is what makes `i18nd()` callable in QML.
    let builder = cxx_qt_build::CxxQtBuilder::new_qml_module(
        cxx_qt_build::QmlModule::new("org.kde.khaxbps").qml_file("qml/Main.qml"),
    )
    .qt_module("Network")
    // Every file with a `#[cxx_qt::bridge]` module: the app backend, the
    // package/queue/log models, the plan/job dialogs and the installed backend.
    // All of them must live flat in `src/` (cxx-qt-build 0.10 panics otherwise).
    .files([
        "src/lib.rs",
        "src/package_model.rs",
        "src/catalog_backend.rs",
        "src/installed_backend.rs",
        "src/plan_backend.rs",
        "src/prefs_backend.rs",
        "src/job_backend.rs",
        "src/queue_model.rs",
        "src/log_model.rs",
        "src/mirror_model.rs",
        "src/mirror_backend.rs",
    ])
    // The package catalog is generated from the web app by `scripts/gen-catalog.mjs`.
    // Because the builder knows the QML module URI, the file lands in the module's
    // import path as `:/qt/qml/org/kde/khaxbps/resources/catalog.json`.
    .qrc_resources(["resources/catalog.json"])
    // `QFile` access to the catalog resource plus the protected
    // `QAbstractItemModel` members the Rust model cannot reach.
    .cpp_file("src/k18n_shim.cpp")
    .cpp_file("src/model_shim.cpp")
    // The mirror model's factory and the `QNetworkAccessManager` probe
    // transport, which `cxx-qt-lib` 0.10 has no binding for.
    .cpp_file("src/mirror_shim.cpp")
    // Child-object factories and the forwarders that let Rust reach a generated
    // method on a `QObject*` it only holds as a pointer.
    .cpp_file("src/backend_shim.cpp");

    // `cxx-qt-build` has no `include_path` helper, so add the KI18n header
    // directory through the underlying `cc::Build`. The crate's own `src` dir is
    // needed as well: `lib.rs` includes `k18n_shim.h` from there and the
    // generated C++ lives in the build directory, so the relative lookup fails.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let builder = unsafe {
        builder.cc_builder(move |cc| {
            cc.include("/usr/include/KF6/KI18n");
            cc.include(format!("{manifest_dir}/src"));
        })
    };

    builder.build();

    // `KLocalizedContext` lives in libKF6I18nQml, `KLocalizedString` in
    // libKF6I18n; neither is pulled in by the Qt module list.
    println!("cargo:rustc-link-lib=KF6I18nQml");
    println!("cargo:rustc-link-lib=KF6I18n");

    // The C++ archive generated for this crate carries the QML module registration
    // (`cxx_qt_init_qml_module_org_kde_khaxbps`) and the `AppBackend` QObject code.
    // Cargo applies the `rustc-link-lib` directive cxx-qt-build emits to the library
    // unit only, so the binary would link without it and fail on the initializer
    // symbol; passing it as a link arg makes sure it reaches the final link too.
    println!("cargo:rustc-link-arg=-lkhaxbps-qt-cxxqt-generated");
}
