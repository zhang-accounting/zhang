//! When the ledger cannot be loaded, `zhang serve` must say why on stderr, whatever the log filter is (#491).
//! Before, the reason was only logged, so without `RUST_LOG` the command exited with code 1 and no message.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Runs `zhang serve` on a ledger whose second line cannot be parsed, with these log variables and no other,
/// and waits for it to exit.
fn serve_broken_ledger(log_env: &[(&str, &str)]) -> Output {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.zhang"), "2024-01-01 open Assets:A CNY\n2024-01-02 broken line here\n").unwrap();
    serve(dir.path(), &[], log_env)
}

/// Runs `zhang serve` on the ledger in `dir` with the arguments `args`, and these log variables and no other, and
/// waits for it to exit.
fn serve(dir: &Path, args: &[&str], log_env: &[(&str, &str)]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_zhang"))
        .args(["serve", dir.to_str().unwrap(), "--addr", "127.0.0.1", "--port", "0", "--no-report"])
        .args(args)
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
    child.wait_with_output().unwrap()
}

fn assert_names_the_file_and_the_line(output: &Output, log_env: &[(&str, &str)]) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{log_env:?}: zhang serve exited with {:?}", output.status);
    assert!(
        stderr.contains("main.zhang") && stderr.contains("line 2"),
        "{:?}: stderr does not name the file and the line:\n{}",
        log_env,
        stderr
    );
}

#[test]
fn a_ledger_that_fails_to_load_is_explained_on_stderr() {
    let output = serve_broken_ledger(&[]);
    assert_names_the_file_and_the_line(&output, &[]);
}

#[test]
fn the_reason_is_printed_whatever_the_log_filter_is() {
    for log_env in [[("RUST_LOG", "off")], [("ZHANG_LOG", "off")]] {
        let output = serve_broken_ledger(&log_env);
        assert_names_the_file_and_the_line(&output, &log_env);
    }
}

/// A file that is not UTF-8 text, as one with a latin-1 `é` in a comment, is a load error naming the file and the
/// line: `zhang serve` panicked on it, with exit code 101 and a dump of the file's bytes.
#[test]
fn a_file_that_is_not_utf8_is_explained_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.zhang"), "2024-01-01 open Assets:A CNY\ninclude \"notes.zhang\"\n").unwrap();
    std::fs::write(dir.path().join("notes.zhang"), b"; \xe9t\xe9\n").unwrap();

    let output = serve(dir.path(), &[], &[]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "zhang serve exited with {:?}: {}", output.status, stderr);
    assert!(
        stderr.contains("error: the file notes.zhang is not UTF-8 text: line 1"),
        "stderr does not name the file and the line:\n{}",
        stderr
    );
}
