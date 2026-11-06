#![cfg(windows)]
use dlm::{
    ipc,
    testserver::{Server, hash},
};
use serde_json::{Value, json};
use std::{
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Guard(Child);
fn test_exe() -> String {
    std::env::var("DLM_TEST_EXE").unwrap_or_else(|_| env!("CARGO_BIN_EXE_dlm").into())
}
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn spawn(args: &[&str]) -> Guard {
    Guard(
        Command::new(test_exe())
            .args(args)
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}
fn call(path: &str, value: Value) -> Value {
    let start = Instant::now();
    let mut f = loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
        {
            Ok(f) => break f,
            Err(e) if e.raw_os_error() == Some(231) && start.elapsed() < Duration::from_secs(2) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(e) => panic!("{e}"),
        }
    };
    ipc::write_msg(&mut f, &serde_json::to_vec(&value).unwrap()).unwrap();
    serde_json::from_slice(&ipc::read_msg(&mut f).unwrap()).unwrap()
}
fn wait(path: &str) {
    let start = Instant::now();
    while std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .is_err()
    {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(50));
    }
}
#[test]
fn daemon_ipc_persistence_and_native_origin() {
    let dir = tempfile::tempdir().unwrap();
    let suffix = format!("test-{}", std::process::id());
    let path = format!(r"\\.\pipe\dlm-{}-{suffix}", ipc::sid().unwrap());
    let server = Server::start(100000).unwrap();
    let mut daemon = Guard(
        Command::new(test_exe())
            .arg("--daemon")
            .env("DLM_DATA_DIR", dir.path())
            .env("DLM_PIPE_SUFFIX", &suffix)
            .env("DLM_PORTABLE", "1")
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait(&path);
    assert_eq!(call(&path, json!({"type":"ping"}))["ok"], true);
    let mut native = Command::new(test_exe())
        .arg(dlm::native::ORIGIN)
        .env("DLM_DATA_DIR", dir.path())
        .env("DLM_PIPE_SUFFIX", &suffix)
        .env("DLM_PORTABLE", "1")
        .creation_flags(0x08000000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    ipc::write_msg(native.stdin.as_mut().unwrap(), br#"{"type":"ping"}"#).unwrap();
    let response: Value =
        serde_json::from_slice(&ipc::read_msg(native.stdout.as_mut().unwrap()).unwrap()).unwrap();
    assert_eq!(response["ok"], true, "{response}");
    ipc::write_msg(
        native.stdin.as_mut().unwrap(),
        br#"{"type":"rpc","method":"Shutdown","args":[]}"#,
    )
    .unwrap();
    let denied: Value =
        serde_json::from_slice(&ipc::read_msg(native.stdout.as_mut().unwrap()).unwrap()).unwrap();
    assert_eq!(denied["ok"], false);
    drop(native.stdin.take());
    assert!(native.wait().unwrap().success());
    let result = call(
        &path,
        json!({"type":"rpc","method":"Add","args":[{"url":server.url("/full.bin"),"dir":dir.path(),"name":"ipc.bin"}]}),
    );
    assert_eq!(result["ok"], true, "{result}");
    let start = Instant::now();
    loop {
        let tasks = call(&path, json!({"type":"rpc","method":"List","args":[]}));
        if tasks["result"][0]["state"] == "done" {
            assert_eq!(tasks["result"][0]["sha256"], hash(server.size));
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(15), "{tasks}");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        call(&path, json!({"type":"rpc","method":"Shutdown","args":[]}))["ok"],
        true
    );
    assert!(daemon.0.wait().unwrap().success());
    let mut host = Command::new(test_exe())
        .arg("chrome-extension://untrusted/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .creation_flags(0x08000000)
        .spawn()
        .unwrap();
    let response: Value =
        serde_json::from_slice(&ipc::read_msg(host.stdout.as_mut().unwrap()).unwrap()).unwrap();
    assert_eq!(response["ok"], false);
    assert!(!host.wait().unwrap().success());
}
#[test]
fn cli_exit_codes_and_hash() {
    let s = Server::start(40000).unwrap();
    let temp = tempfile::tempdir().unwrap();
    for (path, extra, code) in [
        ("/full.bin", vec![], 0),
        ("/expired.bin", vec![], 3),
        (
            "/full.bin",
            vec![
                "-hash",
                "0000000000000000000000000000000000000000000000000000000000000000",
            ],
            4,
        ),
    ] {
        let mut cmd = Command::new(test_exe());
        cmd.args([
            "get",
            &s.url(path),
            "-o",
            temp.path().to_str().unwrap(),
            "-name",
            &format!("file-{code}.bin"),
            "-q",
        ])
        .args(extra)
        .creation_flags(0x08000000);
        let out = cmd.output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let out = Command::new(test_exe())
        .args(["get", "ftp://example.com/file", "-q"])
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
#[test]
fn abrupt_process_kills_resume_without_corruption() {
    let s = Server::start(32 << 20).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_str().unwrap();
    let jp = temp.path().join("crash.bin.dlm");
    for _ in 0..12 {
        let mut process = spawn(&[
            "get",
            &s.url("/slow.bin"),
            "-o",
            dir,
            "-name",
            "crash.bin",
            "-c",
            "4",
            "-q",
        ]);
        std::thread::sleep(Duration::from_millis(160));
        process.0.kill().ok();
        process.0.wait().ok();
        if temp.path().join("crash.bin").exists() {
            break;
        }
    }
    let status = Command::new(test_exe())
        .args([
            "get",
            &s.url("/slow.bin"),
            "-o",
            dir,
            "-name",
            "crash.bin",
            "-q",
        ])
        .creation_flags(0x08000000)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let data = std::fs::read(temp.path().join("crash.bin")).unwrap();
    use sha2::Digest;
    assert_eq!(hex::encode(sha2::Sha256::digest(data)), hash(s.size));
    assert!(!Path::new(&jp).exists());
}
