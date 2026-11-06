use crate::error::{Error, Result};
use std::{
    fs::File,
    path::{Path, PathBuf},
    process::Command,
};
pub fn home() -> PathBuf {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| ".".into()))
}
// Outside Windows the only supported target is macOS.
#[cfg(not(windows))]
pub fn app_support() -> PathBuf {
    home().join("Library/Application Support")
}
pub fn data_dir() -> PathBuf {
    std::env::var_os("DLM_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            #[cfg(windows)]
            {
                PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into()))
                    .join("dlm")
            }
            #[cfg(not(windows))]
            {
                app_support().join("Acqua Downloader")
            }
        })
}
/// Folder that holds the browser extension shipped with the app.
pub fn extension_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| Error::new("other", "the app has no install folder"))?;
    #[cfg(windows)]
    return Ok(dir.join("extension"));
    #[cfg(not(windows))]
    return Ok(dir.join("../Resources/extension"));
}
/// macOS runs a quarantined app from a throwaway read-only copy until the user
/// moves it, and the host and login item would point at a path that vanishes.
pub fn misplaced() -> bool {
    #[cfg(not(windows))]
    if let Ok(exe) = std::env::current_exe() {
        let path = exe.to_string_lossy();
        if !path.contains(".app/Contents/MacOS/") {
            return false;
        }
        let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
        let read_only = c_path(&exe).is_ok_and(|p| {
            let status = unsafe { libc::statvfs(p.as_ptr(), &mut stats) };
            status == 0 && stats.f_flag & libc::ST_RDONLY != 0
        });
        return read_only || path.contains("/AppTranslocation/");
    }
    false
}
pub fn hidden_command(exe: impl AsRef<std::ffi::OsStr>) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut c = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    c
}
pub fn spawn_self(arg: &str) -> Result<()> {
    let mut command = hidden_command(std::env::current_exe()?);
    command.arg(arg);
    #[cfg(windows)]
    command.spawn()?;
    #[cfg(unix)]
    {
        use std::{os::unix::process::CommandExt, process::Stdio};
        // The native host's stdio is the browser's protocol channel, and the
        // browser may signal the host's process group when the port closes.
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        let mut child = command.spawn()?;
        std::thread::spawn(move || child.wait());
    }
    Ok(())
}
#[cfg(not(windows))]
pub fn try_lock(file: &File) -> bool {
    use std::os::fd::AsRawFd;
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}
#[cfg(not(windows))]
fn c_path(path: &Path) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| std::io::ErrorKind::InvalidInput.into())
}
pub fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        s.as_ref().encode_wide().chain(Some(0)).collect()
    }
    #[cfg(not(windows))]
    {
        s.as_ref()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect()
    }
}
#[cfg(windows)]
pub fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
    if unsafe {
        MoveFileExW(
            wide(from).as_ptr(),
            wide(to).as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
pub fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    let (source, target) = (c_path(from)?, c_path(to)?);
    if unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if !matches!(error.raw_os_error(), Some(libc::ENOTSUP | libc::EINVAL)) {
        return Err(error);
    }
    // exFAT and network volumes have no exclusive rename: claim the name, then
    // replace the claim. create_new fails with AlreadyExists if the name is taken.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    let moved = std::fs::rename(from, to);
    if moved.is_err() {
        let _ = std::fs::remove_file(to);
    }
    moved
}
pub fn motw(path: &Path, url: &str) -> Result<()> {
    #[cfg(windows)]
    {
        use std::io::Write;
        let mut f = File::create(format!("{}:Zone.Identifier", path.display()))?;
        let safe = url.replace(['\r', '\n'], "");
        write!(f, "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={safe}\r\n")?;
        f.sync_all()?;
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let value = format!("0081;{now:x};Acqua Downloader;");
        let path = c_path(path)?;
        // Same as a browser: Gatekeeper checks this file. Some volumes have no xattrs.
        unsafe {
            libc::setxattr(
                path.as_ptr(),
                c"com.apple.quarantine".as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            );
        }
    }
    Ok(())
}
pub fn sparse(f: &File) {
    // APFS files are sparse by default.
    #[cfg(not(windows))]
    let _ = f;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let mut returned = 0;
        unsafe {
            windows_sys::Win32::System::IO::DeviceIoControl(
                f.as_raw_handle(),
                0x000900c4,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            );
        }
    }
}
pub fn ensure_space(dir: &Path, needed: u64) -> Result<()> {
    #[cfg(windows)]
    let free = {
        let mut free = 0;
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide(dir).as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        free
    };
    #[cfg(not(windows))]
    let free = {
        let path = c_path(dir)?;
        let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(path.as_ptr(), &mut stats) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        stats.f_bavail as u64 * stats.f_frsize
    };
    if free < needed.saturating_add(64 << 20) {
        return Err(Error::new(
            "no_space",
            "not enough free space (64 MiB margin)",
        ));
    }
    Ok(())
}
#[cfg(windows)]
pub fn crypt(bytes: &[u8], decrypt: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        if decrypt {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        } else {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let result = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    unsafe {
        LocalFree(out.pbData as *mut _);
    };
    Ok(result)
}
#[cfg(not(windows))]
const KEYCHAIN_ITEM: [&str; 4] = ["-s", "com.dlm.app", "-a", "download-secrets"];
/// A locked Keychain can wait on a dialog forever and callers hold the manager
/// lock, so every call has a deadline.
#[cfg(not(windows))]
fn security(args: &[&str], input: Option<&str>) -> Option<String> {
    use std::{
        io::{Read, Write},
        process::Stdio,
        time::{Duration, Instant},
    };
    let mut child = Command::new("/usr/bin/security")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let (Some(mut stdin), Some(input)) = (child.stdin.take(), input) {
        let _ = stdin.write_all(input.as_bytes());
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait().ok()? {
            Some(status) if status.success() => break,
            Some(_) => return None,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let mut output = String::new();
    child.stdout.take()?.read_to_string(&mut output).ok()?;
    Some(output)
}
#[cfg(not(windows))]
fn keychain_read() -> Option<[u8; 32]> {
    let mut args = vec!["find-generic-password"];
    args.extend(KEYCHAIN_ITEM);
    args.push("-w");
    let text = security(&args, None)?;
    hex::decode(text.trim()).ok()?.try_into().ok()
}
#[cfg(not(windows))]
fn keychain_add(key: &[u8; 32]) {
    // Through stdin: arguments are visible to every process.
    let line = format!(
        "add-generic-password {} -w {}\n",
        KEYCHAIN_ITEM.join(" "),
        hex::encode(key)
    );
    security(&["-i"], Some(&line));
}
/// Same protection as DPAPI: readable by this user's processes, not by whoever
/// copies the database. `security` owns the item, so an app with no stable
/// signature never triggers a Keychain prompt after an update.
#[cfg(not(windows))]
fn secret_key() -> Result<&'static [u8; 32]> {
    use aes_gcm::{Aes256Gcm, KeyInit, aead::OsRng};
    static KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
    if let Some(key) = KEY.get() {
        return Ok(key);
    }
    let key = match keychain_read() {
        Some(key) => key,
        None => {
            // Never replaces an item: two processes racing here read the winner's key.
            keychain_add(&Aes256Gcm::generate_key(OsRng).into());
            keychain_read()
                .ok_or_else(|| Error::new("keychain", "could not use the macOS Keychain"))?
        }
    };
    Ok(KEY.get_or_init(|| key))
}
#[cfg(not(windows))]
pub fn crypt(bytes: &[u8], decrypt: bool) -> Result<Vec<u8>> {
    use aes_gcm::{
        Aes256Gcm, Key, KeyInit, Nonce,
        aead::{Aead, AeadCore, OsRng},
    };
    const NONCE_SIZE: usize = 12;
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let failed = || Error::new("crypto", "could not read the stored credentials");
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(secret_key()?));
    if decrypt {
        if bytes.len() < NONCE_SIZE {
            return Err(failed());
        }
        let (nonce, data) = bytes.split_at(NONCE_SIZE);
        cipher
            .decrypt(Nonce::from_slice(nonce), data)
            .map_err(|_| failed())
    } else {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let data = cipher.encrypt(&nonce, bytes).map_err(|_| failed())?;
        Ok([nonce.as_slice(), &data].concat())
    }
}
#[cfg(not(windows))]
pub fn launch_agent(exe: &Path) -> String {
    let exe = exe
        .display()
        .to_string()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.dlm.app</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>--tray</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#
    )
}
pub fn autostart(on: bool) -> Result<()> {
    if std::env::var_os("DLM_PORTABLE").is_some() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        use winreg::{RegKey, enums::HKEY_CURRENT_USER};
        let (k, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?;
        if on {
            k.set_value(
                "dlm",
                &format!("\"{}\" --tray", std::env::current_exe()?.display()),
            )?;
        } else {
            match k.delete_value("dlm") {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    #[cfg(not(windows))]
    {
        let path = home().join("Library/LaunchAgents/com.dlm.app.plist");
        if on {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, launch_agent(&std::env::current_exe()?))?;
        } else {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(())
}
pub fn awake(on: bool) {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Power::SetThreadExecutionState(
            0x80000000 | if on { 1 } else { 0 },
        );
    }
    #[cfg(not(windows))]
    {
        use std::{
            process::{Child, Stdio},
            sync::Mutex,
        };
        static HOLD: Mutex<Option<Child>> = Mutex::new(None);
        let mut hold = HOLD.lock().unwrap();
        if on && hold.is_none() {
            // -w ends it with this process, even after a crash.
            *hold = Command::new("/usr/bin/caffeinate")
                .args(["-i", "-w", &std::process::id().to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok();
        } else if !on {
            if let Some(mut child) = hold.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}
pub fn in_use(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(path)
        {
            Ok(_) => false,
            Err(e) => matches!(e.raw_os_error(), Some(32 | 33)),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}
pub fn clipboard() -> Option<String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::{DataExchange::*, Memory::*};
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return None;
            }
            let h = GetClipboardData(13);
            let mut result = None;
            if !h.is_null() {
                let p = GlobalLock(h) as *const u16;
                if !p.is_null() {
                    let max = (GlobalSize(h) / 2).min(1024 * 1024);
                    let data = std::slice::from_raw_parts(p, max);
                    let n = data.iter().position(|c| *c == 0).unwrap_or(max);
                    result = Some(String::from_utf16_lossy(&data[..n]));
                    GlobalUnlock(h);
                }
            }
            CloseClipboard();
            result
        }
    }
    #[cfg(not(windows))]
    {
        let out = Command::new("/usr/bin/pbpaste")
            .env("LANG", "en_US.UTF-8")
            .output()
            .ok()?;
        let n = out.stdout.len().min(1024 * 1024);
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout[..n]).into_owned())
    }
}
pub fn reveal(path: &Path, dir: &Path) -> Result<()> {
    #[cfg(windows)]
    if path.exists() {
        hidden_command("explorer.exe")
            .arg("/select,")
            .arg(path)
            .spawn()?;
    } else {
        hidden_command("explorer.exe").arg(dir).spawn()?;
    }
    #[cfg(not(windows))]
    {
        let mut open = Command::new("/usr/bin/open");
        if path.exists() {
            open.arg("-R").arg(path);
        } else {
            open.arg(dir);
        }
        open.status()?;
    }
    Ok(())
}
pub fn attach_console() {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(u32::MAX);
    }
}
pub fn watch_network(callback: impl Fn() -> bool + Send + 'static) {
    #[cfg(windows)]
    std::thread::spawn(move || {
        #[link(name = "iphlpapi")]
        unsafe extern "system" {
            fn NotifyAddrChange(
                handle: *mut *mut std::ffi::c_void,
                overlapped: *const std::ffi::c_void,
            ) -> u32;
        }
        while unsafe { NotifyAddrChange(std::ptr::null_mut(), std::ptr::null()) } == 0 {
            if !callback() {
                break;
            }
        }
    });
    #[cfg(not(windows))]
    std::thread::spawn(move || {
        // connect() on UDP sends nothing: it only asks which address routes out.
        let route = || {
            let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
            socket.connect("1.1.1.1:53").ok()?;
            Some(socket.local_addr().ok()?.ip())
        };
        let mut last = route();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let now = route();
            if now != last {
                last = now;
                if !callback() {
                    break;
                }
            }
        }
    });
}
