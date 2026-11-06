#![cfg_attr(
    all(windows, not(debug_assertions), feature = "desktop"),
    windows_subsystem = "windows"
)]
mod cli;
#[cfg(feature = "desktop")]
mod desktop;
use dlm::{error::Result, manager::Manager, system};
fn daemon() -> Result<()> {
    let m = Manager::open(&system::data_dir().join("dlm.db"))?;
    m.serve()?;
    m.start();
    if std::env::var_os("DLM_PORTABLE").is_none() {
        dlm::native::register()?;
        system::autostart(m.settings().autostart)?;
    }
    #[cfg(feature = "desktop")]
    return desktop::tray(m);
    #[cfg(not(feature = "desktop"))]
    {
        while !m.stopped() {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        m.shutdown();
        Ok(())
    }
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(origin) = args
        .first()
        .filter(|s| s.starts_with("chrome-extension://"))
    {
        std::process::exit(dlm::native::run(origin));
    }
    if args
        .first()
        .is_some_and(|s| !s.starts_with("--") || matches!(s.as_str(), "--help" | "--version"))
    {
        system::attach_console();
        std::process::exit(cli::run(&args));
    }
    let result = if args.first().is_some_and(|s| s == "--tray") {
        if dlm::ipc::running() {
            return;
        }
        daemon()
    } else if args.first().is_some_and(|s| s == "--daemon") {
        (|| -> Result<()> {
            let m = Manager::open(&system::data_dir().join("dlm.db"))?;
            m.serve()?;
            m.start();
            while !m.stopped() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            m.shutdown();
            Ok(())
        })()
    } else {
        #[cfg(feature = "desktop")]
        {
            desktop::window()
        }
        #[cfg(not(feature = "desktop"))]
        {
            system::attach_console();
            std::process::exit(cli::run(&["help".into()]));
        }
    };
    if let Err(e) = result {
        eprintln!("{e}");
        #[cfg(feature = "desktop")]
        rfd::MessageDialog::new()
            .set_title("Acqua Downloader")
            .set_description(e.to_string())
            .show();
        std::process::exit(1);
    }
}
