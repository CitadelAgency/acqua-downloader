use crate::error::{Error, Result};
use std::{
    fs::File,
    path::{Path, PathBuf},
    process::{Child, Command},
};
pub fn data_dir() -> PathBuf {
    std::env::var_os("DLM_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into()))
                .join("dlm")
        })
}
pub fn hidden_command(exe: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    c
}
pub fn spawn_self(arg: &str) -> Result<Child> {
    Ok(hidden_command(std::env::current_exe()?).arg(arg).spawn()?)
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
    std::fs::hard_link(from, to)?;
    std::fs::remove_file(from)
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
    Ok(())
}
pub fn sparse(f: &File) {
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
    {
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
        if free < needed.saturating_add(64 << 20) {
            return Err(Error::new(
                "no_space",
                "not enough free space (64 MiB margin)",
            ));
        }
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
pub fn crypt(bytes: &[u8], _: bool) -> Result<Vec<u8>> {
    if bytes.is_empty() {
        Ok(Vec::new())
    } else {
        Err(Error::new("unsupported", "DPAPI requires Windows"))
    }
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
    Ok(())
}
pub fn awake(on: bool) {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Power::SetThreadExecutionState(
            0x80000000 | if on { 1 } else { 0 },
        );
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
        None
    }
}
pub fn reveal(path: &Path, dir: &Path) -> Result<()> {
    if path.exists() {
        hidden_command("explorer.exe")
            .arg("/select,")
            .arg(path)
            .spawn()?;
    } else {
        hidden_command("explorer.exe").arg(dir).spawn()?;
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
    let _ = callback;
}
