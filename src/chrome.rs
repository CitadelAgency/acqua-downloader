use crate::{
    engine::{self, Config},
    error::{Error, Result},
    fetch::{self, Source, Validator},
    journal::{Journal, Meta},
    ranges::{Range, RangeSet},
    system,
};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Item {
    pub browser: String,
    pub profile: String,
    pub id: i64,
    pub guid: String,
    pub current_path: String,
    pub target_path: String,
    pub received: i64,
    pub total: i64,
    pub state: i64,
    pub interrupt_reason: i64,
    pub etag: String,
    pub last_modified: String,
    pub url: String,
    pub first_url: String,
    pub referrer: String,
    pub tab_url: String,
    pub mime: String,
    pub start: String,
    pub sliced: bool,
    pub prefix: u64,
    pub file_size: i64,
    pub in_use: bool,
    pub resume_at: u64,
    pub problem: String,
}
impl Item {
    pub fn source(&self) -> Source {
        let mut src = Source::new(&self.url);
        if let Ok(v) = self.referrer.parse() {
            src.headers.insert("referer", v);
        }
        src
    }
}
pub fn list_history(profile_dir: &Path, browser: &str, profile: &str) -> Result<Vec<Item>> {
    let temp = tempfile::tempdir()?;
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let from = profile_dir.join(format!("History{suffix}"));
        if from.exists() {
            fs::copy(from, temp.path().join(format!("History{suffix}")))?;
        }
    }
    let db = Connection::open(temp.path().join("History"))?;
    let mut query=db.prepare("SELECT d.id,d.guid,d.current_path,d.target_path,d.received_bytes,d.total_bytes,d.state,d.interrupt_reason,d.etag,d.last_modified,d.referrer,d.tab_url,d.mime_type,d.start_time,COALESCE((SELECT url FROM downloads_url_chains c WHERE c.id=d.id ORDER BY chain_index DESC LIMIT 1),''),COALESCE((SELECT url FROM downloads_url_chains c WHERE c.id=d.id ORDER BY chain_index ASC LIMIT 1),'') FROM downloads d WHERE d.state IN (0,4) ORDER BY d.start_time DESC")?;
    let rows = query.query_map([], |r| {
        let micros: i64 = r.get(13)?;
        let start = chrono::DateTime::from_timestamp_micros(micros - 11644473600 * 1_000_000)
            .map(|d| d.to_rfc3339())
            .unwrap_or_default();
        Ok(Item {
            browser: browser.into(),
            profile: profile.into(),
            id: r.get(0)?,
            guid: r.get(1)?,
            current_path: r.get(2)?,
            target_path: r.get(3)?,
            received: r.get(4)?,
            total: r.get(5)?,
            state: r.get(6)?,
            interrupt_reason: r.get(7)?,
            etag: r.get(8)?,
            last_modified: r.get(9)?,
            referrer: r.get(10)?,
            tab_url: r.get(11)?,
            mime: r.get(12)?,
            url: r.get(14)?,
            first_url: r.get(15)?,
            start,
            ..Item::default()
        })
    })?;
    let mut items = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    for it in &mut items {
        it.prefix = it.received.max(0) as u64;
        if let Ok(mut stmt)=db.prepare("SELECT offset,received_bytes FROM downloads_slices WHERE download_id=? ORDER BY offset"){let slices=stmt.query_map([it.id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;if let Some((offset,len))=slices.first(){it.sliced=true;it.prefix=if *offset==0{(*len).max(0) as u64}else{0};}}
        match fs::metadata(&it.current_path) {
            Ok(meta) => {
                it.file_size = meta.len() as i64;
                it.resume_at = meta.len().min(it.prefix);
                it.in_use = system::in_use(Path::new(&it.current_path));
            }
            Err(_) => {
                it.file_size = -1;
                it.problem = "the partial file no longer exists".into();
            }
        }
        if fetch::validate_url(&it.url).is_err() {
            it.problem = "no valid HTTP(S) URL".into();
        } else if it.total <= 0 {
            it.problem = "unknown total size".into();
        } else if it.file_size > it.total {
            it.problem = "partial file is larger than the total".into();
        }
    }
    Ok(items)
}
pub fn all() -> Result<Vec<Item>> {
    let root = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default());
    let mut items = Vec::new();
    let mut errors = Vec::new();
    for (name, dir) in [
        ("Chrome", r"Google\Chrome\User Data"),
        ("Chrome Beta", r"Google\Chrome Beta\User Data"),
        ("Chrome Canary", r"Google\Chrome SxS\User Data"),
        ("Edge", r"Microsoft\Edge\User Data"),
        ("Brave", r"BraveSoftware\Brave-Browser\User Data"),
    ] {
        let path = root.join(dir);
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        let state: serde_json::Value = fs::read(path.join("Local State"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        for entry in entries.flatten() {
            if !entry.path().join("History").exists() {
                continue;
            }
            let dir = entry.file_name().to_string_lossy().into_owned();
            let profile = state["profile"]["info_cache"][&dir]["name"]
                .as_str()
                .unwrap_or(&dir);
            match list_history(&entry.path(), name, profile) {
                Ok(mut v) => items.append(&mut v),
                Err(e) => errors.push(e),
            }
        }
    }
    if items.is_empty() && !errors.is_empty() {
        return Err(errors.remove(0));
    }
    Ok(items)
}
pub fn prepare(it: &Item) -> Result<Config> {
    if !it.problem.is_empty() {
        return Err(Error::new("chrome", &it.problem));
    }
    let target = if it.target_path.is_empty() {
        it.current_path.trim_end_matches(".crdownload")
    } else {
        &it.target_path
    };
    let target = PathBuf::from(target);
    let part = PathBuf::from(format!("{}.dlmpart", target.display()));
    let jp = PathBuf::from(format!("{}.dlm", target.display()));
    let current = Path::new(&it.current_path);
    let proof_path = if current.exists() { current } else { &part };
    let f = File::open(proof_path)?;
    let mut done = RangeSet::default();
    done.add(Range::new(0, f.metadata()?.len().min(it.prefix)));
    engine::prove_identity(
        &fetch::client()?,
        &it.source(),
        it.total as u64,
        &Validator {
            etag: it.etag.clone(),
            lm: it.last_modified.clone(),
        },
        &done,
        &f,
    )?;
    drop(f);
    if !part.exists() {
        if system::in_use(current) {
            fs::hard_link(current, &part).map_err(|e| {
                Error::new(
                    "browser_in_use",
                    format!("close the browser without canceling and try again: {e}"),
                )
            })?;
            return Err(Error::new(
                "browser_in_use",
                "partial file is protected by a hard link; cancel it in the browser and try again",
            ));
        }
        system::rename_no_replace(current, &part)?;
    } else {
        if system::in_use(&part) {
            return Err(Error::new(
                "browser_in_use",
                "the browser is still using the file",
            ));
        }
        if current.exists() {
            // Identity of filesystem objects must be checked before dropping the browser name.
            let mine = File::open(&part)?;
            let other = File::open(current)?;
            if !same_file(&mine, &other)? {
                return Err(Error::new(
                    "not_same_file",
                    "another partial file already exists at the destination",
                ));
            }
            drop(mine);
            drop(other);
            fs::remove_file(current)?;
        }
    }
    if !jp.exists() {
        let meta = Meta {
            url: it.url.clone(),
            name: target
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            total: it.total,
            ranges: true,
            etag: it.etag.clone(),
            lm: it.last_modified.clone(),
            imported: it.browser.to_lowercase(),
            referrer: it.referrer.clone(),
            ..Meta::default()
        };
        drop(Journal::create(&jp, meta, done)?);
    }
    Ok(Config {
        sources: vec![it.source()],
        dir: target.parent().unwrap_or(Path::new(".")).into(),
        name: target
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        journal_path: jp,
        part_path: part,
        ..Config::default()
    })
}
fn same_file(a: &File, b: &File) -> Result<bool> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::*;
        let mut ai = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
        let mut bi = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
        if unsafe { GetFileInformationByHandle(a.as_raw_handle(), &mut ai) } == 0
            || unsafe { GetFileInformationByHandle(b.as_raw_handle(), &mut bi) } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(ai.dwVolumeSerialNumber == bi.dwVolumeSerialNumber
            && ai.nFileIndexHigh == bi.nFileIndexHigh
            && ai.nFileIndexLow == bi.nFileIndexLow)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let (a, b) = (a.metadata()?, b.metadata()?);
        Ok(a.ino() == b.ino() && a.dev() == b.dev())
    }
}
pub fn snapshot_database(from: &Path, to: &Path) -> Result<()> {
    let source = Connection::open_with_flags(from, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut dest = Connection::open(to)?;
    let backup = rusqlite::backup::Backup::new(&source, &mut dest)?;
    backup.run_to_completion(64, std::time::Duration::from_millis(10), None)?;
    Ok(())
}
