//! `zhang serve` stops gracefully on SIGTERM, as on SIGINT: `docker stop`, Kubernetes and systemd send SIGTERM, and a
//! server that did not listen to it was killed once their timeout passed (10 seconds for `docker stop`, exit code 137).

#![cfg(unix)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A port no one listens on now
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Waits until `child` takes connections on `port`; panics when it exits or takes too long.
fn wait_until_listening(child: &mut Child, port: u16) {
    let started = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("zhang serve exited with {:?} before it listened", status);
        }
        if started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("zhang serve did not listen on {} within a minute", port);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Opens the event stream of the server on `port`, a connection that stays open until the server ends it; the stream,
/// once the headers of the answer arrived.
fn open_event_stream(port: u16) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(stream, "GET /api/sse HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/event-stream\r\n\r\n").unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        assert_eq!(stream.read(&mut byte).unwrap(), 1, "the server closed the event stream before its headers");
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head);
    assert!(head.starts_with("HTTP/1.1 200"), "{}", head);
    stream
}

/// Starts `zhang serve` on `dir` with `args` added, and waits until it takes connections; the process and its port.
fn serve(dir: &std::path::Path, args: &[&str]) -> (Child, u16) {
    let port = free_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_zhang"))
        .args([
            "serve",
            dir.to_str().unwrap(),
            "--addr",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--no-report",
        ])
        .args(args)
        .env("ZHANG_LOG", "info")
        .env_remove("RUST_LOG")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_until_listening(&mut child, port);
    (child, port)
}

/// Sends SIGTERM to `child`.
fn terminate(child: &Child) {
    let killed = Command::new("kill").args(["-TERM", &child.id().to_string()]).status().unwrap();
    assert!(killed.success());
}

/// Waits until `child` exits, at most `limit` from `since`; its exit code, how long after `since` it exited, and its
/// stderr.
fn wait_for_exit(mut child: Child, since: Instant, limit: Duration) -> (Option<i32>, Duration, String) {
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if since.elapsed() > limit {
            let _ = child.kill();
            let mut stderr = String::new();
            child.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
            panic!("zhang serve was still running {:?} after the signal:\n{}", limit, stderr);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let took = since.elapsed();
    let mut stderr = String::new();
    child.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
    (status.code(), took, stderr)
}

/// A ledger with a router plugin that never answers: `GET /api/plugins/spin/` runs on a blocking thread until the
/// plugin's timeout (30 seconds by default), as a reload with plugins or a long query does.
fn ledger_with_a_spinning_plugin() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.zhang"),
        "option \"operating_currency\" \"CNY\"\noption \"features.plugin\" \"true\"\nplugin \"spin.lua\"\n1970-01-01 commodity CNY\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("spin.lua"),
        "local M = { name = \"spin\", version = \"0.1.0\", types = { \"Router\" } }\nfunction M.router(request)\n  while true do end\nend\nreturn M\n",
    )
    .unwrap();
    dir
}

/// Sends `GET /api/plugins/spin/` to the server on `port`, and waits a moment for the plugin to run; the connection,
/// which gets no answer.
fn start_spinning(port: u16) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(stream, "GET /api/plugins/spin/ HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n", port).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    stream
}

#[test]
fn serve_stops_gracefully_on_sigterm() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.zhang"),
        "option \"operating_currency\" \"CNY\"\n1970-01-01 commodity CNY\n",
    )
    .unwrap();
    let (child, port) = serve(dir.path(), &["--plugins", "off"]);
    // a client of the web UI holds its event stream open: the stop must end it rather than wait for it
    let mut events = open_event_stream(port);

    let stopping = Instant::now();
    terminate(&child);
    let (code, took, stderr) = wait_for_exit(child, stopping, Duration::from_secs(20));

    assert_eq!(code, Some(0), "zhang serve exited with {code:?} on SIGTERM:\n{stderr}");
    assert!(took < Duration::from_secs(4), "zhang serve took {:?} to stop:\n{}", took, stderr);
    assert!(stderr.contains("received SIGTERM"), "the stop is not logged:\n{}", stderr);
    // the event stream was ended by the server, not cut by the exit
    let mut rest = Vec::new();
    events.read_to_end(&mut rest).expect("the event stream ends");
    assert!(
        rest.ends_with(b"0\r\n\r\n"),
        "the event stream was not ended: {:?}",
        String::from_utf8_lossy(&rest)
    );
}

/// Blocking work that outlives the grace of a stop, here a plugin call that runs until its 30 seconds timeout, does not
/// hold the exit: the server waits for the request [`zhang_server::STOP_GRACE`], the process for the blocking thread
/// [`zhang_server::EXIT_GRACE`], then exits.
#[test]
fn a_stop_does_not_wait_for_blocking_work_past_its_grace() {
    let dir = ledger_with_a_spinning_plugin();
    let (child, port) = serve(dir.path(), &[]);
    let _spinning = start_spinning(port);

    let stopping = Instant::now();
    terminate(&child);
    let (code, took, stderr) = wait_for_exit(child, stopping, Duration::from_secs(25));

    let cap = zhang_server::STOP_GRACE + zhang_server::EXIT_GRACE;
    assert_eq!(code, Some(0), "zhang serve exited with {code:?} on SIGTERM:\n{stderr}");
    assert!(
        took < cap + Duration::from_secs(2),
        "zhang serve took {:?} to stop, more than {:?}:\n{}",
        took,
        cap,
        stderr
    );
    assert!(stderr.contains("connections were still open"), "the request left is not logged:\n{}", stderr);
}

/// A second signal while the stop waits exits at once, with the code of a process SIGTERM ended.
#[test]
fn a_second_signal_exits_at_once() {
    let dir = ledger_with_a_spinning_plugin();
    let (child, port) = serve(dir.path(), &[]);
    let _spinning = start_spinning(port);

    terminate(&child);
    std::thread::sleep(Duration::from_millis(300));
    let again = Instant::now();
    terminate(&child);
    let (code, took, stderr) = wait_for_exit(child, again, Duration::from_secs(25));

    assert_eq!(code, Some(143), "zhang serve exited with {code:?} on the second SIGTERM:\n{stderr}");
    assert!(
        took < Duration::from_secs(2),
        "zhang serve took {:?} to exit on the second SIGTERM:\n{}",
        took,
        stderr
    );
    assert!(stderr.contains("received SIGTERM again"), "the second signal is not logged:\n{}", stderr);
}
