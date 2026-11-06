use crate::{
    error::{Error, Result},
    ipc, system,
};
use serde_json::{Value, json};
pub const EXTENSION_ID: &str = "djlnbnckkmhijbocpoehdlcfnkfnikbi";
pub const ORIGIN: &str = "chrome-extension://djlnbnckkmhijbocpoehdlcfnkfnikbi/";
const ROOTS: [&str; 4] = [
    r"Software\Google\Chrome\NativeMessagingHosts\com.dlm.host",
    r"Software\Microsoft\Edge\NativeMessagingHosts\com.dlm.host",
    r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.dlm.host",
    r"Software\Chromium\NativeMessagingHosts\com.dlm.host",
];
pub fn manifest_path() -> std::path::PathBuf {
    system::data_dir().join("com.dlm.host.json")
}
pub fn register() -> Result<()> {
    std::fs::create_dir_all(system::data_dir())?;
    let path = manifest_path();
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(
            &json!({"name":"com.dlm.host","description":"dlm download manager","path":std::env::current_exe()?,"type":"stdio","allowed_origins":[ORIGIN]}),
        )?,
    )?;
    #[cfg(windows)]
    for root in ROOTS {
        let (k, _) =
            winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).create_subkey(root)?;
        k.set_value("", &path.display().to_string())?;
    }
    Ok(())
}
pub fn unregister() -> Result<()> {
    #[cfg(windows)]
    for root in ROOTS {
        match winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).delete_subkey(root) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    if manifest_path().exists() {
        std::fs::remove_file(manifest_path())?;
    }
    Ok(())
}
pub fn registered() -> bool {
    #[cfg(windows)]
    {
        winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .open_subkey(ROOTS[0])
            .and_then(|k| k.get_value::<String, _>(""))
            .is_ok_and(|p| p.eq_ignore_ascii_case(&manifest_path().display().to_string()))
    }
    #[cfg(not(windows))]
    {
        false
    }
}
pub fn ensure_daemon() -> Result<()> {
    if ipc::running() {
        return Ok(());
    }
    system::spawn_self("--tray")?;
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if ipc::running() {
            return Ok(());
        }
    }
    Err(Error::new(
        "not_running",
        "o processo de fundo não respondeu",
    ))
}
pub fn run(origin: &str) -> i32 {
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    if origin != ORIGIN {
        let _ = ipc::write_msg(
            &mut output,
            br#"{"ok":false,"error":"extensao nao autorizada"}"#,
        );
        return 1;
    }
    while let Ok(bytes) = ipc::read_msg(&mut input) {
        let response = (|| -> Result<Value> {
            let r: Value = serde_json::from_slice(&bytes)?;
            if !matches!(r["type"].as_str(), Some("ping" | "add")) {
                return Err(Error::new("native", "mensagem não permitida"));
            }
            ensure_daemon()?;
            ipc::call(&r)
        })();
        let value = response.unwrap_or_else(|e| json!({"ok":false,"error":e.to_string()}));
        if ipc::write_msg(&mut output, &serde_json::to_vec(&value).unwrap_or_default()).is_err() {
            break;
        }
    }
    0
}
