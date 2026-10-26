fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        std::env::set_var("MACOSX_DEPLOYMENT_TARGET", "14.0");
        println!("cargo:rustc-env=MACOSX_DEPLOYMENT_TARGET=14.0");
        cc::Build::new()
            .file("src/macos_native.m")
            .flag("-fobjc-arc")
            .flag("-mmacosx-version-min=14.0")
            .compile("fievel_macos_native");
        for framework in [
            "AppKit",
            "ApplicationServices",
            "CoreGraphics",
            "CoreFoundation",
            "ScreenCaptureKit",
        ] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        println!("cargo:rustc-link-arg=-mmacosx-version-min=14.0");
        println!("cargo:rerun-if-changed=src/macos_native.m");
    }
}
