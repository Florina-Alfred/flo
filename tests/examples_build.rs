//! Ensures all examples compile (default features + media feature).

use std::process::Command;

#[test]
fn examples_compile_default() {
    let status = Command::new("cargo")
        .args(["build", "--examples"])
        .status()
        .expect("cargo build --examples");
    assert!(
        status.success(),
        "examples failed to build (default features)"
    );
}

#[test]
fn examples_compile_media() {
    let status = Command::new("cargo")
        .args(["build", "--examples", "--features", "media"])
        .status()
        .expect("cargo build --examples --features media");
    assert!(status.success(), "examples failed to build (media feature)");
}
