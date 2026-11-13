//! The app never binds a network socket, with or without `CORVUS_API_TOKEN`.
//! Runs the real app binary, so it needs a GUI session: set `CORVUS_GUI_TESTS=1`.
#![allow(clippy::unwrap_used)]

use std::net::TcpListener;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// The single-instance plugin's rendezvous point (a Unix-domain socket, not a network port).
const SINGLE_INSTANCE_SOCKET: &str = "/tmp/dev_corvus_desktop_si.sock";

fn lsof(pid: u32, proto: &[&str]) -> String {
    let out = Command::new("lsof")
        .args(["-nP", "-a", "-p", &pid.to_string()])
        .args(proto)
        .output()
        .expect("lsof is required for this test");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn listening_tcp(pid: u32) -> String {
    lsof(pid, &["-iTCP", "-sTCP:LISTEN"])
}

fn udp(pid: u32) -> String {
    lsof(pid, &["-iUDP"])
}

struct Killed(Child);
impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run_app(home: &Path, token: Option<&str>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_corvus-desktop"));
    cmd.env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(t) = token {
        cmd.env("CORVUS_API_TOKEN", t);
    }
    let mut app = Killed(cmd.spawn().unwrap());
    let pid = app.0.id();

    // Wait for the app to open its database, then give it time to start everything else.
    let db = home.join(".corvus/corvus.db");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !db.exists() && Instant::now() < deadline {
        assert!(app.0.try_wait().unwrap().is_none(), "the app exited early");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        db.exists(),
        "the app never created its database under the temp HOME"
    );
    std::thread::sleep(Duration::from_secs(3));

    assert!(
        app.0.try_wait().unwrap().is_none(),
        "the app is not running (token: {})",
        token.is_some()
    );
    let (tcp, udp) = (listening_tcp(pid), udp(pid));
    assert!(
        tcp.is_empty(),
        "listening TCP sockets (token: {}):\n{tcp}",
        token.is_some()
    );
    assert!(udp.is_empty(), "UDP sockets (token: {}):\n{udp}", token.is_some());
}

#[test]
fn the_app_binds_no_socket_with_or_without_a_token() {
    if std::env::var("CORVUS_GUI_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIPPED: needs a GUI session; run with CORVUS_GUI_TESTS=1");
        return;
    }
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/dist/index.html");
    if !dist.exists() {
        eprintln!("SKIPPED: ui/dist does not exist; build the UI first (make ui)");
        return;
    }
    if UnixStream::connect(SINGLE_INSTANCE_SOCKET).is_ok() {
        eprintln!("SKIPPED: a Corvus desktop app is already running; quit it first (single instance)");
        return;
    }

    // Positive control: lsof really does see a listener in this very process.
    let control = TcpListener::bind("127.0.0.1:0").unwrap();
    assert!(
        !listening_tcp(std::process::id()).is_empty(),
        "lsof did not report a known listener; the check below would prove nothing"
    );
    drop(control);

    for token in [None, Some(TOKEN)] {
        let home = tempfile::tempdir().unwrap();
        run_app(home.path(), token);
    }
}
