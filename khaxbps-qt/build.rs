fn main() {
    // Hand-written C++ shim that installs KI18n's `KLocalizedContext` on the QML
    // engine's root context, which is what makes `i18nd()` callable in QML.
    let builder = cxx_qt_build::CxxQtBuilder::new_qml_module(
        cxx_qt_build::QmlModule::new("org.kde.khaxbps").qml_file("qml/Main.qml"),
    )
    .qt_module("Network")
    .files(["src/lib.rs"])
    // The package catalog is generated from the web app by `scripts/gen-catalog.mjs`.
    // Because the builder knows the QML module URI, the file lands in the module's
    // import path as `:/qt/qml/org/kde/khaxbps/resources/catalog.json`.
    .qrc_resources(["resources/catalog.json"])
    .cpp_file("src/k18n_shim.cpp");

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
