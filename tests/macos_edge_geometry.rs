#![cfg(target_os = "macos")]

use std::{path::Path, process::Command};

#[test]
fn native_edge_geometry() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/macos_edge_geometry.c");
    let binary = std::env::temp_dir().join(format!("fievel-edge-test-{}", std::process::id()));
    let compilation = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-mmacosx-version-min=14.0",
        ])
        .arg(source)
        .args(["-framework", "CoreGraphics", "-o"])
        .arg(&binary)
        .output()
        .expect("failed to invoke the macOS C compiler");
    assert!(
        compilation.status.success(),
        "native test compilation failed: {}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    let result = Command::new(&binary)
        .output()
        .expect("failed to run native tests");
    let _ = std::fs::remove_file(&binary);
    assert!(
        result.status.success(),
        "native tests failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
