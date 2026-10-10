//! `zhang serve` must exit with a non-zero code when the ledger cannot be loaded, so process
//! supervisors and hosting platforms see a failed start instead of a clean exit (#486).

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn serve_exits_non_zero_when_the_ledger_fails_to_load() {
    let dir = tempfile::tempdir().unwrap();
    // an unterminated string: the ledger cannot be parsed, so the initial load fails
    std::fs::write(
        dir.path().join("main.zhang"),
        "option \"operating_currency\" \"CNY\"\n2024-01-02 * \"broken\n  this is not valid @@@ ::\n",
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_zhang"))
        .args(["serve", dir.path().to_str().unwrap(), "--addr", "127.0.0.1", "--port", "0", "--no-report"])
        .env("ZHANG_LOG", "error")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("zhang serve kept running although its ledger could not be loaded");
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    assert_eq!(status.code(), Some(1), "zhang serve exited with {:?}", status);
}
