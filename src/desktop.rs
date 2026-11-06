use dlm::{
    error::{Error, Result},
    ipc,
    manager::Manager as DownloadManager,
    native, system,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::Emitter;
#[tauri::command]
async fn rpc(method: String, args: Vec<Value>) -> std::result::Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if method == "ChooseDir" {
            return Ok(json!(
                rfd::FileDialog::new()
                    .pick_folder()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ));
        }
        let response = ipc::call(&json!({"type":"rpc","method":method,"args":args}))
            .map_err(|e| e.to_string())?;
        if response["ok"] == true {
            Ok(response["result"].clone())
        } else {
            Err(response["error"]
                .as_str()
                .unwrap_or("erro desconhecido")
                .into())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
pub fn window() -> Result<()> {
    let _lock = WindowLock::acquire()?;
    if _lock.is_none() {
        return Ok(());
    }
    native::ensure_daemon()?;
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![rpc])
        .setup(|app| {
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("dlm")
            .decorations(false)
            .shadow(true)
            .inner_size(980., 640.)
            .min_inner_size(560., 380.)
            .data_directory(system::data_dir().join("webview-rust"))
            .build()?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut failures = 0;
                loop {
                    match ipc::call(&json!({"type":"rpc","method":"Watch","args":[0]})) {
                        Ok(r) => {
                            failures = 0;
                            let _ = handle.emit("tasks", &r["result"]["tasks"]);
                        }
                        Err(_) => {
                            failures += 1;
                            if failures > 15 {
                                handle.exit(0);
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(300));
                        }
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .map_err(|e| Error::new("desktop", e.to_string()))?;
    Ok(())
}
pub fn tray(m: Arc<DownloadManager>) -> Result<()> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    };
    let background = m.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            let open = MenuItem::with_id(app, "open", "Abrir o dlm", true, None::<&str>)?;
            let pause = MenuItem::with_id(app, "pause", "Pausar tudo", true, None::<&str>)?;
            let resume = MenuItem::with_id(app, "resume", "Retomar tudo", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &pause, &resume, &quit])?;
            let manager = background.clone();
            TrayIconBuilder::new()
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../extension/icons/icon32.png"
                ))?)
                .tooltip("dlm")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|_, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        let _ = system::spawn_self("--ui");
                    }
                })
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "open" => {
                        let _ = system::spawn_self("--ui");
                    }
                    "pause" => {
                        let manager = manager.clone();
                        std::thread::spawn(move || {
                            let _ = manager.all_action(false);
                        });
                    }
                    "resume" => {
                        let _ = manager.all_action(true);
                    }
                    "quit" => {
                        let manager = manager.clone();
                        let handle = app.clone();
                        std::thread::spawn(move || {
                            manager.shutdown();
                            handle.exit(0);
                        });
                    }
                    _ => {}
                })
                .build(app)?;
            let handle = app.handle().clone();
            let manager = background.clone();
            std::thread::spawn(move || {
                use tauri_plugin_notification::NotificationExt;
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    if manager.stopped() {
                        manager.shutdown();
                        handle.exit(0);
                        break;
                    }
                    for (title, body) in manager.notifications() {
                        let _ = handle
                            .notification()
                            .builder()
                            .title(title)
                            .body(body)
                            .show();
                    }
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .map_err(|e| Error::new("desktop", e.to_string()))?
        .run(|_, _| {});
    m.shutdown();
    Ok(())
}
struct WindowLock(#[cfg(windows)] windows_sys::Win32::Foundation::HANDLE);
impl WindowLock {
    fn acquire() -> Result<Option<Self>> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::{
                Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError},
                System::Threading::CreateMutexW,
                UI::WindowsAndMessaging::{
                    FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow,
                },
            };
            let name = system::wide(format!(
                "Local\\dlm-rust-ui-{}{}",
                ipc::sid()?,
                ipc::pipe_suffix()
            ));
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe {
                    CloseHandle(handle);
                    let window = FindWindowW(std::ptr::null(), system::wide("dlm").as_ptr());
                    if !window.is_null() {
                        ShowWindow(window, SW_RESTORE);
                        SetForegroundWindow(window);
                    }
                }
                return Ok(None);
            }
            Ok(Some(Self(handle)))
        }
        #[cfg(not(windows))]
        {
            Ok(Some(Self()))
        }
    }
}
impl Drop for WindowLock {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
