use std::path::PathBuf;
use std::process::Command;

/// Server mode must start with a valid config, not crash.
#[test]
fn server_starts_with_config() {
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("minimal-server-config.toml");

    let mut child = Command::new(env!("CARGO_BIN_EXE_flo-server"))
        .args([
            "--robot-id",
            "server-1",
            "--config",
            &config_path.to_string_lossy(),
            "--auth-mode",
            "none",
            "--auth-allow-insecure",
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn flo-server");

    let stdout_handle = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            let mut buf = [0u8; 4096];
            while let Ok(n) = out.read(&mut buf) {
                if n == 0 {
                    break;
                }
                s.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            s
        })
    });

    std::thread::sleep(std::time::Duration::from_millis(2000));

    let _ = child.kill();
    let _ = child.wait();

    let output = stdout_handle
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();

    assert!(
        !output.contains("panic"),
        "flo-server panicked: {output}"
    );
    assert!(
        output.contains("server mode started"),
        "expected 'server mode started', got: {output}"
    );
}
