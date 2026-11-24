//! The time zone of a ledger without `option "timezone"`: the `TZ` environment variable first, so that
//! `docker run -e TZ=Europe/Berlin` works in the image, which has no system time zone; a `TZ` that is not an IANA name
//! is ignored with a warning. The ledger's option wins over `TZ`. The system's time zone and the UTC fallback, which
//! a test cannot take away from the machine, are tested in `zhang_core::options`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The timezone the server reports, and what it logged
struct Start {
    timezone: String,
    output: String,
}

/// Runs `zhang serve` on a ledger of `main` with the environment variable `TZ` set to `tz` (not set: `None`), until it
/// takes connections; the `timezone` option `GET /api/options` gives, and the output of the server, which is then killed.
fn start(main: &str, tz: Option<&str>) -> Start {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.zhang"), main).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut command = Command::new(env!("CARGO_BIN_EXE_zhang"));
    command
        .args([
            "serve",
            dir.path().to_str().unwrap(),
            "--addr",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--no-report",
        ])
        .args(["--plugins", "off"])
        .env_remove("ZHANG_LOG")
        .env("RUST_LOG", "info")
        .env_remove("TZ")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(tz) = tz {
        command.env("TZ", tz);
    }
    let mut child = command.spawn().unwrap();
    let started = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("TZ={:?}: zhang serve exited with {:?} before it listened", tz, status);
        }
        if started.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            panic!("TZ={:?}: zhang serve did not listen within two minutes", tz);
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(stream, "GET /api/options HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();
    let output = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));

    let (_, body) = response
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("TZ={:?}: no HTTP response: {}", tz, response));
    let body: serde_json::Value = serde_json::from_str(body).unwrap_or_else(|e| panic!("TZ={:?}: {}: {}", tz, e, body));
    let options = body["data"].as_array().unwrap_or_else(|| panic!("TZ={:?}: no options: {}", tz, body));
    let timezone = options
        .iter()
        .find(|it| it["key"] == "timezone")
        .unwrap_or_else(|| panic!("TZ={:?}: no timezone: {}", tz, body));
    Start {
        timezone: timezone["value"].as_str().unwrap().to_owned(),
        output,
    }
}

const WITHOUT_OPTION: &str = "option \"operating_currency\" \"CNY\"\n1970-01-01 open Assets:Cash CNY\n";

#[test]
fn tz_sets_the_timezone_of_a_ledger_without_the_option() {
    let start = start(WITHOUT_OPTION, Some("Asia/Tokyo"));
    assert_eq!(start.timezone, "Asia/Tokyo", "{}", start.output);
    assert!(
        start
            .output
            .contains("a ledger without option \"timezone\" is in Asia/Tokyo, from the TZ environment variable"),
        "{}",
        start.output
    );

    // with the leading colon the C library accepts
    assert_eq!(start_timezone(WITHOUT_OPTION, Some(":Europe/Berlin")), "Europe/Berlin");
    assert_eq!(start_timezone(WITHOUT_OPTION, Some("UTC")), "UTC");
}

/// The timezone a start reports
fn start_timezone(main: &str, tz: Option<&str>) -> String {
    start(main, tz).timezone
}

#[test]
fn a_tz_that_is_not_an_iana_name_is_ignored_with_a_warning() {
    // the system's time zone, or UTC: what a start without TZ takes, whatever the machine's is
    let without = start_timezone(WITHOUT_OPTION, None);
    for tz in ["UTC0", "Mars/Olympus"] {
        let start = start(WITHOUT_OPTION, Some(tz));
        assert!(
            start.output.contains(&format!("the TZ environment variable \"{tz}\" is ignored")),
            "TZ={tz}: no warning:\n{}",
            start.output
        );
        assert_eq!(start.timezone, without, "TZ={tz}: {}", start.output);
    }
}

#[test]
fn the_ledgers_option_wins_over_tz() {
    let main = format!("option \"timezone\" \"America/New_York\"\n{WITHOUT_OPTION}");
    let start = start(&main, Some("Asia/Tokyo"));
    assert_eq!(start.timezone, "America/New_York", "{}", start.output);
    // the default is not even looked for
    assert!(!start.output.contains("TZ environment variable"), "{}", start.output);
}

#[test]
fn an_invalid_option_falls_back_to_tz() {
    let main = format!("option \"timezone\" \"Mars/Olympus\"\n{WITHOUT_OPTION}");
    let start = start(&main, Some("Asia/Tokyo"));
    assert_eq!(start.timezone, "Asia/Tokyo", "{}", start.output);
    assert!(
        start.output.contains("option \"timezone\" \"Mars/Olympus\" is not a valid IANA time zone name"),
        "{}",
        start.output
    );
}
