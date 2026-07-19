//! Ensures all default-feature examples compile. (The `media`-feature example
//! requires system GStreamer, which is not present in CI; it is documented in
//! the README and verified manually where GStreamer is installed.)

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
