//! `ZHANG_LOG` sets the log filter, `RUST_LOG` is the fallback and `info` the default (#491). Before, the binary
//! read `RUST_LOG` only and logged nothing without it, so `ZHANG_LOG` had no effect.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The line `zhang serve` logs at `info`, from the `zhang` module, before it loads the ledger
const INFO_LINE: &str = "active file system is Fs";

/// Runs `zhang serve` on a ledger that cannot be parsed, so it exits on its own, with these log variables and no
/// other; its stderr.
fn stderr_of(log_env: &[(&str, &str)]) -> String {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.zhang"), "2024-01-01 open Assets:A CNY\n2024-01-02 broken line here\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_zhang"))
        .args(["serve", dir.path().to_str().unwrap(), "--addr", "127.0.0.1", "--port", "0", "--no-report"])
        .env_remove("ZHANG_LOG")
        .env_remove("RUST_LOG")
        .envs(log_env.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("zhang serve kept running although its ledger could not be loaded");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{log_env:?}: zhang serve exited with {:?}", output.status);
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn zhang_log_sets_the_filter() {
    let stderr = stderr_of(&[("ZHANG_LOG", "debug")]);
    assert!(stderr.contains(INFO_LINE), "ZHANG_LOG=debug logs nothing at info:\n{}", stderr);

    // in env_logger syntax, a module filter included
    let stderr = stderr_of(&[("ZHANG_LOG", "zhang=info")]);
    assert!(stderr.contains(INFO_LINE), "ZHANG_LOG=zhang=info logs nothing from zhang:\n{}", stderr);
    let stderr = stderr_of(&[("ZHANG_LOG", "zhang_core=trace")]);
    assert!(!stderr.contains(INFO_LINE), "ZHANG_LOG=zhang_core=trace logs the zhang module:\n{}", stderr);

    // it wins over RUST_LOG
    let stderr = stderr_of(&[("ZHANG_LOG", "error"), ("RUST_LOG", "info")]);
    assert!(!stderr.contains(INFO_LINE), "RUST_LOG=info is read although ZHANG_LOG is set:\n{}", stderr);
}

#[test]
fn rust_log_is_the_fallback() {
    let stderr = stderr_of(&[("RUST_LOG", "info")]);
    assert!(stderr.contains(INFO_LINE), "RUST_LOG=info logs nothing at info:\n{}", stderr);
    let stderr = stderr_of(&[("RUST_LOG", "error")]);
    assert!(!stderr.contains(INFO_LINE), "RUST_LOG=error logs at info:\n{}", stderr);
}

#[test]
fn the_default_filter_is_info() {
    let stderr = stderr_of(&[]);
    assert!(
        stderr.contains(INFO_LINE),
        "without ZHANG_LOG and RUST_LOG nothing is logged at info:\n{}",
        stderr
    );
}
