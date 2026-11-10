fn main() {
    cxx_qt_build::CxxQtBuilder::new_qml_module(
        cxx_qt_build::QmlModule::new("org.kde.khaxbps").qml_file("qml/Main.qml"),
    )
    .qt_module("Network")
    .files(["src/lib.rs"])
    // The package catalog is generated from the web app by `scripts/gen-catalog.mjs`.
    // Because the builder knows the QML module URI, the file lands in the module's
    // import path as `:/qt/qml/org/kde/khaxbps/resources/catalog.json`.
    .qrc_resources(["resources/catalog.json"])
    .build();

    // The C++ archive generated for this crate carries the QML module registration
    // (`cxx_qt_init_qml_module_org_kde_khaxbps`) and the `AppBackend` QObject code.
    // Cargo applies the `rustc-link-lib` directive cxx-qt-build emits to the library
    // unit only, so the binary would link without it and fail on the initializer
    // symbol; passing it as a link arg makes sure it reaches the final link too.
    println!("cargo:rustc-link-arg=-lkhaxbps-qt-cxxqt-generated");
}
