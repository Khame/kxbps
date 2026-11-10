fn main() {
    cxx_qt_build::CxxQtBuilder::new_qml_module(
        cxx_qt_build::QmlModule::new("org.kde.qxbps").qml_file("qml/Main.qml"),
    )
    .qt_module("Network")
    .files(["src/lib.rs"])
    .build();

    // The C++ archive generated for this crate carries the QML module registration
    // (`cxx_qt_init_qml_module_org_kde_qxbps`) and the `AppBackend` QObject code.
    // Cargo applies the `rustc-link-lib` directive cxx-qt-build emits to the library
    // unit only, so the binary would link without it and fail on the initializer
    // symbol; passing it as a link arg makes sure it reaches the final link too.
    println!("cargo:rustc-link-arg=-lqxbps-qt-cxxqt-generated");
}
