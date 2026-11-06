use crate::error::{Error, Result};
use std::{
    io::{Read, Write},
    sync::Arc,
};
pub const MAX_MESSAGE: usize = 1 << 20;
pub fn read_msg(r: &mut impl Read) -> Result<Vec<u8>> {
    let mut len = [0; 4];
    r.read_exact(&mut len)?;
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_MESSAGE {
        return Err(Error::new("ipc", "message too large"));
    }
    let mut bytes = vec![0; n];
    r.read_exact(&mut bytes)?;
    Ok(bytes)
}
pub fn write_msg(w: &mut impl Write, bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_MESSAGE {
        return Err(Error::new("ipc", "message too large"));
    }
    w.write_all(&(bytes.len() as u32).to_le_bytes())?;
    w.write_all(bytes)?;
    w.flush()?;
    Ok(())
}
pub fn running() -> bool {
    call(&serde_json::json!({"type":"ping"})).is_ok()
}
pub fn pipe_suffix() -> String {
    std::env::var("DLM_PIPE_SUFFIX")
        .ok()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .map(|s| format!("-{s}"))
        .unwrap_or_default()
}
fn timeout_secs(value: &serde_json::Value) -> u64 {
    if value["type"] == "ping" { 1 } else { 120 }
}
fn respond(
    stream: &mut (impl Read + Write),
    handler: &Arc<dyn Fn(serde_json::Value) -> serde_json::Value + Send + Sync>,
) {
    if let Ok(raw) = read_msg(stream) {
        let response = serde_json::from_slice(&raw)
            .map(|r| handler(r))
            .unwrap_or_else(|e| serde_json::json!({"ok":false,"error":e.to_string()}));
        if let Ok(bytes) = serde_json::to_vec(&response) {
            let _ = write_msg(stream, &bytes);
        }
    }
}
#[cfg(windows)]
pub fn pipe_name() -> Result<String> {
    Ok(format!(r"\\.\pipe\dlm-{}{}", sid()?, pipe_suffix()))
}
#[cfg(windows)]
struct TimedPipe {
    file: std::fs::File,
    deadline: std::time::Instant,
}
#[cfg(windows)]
impl Read for TimedPipe {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        use std::os::windows::io::AsRawHandle;
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            let mut available = 0;
            if unsafe {
                windows_sys::Win32::System::Pipes::PeekNamedPipe(
                    self.file.as_raw_handle(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if available > 0 {
                let n = buffer.len().min(available as usize);
                return self.file.read(&mut buffer[..n]);
            }
            if std::time::Instant::now() >= self.deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "named pipe did not respond",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}
#[cfg(windows)]
impl Write for TimedPipe {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.file.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
#[cfg(windows)]
pub fn sid() -> Result<String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::Authorization::ConvertSidToStringSidW,
        Security::*,
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut size = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
        let mut data = vec![0u64; (size as usize).div_ceil(8)];
        let ok = GetTokenInformation(token, TokenUser, data.as_mut_ptr().cast(), size, &mut size);
        CloseHandle(token);
        if ok == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let user = &*(data.as_ptr() as *const TOKEN_USER);
        let mut text = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut n = 0;
        while *text.add(n) != 0 {
            n += 1
        }
        let result = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
        LocalFree(text.cast());
        Ok(result)
    }
}
#[cfg(windows)]
pub fn call(value: &serde_json::Value) -> Result<serde_json::Value> {
    use std::os::windows::fs::OpenOptionsExt;
    let path = pipe_name()?;
    let mut attempts = 0;
    let file = loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(0x00100000)
            .open(&path)
        {
            Ok(f) => break f,
            Err(e) if e.raw_os_error() == Some(231) && attempts < 30 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => return Err(Error::new("not_running", e.to_string())),
        }
    };
    let mut f = TimedPipe {
        file,
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs(value)),
    };
    write_msg(&mut f, &serde_json::to_vec(value)?)?;
    Ok(serde_json::from_slice(&read_msg(&mut f)?)?)
}
#[cfg(windows)]
pub fn serve(
    handler: Arc<dyn Fn(serde_json::Value) -> serde_json::Value + Send + Sync>,
) -> Result<()> {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::{
        Foundation::{ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE, LocalFree},
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        },
        Storage::FileSystem::FILE_FLAG_FIRST_PIPE_INSTANCE,
        System::Pipes::*,
    };
    let sid = sid()?;
    let name = crate::system::wide(pipe_name()?);
    let sddl = crate::system::wide(format!("D:P(A;;GA;;;{sid})"));
    let make = move |first: bool| -> Result<std::fs::File> {
        unsafe {
            let mut descriptor = std::ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            let sa = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            let h = CreateNamedPipeW(
                name.as_ptr(),
                3 | if first {
                    FILE_FLAG_FIRST_PIPE_INSTANCE
                } else {
                    0
                },
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                255,
                65536,
                65536,
                1000,
                &sa,
            );
            LocalFree(descriptor);
            if h == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(std::fs::File::from_raw_handle(h))
        }
    };
    let first = make(true)?;
    std::thread::spawn(move || {
        use std::os::windows::io::AsRawHandle;
        let mut next = Some(first);
        loop {
            let f = match next.take().map(Ok).unwrap_or_else(|| make(false)) {
                Ok(f) => f,
                Err(_) => break,
            };
            let h = f.as_raw_handle();
            let ok = unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) };
            if ok == 0
                && std::io::Error::last_os_error().raw_os_error()
                    != Some(ERROR_PIPE_CONNECTED as i32)
            {
                continue;
            }
            let handle = handler.clone();
            std::thread::spawn(move || {
                let mut f = TimedPipe {
                    file: f,
                    deadline: std::time::Instant::now() + std::time::Duration::from_secs(120),
                };
                respond(&mut f, &handle);
            });
        }
    });
    Ok(())
}
#[cfg(unix)]
pub fn socket_path() -> std::path::PathBuf {
    crate::system::data_dir().join(format!("dlm{}.sock", pipe_suffix()))
}
#[cfg(unix)]
fn same_user(stream: &std::os::unix::net::UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (0, 0);
    unsafe {
        libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) == 0 && uid == libc::geteuid()
    }
}
#[cfg(unix)]
pub fn call(value: &serde_json::Value) -> Result<serde_json::Value> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket_path())
        .map_err(|e| Error::new("not_running", e.to_string()))?;
    let timeout = Some(std::time::Duration::from_secs(timeout_secs(value)));
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    write_msg(&mut stream, &serde_json::to_vec(value)?)?;
    Ok(serde_json::from_slice(&read_msg(&mut stream)?)?)
}
#[cfg(unix)]
pub fn serve(
    handler: Arc<dyn Fn(serde_json::Value) -> serde_json::Value + Send + Sync>,
) -> Result<()> {
    use std::{
        fs::{self, Permissions},
        os::unix::{fs::PermissionsExt, net::UnixListener},
        time::Duration,
    };
    let path = socket_path();
    let dir = crate::system::data_dir();
    fs::create_dir_all(&dir)?;
    if std::env::var_os("DLM_DATA_DIR").is_none() {
        fs::set_permissions(&dir, Permissions::from_mode(0o700))?;
    }
    // Held until the process exits, so one daemon runs and a crash leaves no lock.
    let lock = fs::File::create(path.with_extension("lock"))?;
    if !crate::system::try_lock(&lock) {
        return Err(Error::new("already_running", "another instance is running"));
    }
    std::mem::forget(lock);
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, Permissions::from_mode(0o600))?;
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            // Only this user, like the pipe's ACL on Windows. Modes alone do not
            // cover an inherited macOS ACL.
            if !same_user(&stream) {
                continue;
            }
            let handle = handler.clone();
            std::thread::spawn(move || {
                let timeout = Some(Duration::from_secs(120));
                let _ = stream.set_read_timeout(timeout);
                let _ = stream.set_write_timeout(timeout);
                respond(&mut stream, &handle);
            });
        }
    });
    Ok(())
}
