#![cfg(target_os = "macos")]
use dlm::system;
use std::process::Command;

fn acqua() -> Command {
    Command::new(env!("CARGO_BIN_EXE_acqua"))
}

#[test]
fn secrets_are_encrypted_and_survive_a_new_process_key_lookup() {
    let first = system::crypt(b"session=secret", false).unwrap();
    let second = system::crypt(b"session=secret", false).unwrap();
    assert_ne!(first, second, "every value needs its own nonce");
    assert!(!first.windows(6).any(|w| w == b"secret"));
    assert_eq!(system::crypt(&first, true).unwrap(), b"session=secret");
    let mut tampered = first.clone();
    *tampered.last_mut().unwrap() ^= 1;
    assert!(system::crypt(&tampered, true).is_err());
    assert!(system::crypt(b"short", true).is_err());
}

#[test]
fn download_gets_quarantine_flag() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.bin");
    std::fs::write(&path, b"data").unwrap();
    system::motw(&path, "https://example.com/file.bin").unwrap();
    let out = Command::new("/usr/bin/xattr")
        .args(["-p", "com.apple.quarantine"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let value = String::from_utf8(out.stdout).unwrap();
    assert!(value.starts_with("0081;"), "{value}");
    assert!(value.trim_end().ends_with(";Acqua Downloader;"), "{value}");
}

#[test]
fn rename_never_replaces_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let (from, to) = (dir.path().join("from"), dir.path().join("to"));
    std::fs::write(&from, b"new").unwrap();
    std::fs::write(&to, b"old").unwrap();
    let error = system::rename_no_replace(&from, &to).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&to).unwrap(), b"old");
    std::fs::remove_file(&to).unwrap();
    system::rename_no_replace(&from, &to).unwrap();
    assert_eq!(std::fs::read(&to).unwrap(), b"new");
    assert!(!from.exists());
}

#[test]
fn free_space_is_measured() {
    let dir = tempfile::tempdir().unwrap();
    system::ensure_space(dir.path(), 1).unwrap();
    let error = system::ensure_space(dir.path(), u64::MAX / 2).unwrap_err();
    assert_eq!(error.code, "no_space");
}

#[test]
fn login_item_is_a_valid_plist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.plist");
    let exe = std::path::Path::new("/Applications/R&D <Acqua>.app/Contents/MacOS/acqua");
    std::fs::write(&path, system::launch_agent(exe)).unwrap();
    let lint = Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(&path)
        .output()
        .unwrap();
    assert!(lint.status.success(), "{lint:?}");
    let out = Command::new("/usr/bin/plutil")
        .args(["-extract", "ProgramArguments.0", "raw", "-o", "-"])
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim(),
        exe.display().to_string()
    );
}

#[test]
fn clipboard_text_round_trips() {
    use std::{io::Write, process::Stdio};
    let mut copy = Command::new("/usr/bin/pbcopy")
        .env("LANG", "en_US.UTF-8")
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    copy.stdin
        .take()
        .unwrap()
        .write_all("https://example.com/água.zip".as_bytes())
        .unwrap();
    copy.wait().unwrap();
    assert_eq!(
        system::clipboard().as_deref(),
        Some("https://example.com/água.zip")
    );
}

#[test]
fn host_manifest_reaches_browsers_installed_later() {
    let home = tempfile::tempdir().unwrap();
    let support = home.path().join("Library/Application Support");
    let chrome = support.join("Google/Chrome");
    let run = |action: &str| {
        let status = acqua()
            .arg(action)
            .env("HOME", home.path())
            .env("DLM_DATA_DIR", home.path().join("data"))
            .status()
            .unwrap();
        assert!(status.success(), "{action}");
    };
    run("register-host");
    let manifest = chrome.join("NativeMessagingHosts/com.dlm.host.json");
    let host: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(host["path"], env!("CARGO_BIN_EXE_acqua"));
    assert_eq!(host["allowed_origins"][0], dlm::native::ORIGIN);
    let edge = support.join("Microsoft Edge/NativeMessagingHosts/com.dlm.host.json");
    assert!(edge.exists());
    run("unregister-host");
    assert!(!manifest.exists());
    assert!(!edge.exists());
}

#[test]
fn second_daemon_refuses_and_stale_socket_is_replaced() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
    let socket = dir.path().join("dlm-lock.sock");
    std::fs::write(&socket, b"left by a crash").unwrap();
    let start = || {
        acqua()
            .arg("--daemon")
            .env("DLM_DATA_DIR", dir.path())
            .env("DLM_PIPE_SUFFIX", "lock")
            .env("DLM_PORTABLE", "1")
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };
    let mut first = start();
    let begin = std::time::Instant::now();
    while std::os::unix::net::UnixStream::connect(&socket).is_err() {
        assert!(begin.elapsed().as_secs() < 10, "daemon did not start");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let mode =
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&socket).unwrap().permissions());
    assert_eq!(mode & 0o777, 0o600);
    let mut second = start();
    assert!(!second.wait().unwrap().success());
    let folder = std::fs::metadata(dir.path()).unwrap().permissions().mode();
    assert_eq!(folder & 0o777, 0o750, "a chosen folder keeps its mode");
    first.kill().unwrap();
    first.wait().unwrap();
}
