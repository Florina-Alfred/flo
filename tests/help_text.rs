use std::process::Command;

#[test]
fn help_lists_all_flags_including_video() {
    let out = Command::new(env!("CARGO_BIN_EXE_flo"))
        .arg("--help")
        .output()
        .expect("run flo --help");
    let text = String::from_utf8_lossy(&out.stdout);
    for flag in [
        "--robot-id",
        "--config",
        "--simulate",
        "--simulate-period-ms",
        "--video-peer",
        "--video-device",
        "--video-codec",
        "--video-self-test",
        "--help",
    ] {
        assert!(text.contains(flag), "help missing flag: {flag}");
    }
}
