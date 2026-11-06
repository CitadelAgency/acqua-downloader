use dlm::{
    chrome,
    engine::{self, Config, Download, RateLimiter},
    error::{Error, Result},
    fetch::Source,
    journal::Journal,
    native,
};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};
const HELP: &str = "Acqua Downloader 2\n\nacqua get <url> [-o folder] [-name file] [-c 16] [-hash sha256:...|md5:...]\n              [-mirror url]... [-limit 5M] [-H \"Name: value\"]... [-q] [-v]\nacqua resume <file.dlm>\nacqua relink <file.dlm> <new-url>\nacqua chrome [list] | chrome import <n>\nacqua register-host | unregister-host\n\nNo arguments: window; --tray: tray.\nExit codes: 0 success, 1 error/pause, 2 usage, 3 expired link, 4 integrity.\n";
pub fn run(args: &[String]) -> i32 {
    match command(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            if e.integrity() {
                4
            } else if e.code == "needs_link" {
                3
            } else if e.code == "usage" {
                2
            } else {
                1
            }
        }
    }
}
fn command(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str).unwrap_or("help") {
        "help" | "--help" | "-h" => print!("{HELP}"),
        "version" | "--version" => println!("Acqua Downloader {}", env!("CARGO_PKG_VERSION")),
        "get" => {
            let mut cfg = Config::default();
            let mut urls = Vec::new();
            let mut headers = reqwest::header::HeaderMap::new();
            let mut quiet = false;
            let mut i = 1;
            while i < args.len() {
                let a = &args[i];
                if !a.starts_with('-') {
                    if !urls.is_empty() {
                        return Err(Error::new("usage", "unexpected extra URL; use -mirror"));
                    }
                    urls.push(a.clone());
                    i += 1;
                    continue;
                }
                if matches!(a.as_str(), "-q" | "--quiet") {
                    quiet = true;
                    i += 1;
                    continue;
                }
                if matches!(a.as_str(), "-v" | "--verbose") {
                    i += 1;
                    continue;
                }
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| Error::new("usage", format!("missing value for {a}")))?;
                match a.as_str() {
                    "-o" | "--output" => cfg.dir = PathBuf::from(v),
                    "-name" | "--name" => cfg.name = v.clone(),
                    "-c" | "--connections" => {
                        cfg.max_conns = v
                            .parse()
                            .map_err(|_| Error::new("usage", "invalid connections"))?;
                        if !(1..=32).contains(&cfg.max_conns) {
                            return Err(Error::new("usage", "use 1 to 32 connections"));
                        }
                    }
                    "-hash" | "--hash" => cfg.expected_hash = v.clone(),
                    "-mirror" | "--mirror" => urls.push(v.clone()),
                    "-limit" | "--limit" => cfg
                        .limiters
                        .push(Arc::new(RateLimiter::new(parse_rate(v)?))),
                    "-H" | "--header" => {
                        let (k, v) = v
                            .split_once(':')
                            .ok_or_else(|| Error::new("usage", "header must be Name: value"))?;
                        let k = reqwest::header::HeaderName::from_bytes(k.trim().as_bytes())
                            .map_err(|_| Error::new("usage", "invalid header name"))?;
                        headers.insert(
                            k,
                            v.trim()
                                .parse()
                                .map_err(|_| Error::new("usage", "invalid header value"))?,
                        );
                    }
                    _ => return Err(Error::new("usage", format!("unknown option: {a}"))),
                }
                i += 2;
            }
            if urls.is_empty() {
                return Err(Error::new("usage", HELP));
            }
            cfg.sources = urls
                .into_iter()
                .map(|url| Source {
                    url,
                    headers: headers.clone(),
                })
                .collect();
            download(cfg, quiet)?;
        }
        "resume" => {
            if args.len() != 2 {
                return Err(Error::new("usage", HELP));
            }
            let path = std::fs::canonicalize(&args[1])?;
            let j = Journal::open(&path)?;
            let mut urls = vec![j.meta.url.clone()];
            urls.extend(j.meta.mirrors.clone());
            let cfg = Config {
                sources: urls.into_iter().map(Source::new).collect(),
                dir: path.parent().unwrap().into(),
                journal_path: path,
                ..Config::default()
            };
            drop(j);
            download(cfg, false)?;
        }
        "relink" => {
            if args.len() != 3 {
                return Err(Error::new("usage", HELP));
            }
            let path = std::fs::canonicalize(&args[1])?;
            let part = PathBuf::from(format!(
                "{}.dlmpart",
                path.to_string_lossy().trim_end_matches(".dlm")
            ));
            engine::relink(&path, &part, &Source::new(&args[2]))?;
            println!("link accepted; run: acqua resume {}", args[1]);
        }
        "register-host" => {
            native::register()?;
            println!("host registered: {}", native::manifest_path().display());
        }
        "unregister-host" => {
            native::unregister()?;
            println!("host removed");
        }
        "chrome" => {
            let items = chrome::all()?;
            if args.len() == 1 || args.get(1).is_some_and(|s| s == "list") {
                for (i, it) in items.iter().enumerate() {
                    println!(
                        "{i:3}  {} · {}  {}\n     {} / {} bytes {}",
                        it.browser, it.profile, it.target_path, it.resume_at, it.total, it.problem
                    );
                }
                if items.is_empty() {
                    println!("No interrupted downloads found.");
                }
            } else if args.len() == 3 && args[1] == "import" {
                let n = args[2]
                    .parse::<usize>()
                    .map_err(|_| Error::new("usage", "invalid index"))?;
                let it = items
                    .get(n)
                    .ok_or_else(|| Error::new("usage", "invalid index"))?;
                download(chrome::prepare(it)?, false)?;
            } else {
                return Err(Error::new("usage", HELP));
            }
        }
        _ => return Err(Error::new("usage", HELP)),
    }
    Ok(())
}
fn download(cfg: Config, quiet: bool) -> Result<()> {
    let d = Download::new(cfg);
    let cancel = d.cancel.clone();
    let _ = ctrlc::set_handler(move || cancel.store(true, Ordering::Relaxed));
    let live = d.live.clone();
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = finished.clone();
    let monitor = if quiet {
        None
    } else {
        Some(std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(400));
                let s = live.lock().unwrap();
                eprint!(
                    "\r{} {}/{} · {:.1} MiB/s · {} connections       ",
                    s.state,
                    s.done,
                    s.total,
                    s.speed / 1048576.,
                    s.conns
                );
            }
        }))
    };
    let result = d.run();
    finished.store(true, Ordering::Relaxed);
    if let Some(handle) = monitor {
        let _ = handle.join();
        eprintln!();
    }
    let result = result?;
    if result.check.checked {
        println!(
            "Hash {} verified ({})",
            result.check.algo, result.check.source
        );
    }
    println!("{}\nSHA-256 {}", result.path, result.sha256);
    Ok(())
}
pub fn parse_rate(text: &str) -> Result<u64> {
    let text = text
        .trim()
        .to_uppercase()
        .replace("/S", "")
        .replace("IB", "")
        .replace('B', "");
    let (text, mul) = if let Some(s) = text.strip_suffix('K') {
        (s, 1024.)
    } else if let Some(s) = text.strip_suffix('M') {
        (s, 1048576.)
    } else if let Some(s) = text.strip_suffix('G') {
        (s, 1073741824.)
    } else {
        (text.as_str(), 1.)
    };
    let n = text
        .parse::<f64>()
        .map_err(|_| Error::new("usage", "invalid limit"))?;
    if !n.is_finite() || n < 0. || n * mul > u64::MAX as f64 {
        return Err(Error::new("usage", "invalid limit"));
    }
    Ok((n * mul) as u64)
}
