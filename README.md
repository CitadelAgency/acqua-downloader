<p align="center"><img src="docs/logo.png" alt="Acqua Downloader" width="640"></p>

Download manager for Windows and macOS, written in Rust, with a Tauri/Svelte window, tray, CLI and Chromium extension. The engine downloads in segments, resumes through a journal, and verifies every file before delivering it.

## Install

### Windows

1. Download `Acqua-Downloader-setup.exe` from [Releases](https://github.com/CitadelAgency/dlm/releases/latest) and run it. It needs no administrator rights and installs WebView2 if it is missing.
2. The installer has no digital signature, so Windows may show "Windows protected your PC". Click **More info**, then **Run anyway**.
3. To capture downloads from your browser, open `chrome://extensions` (or `edge://extensions`, `brave://extensions`), turn on **Developer mode**, click **Load unpacked** and pick the extension folder. It is at `%LOCALAPPDATA%\Acqua Downloader\extension`, and the Start menu has a shortcut named **Acqua Downloader - Browser Extension**.

Requires 64-bit Windows 10 or 11.

### macOS

1. Download `Acqua-Downloader.dmg` from [Releases](https://github.com/CitadelAgency/dlm/releases/latest), open it and drag the app to **Applications**. It does not run from the disk image.
2. The app is not notarized, so macOS blocks the first launch. Open it once, then go to **System Settings > Privacy & Security** and click **Open Anyway**.
3. To capture downloads from your browser, open **Settings > Show extension folder** in the app. Open `chrome://extensions` (or `edge://extensions`, `brave://extensions`), turn on **Developer mode** and drag the `extension` folder onto the page.

Requires macOS 13 or later, on Apple silicon or Intel. To uninstall, run `"/Applications/Acqua Downloader.app/Contents/MacOS/acqua" unregister-host`, then delete the app, `~/Library/LaunchAgents/com.dlm.app.plist` and `~/Library/Application Support/Acqua Downloader`.

The interface is in English, and Portuguese is available in Settings.

## Build

Requirements: Rust MSVC 1.85+ (validated with 1.93), Visual Studio Build Tools with C++, Windows SDK, Node 20+ and WebView2.

```powershell
.\build.ps1 -Check
# build\bin\acqua.exe
```

Manual build:

```powershell
npm --prefix frontend ci
npm --prefix frontend run build
cargo build --locked --release --features custom-protocol
# target\release\acqua.exe
```

The `custom-protocol` feature embeds `frontend/dist`, so no Vite server is needed. Rebuild the Rust side after changing the frontend. CLI only: `cargo build --release --no-default-features --bin acqua`.

Installer (produces `target\release\bundle\nsis\Acqua Downloader_2.1.0_x64-setup.exe`):

```powershell
npm --prefix frontend ci
npx @tauri-apps/cli@2 build --bundles nsis
```

On macOS (Rust 1.85+, Xcode Command Line Tools, Node 20+):

```sh
npm --prefix frontend ci
npx @tauri-apps/cli@2 build --bundles dmg
```

## Run

```powershell
acqua
acqua --tray
acqua get https://server/file.zip -o D:\Downloads -c 16
acqua resume D:\Downloads\file.zip.dlm
acqua relink D:\Downloads\file.zip.dlm https://server/new-link
acqua chrome list
acqua help
```

`get` accepts `-name`, `-hash sha256:HEX|md5:HEX`, repeated `-mirror URL`, `-limit 5M`, `-H "Name: value"`, `-q` and `-v`. Exit codes: 0 success, 1 error or pause, 2 invalid usage, 3 expired link, 4 integrity. Ctrl+C keeps the confirmed progress.

The tray keeps the queue in `dlm.db`, under `%LOCALAPPDATA%\dlm` on Windows and `~/Library/Application Support/Acqua Downloader` on macOS. The window is a separate process, so closing it keeps downloads running. There are priorities, bandwidth limits, a schedule, languages, notifications, start with Windows, and optional clipboard watching.

The title bar is part of the header, with its own minimize, maximize/restore and close controls. Drag the logo or the empty header space to move the window, and double-click there to toggle maximize. Closing the window keeps the queue in the tray.

## Extension

Load the `extension` folder unpacked in Chrome, Edge or Brave. Its ID is fixed: `djlnbnckkmhijbocpoehdlcfnkfnikbi`. The tray registers the host automatically. `register-host` and `unregister-host` are also available.

Native Messaging and a named pipe with a per-user ACL handle communication, with no HTTP port. The extension sends the URL, cookies, referrer and User-Agent. Only the authorized origin can send `ping` and `add`.

## Integrity

- Ranges require a 206, Content-Range, size and a compatible validator. A 200 on resume is refused.
- The partial file gets `sync_data` before ranges and hashes are synced to the CRC32C journal.
- 4 MiB blocks use xxh3. Resuming checks blocks, 64 KiB seams and samples before writing.
- Delivery requires a full re-read and SHA-256. Hashes that were provided, published in headers or in `.sha256` files must match.
- The file gets the Mark-of-the-Web and a rename that never replaces an existing destination. Acqua Downloader never runs downloaded files.

## Tests

```powershell
cargo fmt --check
cargo clippy --all-targets --features custom-protocol,dev-tools -- -D warnings
cargo test --no-default-features --tests
node --test extension/rules.test.js
cargo run --release --no-default-features --features dev-tools --bin dlm-testserver -- 127.0.0.1:18765 67108864
cargo run --release --no-default-features --features dev-tools --bin dlm-bench -- 268435456
```

The server provides deterministic bytes and failure routes (`/slow.bin`, `/bad-range.bin`, `/range-200.bin`, `/drop.bin`, `/expired.bin`, `/unknown.bin`). The benchmark compares 1, 4 and 16 connections with a final hash.

For isolated tests, set `DLM_DATA_DIR`, `DLM_PIPE_SUFFIX` and `DLM_PORTABLE=1`. This separates the database and pipe and skips automatic host registration and startup. `--daemon` starts the queue without a tray and accepts the `Shutdown` RPC.

## Code

| Path | Responsibility |
|---|---|
| `src/engine.rs`, `fetch.rs`, `journal.rs`, `ranges.rs` | HTTP, integrity and resume |
| `src/manager.rs` | queue, SQLite, limits and schedule |
| `src/ipc.rs`, `native.rs`, `system.rs`, `chrome.rs` | Windows, macOS, extension and import |
| `src/desktop.rs`, `cli.rs`, `main.rs` | window, tray and commands |
| `frontend/`, `extension/` | Svelte and MV3 extension |
| `tests/`, `src/testserver.rs`, `src/bin/` | tests, server and benchmark |

## License

[MIT](LICENSE)
