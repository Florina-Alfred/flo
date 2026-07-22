use std::process::Command;

/// Production mode with a missing config must still start (fail-safe), not crash.
#[test]
fn production_missing_config_starts_safe() {
    // Provide explicit auth: none (with allow-insecure) so the auth gate passes
    // and the test reaches the config-file fallback (safe-state) path.
    let mut child = Command::new(env!("CARGO_BIN_EXE_flo"))
        .args([
            "--robot-id",
            "7",
            "--config",
            "/nonexistent/flo/rules.toml",
            "--auth-mode",
            "none",
            "--auth-allow-insecure",
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn flo");

    // flo is a long-running daemon. Spawn a reader thread that drains stderr
    // (it will block until we kill the child), then give flo time to emit the
    // safe-state log before killing it.
    let stderr_handle = child.stdout.take().map(|mut out| {
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

    std::thread::sleep(std::time::Duration::from_millis(1500));

    let _ = child.kill();
    let _ = child.wait();

    let stderr = stderr_handle
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();

    assert!(
        !stderr.contains("panic"),
        "flo panicked on missing config: {stderr}"
    );
    assert!(
        stderr.contains("safe-state"),
        "expected safe-state fallback, got: {stderr}"
    );
}
