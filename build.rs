use std::{env, fs, path::Path};

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        embed_menu_bar_icon();
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

fn embed_menu_bar_icon() {
    println!("cargo:rerun-if-env-changed=FIEVEL_MENU_BAR_ICON");
    let out_dir = env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo");
    let out_dir = Path::new(&out_dir);
    let icon_source = match env::var_os("FIEVEL_MENU_BAR_ICON") {
        Some(path) => {
            let path = Path::new(&path);
            println!("cargo:rerun-if-changed={}", path.display());
            let icon = fs::read(path).unwrap_or_else(|error| {
                panic!(
                    "could not read FIEVEL_MENU_BAR_ICON at {}: {error}",
                    path.display()
                )
            });
            if icon.is_empty() {
                panic!("FIEVEL_MENU_BAR_ICON must point to a non-empty image file");
            }
            fs::write(out_dir.join("menu_bar_icon.data"), icon)
                .expect("could not write embedded menu bar icon");
            "pub const MENU_BAR_ICON: Option<&[u8]> = Some(include_bytes!(concat!(env!(\"OUT_DIR\"), \"/menu_bar_icon.data\")));\n"
        }
        None => "pub const MENU_BAR_ICON: Option<&[u8]> = None;\n",
    };
    fs::write(out_dir.join("menu_bar_icon.rs"), icon_source)
        .expect("could not generate menu bar icon module");
}
