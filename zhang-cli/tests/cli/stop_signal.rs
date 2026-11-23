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

#[test]
fn serve_stops_gracefully_on_sigterm() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.zhang"),
        "option \"operating_currency\" \"CNY\"\n1970-01-01 commodity CNY\n",
    )
    .unwrap();
    let port = free_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_zhang"))
        .args([
            "serve",
            dir.path().to_str().unwrap(),
            "--addr",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--no-report",
            "--plugins",
            "off",
        ])
        .env("ZHANG_LOG", "info")
        .env_remove("RUST_LOG")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_until_listening(&mut child, port);
    // a client of the web UI holds its event stream open: the stop must end it rather than wait for it
    let mut events = open_event_stream(port);

    let stopping = Instant::now();
    let killed = Command::new("kill").args(["-TERM", &child.id().to_string()]).status().unwrap();
    assert!(killed.success());
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if stopping.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            panic!("zhang serve was still running 20 seconds after SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let took = stopping.elapsed();
    let mut stderr = String::new();
    child.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();

    assert_eq!(status.code(), Some(0), "zhang serve exited with {status:?} on SIGTERM:\n{stderr}");
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
