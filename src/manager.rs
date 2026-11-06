use crate::{
    chrome::{self, Item},
    engine::{self, Config, Download, HashCheck, HostLimiter, RateLimiter, Snapshot},
    error::{Error, Result},
    fetch::{self, Source},
    ipc,
    journal::Journal,
    native,
    ranges::Range,
    system,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    pub enabled: bool,
    pub start: String,
    pub end: String,
}
impl Default for Schedule {
    fn default() -> Self {
        Self {
            enabled: false,
            start: "00:00".into(),
            end: "23:59".into(),
        }
    }
}
impl Schedule {
    pub fn allows(&self, time: chrono::NaiveTime) -> bool {
        if !self.enabled {
            return true;
        }
        let (Ok(start), Ok(end)) = (
            chrono::NaiveTime::parse_from_str(&self.start, "%H:%M"),
            chrono::NaiveTime::parse_from_str(&self.end, "%H:%M"),
        ) else {
            return false;
        };
        if start == end {
            return true;
        }
        if start < end {
            time >= start && time < end
        } else {
            time >= start || time < end
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub lang: String,
    pub dir: String,
    pub max_active: usize,
    pub max_conns: usize,
    pub global_limit_bps: u64,
    pub schedule: Schedule,
    pub watch_clipboard: bool,
    pub autostart: bool,
    pub notify: bool,
    pub capture_min_size: u64,
    pub capture_types: Vec<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: "en".into(),
            dir: PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_else(|| ".".into()))
                .join("Downloads")
                .display()
                .to_string(),
            max_active: 3,
            max_conns: 16,
            global_limit_bps: 0,
            schedule: Schedule::default(),
            watch_clipboard: false,
            autostart: true,
            notify: true,
            capture_min_size: 5 << 20,
            capture_types: [
                "zip", "rar", "7z", "iso", "exe", "msi", "mp4", "mkv", "pdf", "gz", "xz", "bin",
                "img", "dmg", "apk",
            ]
            .map(str::to_string)
            .into(),
        }
    }
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Task {
    pub id: i64,
    pub url: String,
    pub mirrors: Vec<String>,
    pub dir: String,
    pub name: String,
    pub user_name: bool,
    pub final_path: String,
    pub part_path: String,
    pub journal_path: String,
    pub state: String,
    pub err_code: String,
    pub message: String,
    pub priority: i64,
    pub limit_bps: u64,
    pub total: i64,
    pub done: u64,
    pub sha256: String,
    pub check: HashCheck,
    pub expected_hash: String,
    pub referrer: String,
    pub user_agent: String,
    pub created: i64,
    pub finished: i64,
    pub start_at: i64,
    pub no_resume: bool,
    pub chrome: Option<Item>,
    pub retry_at: i64,
    pub ranges: Vec<Range>,
    #[serde(skip)]
    pub cookies: String,
}
impl Task {
    fn source(&self) -> Source {
        let mut src = Source::new(&self.url);
        for (k, v) in [
            ("cookie", &self.cookies),
            ("referer", &self.referrer),
            ("user-agent", &self.user_agent),
        ] {
            if !v.is_empty() {
                if let Ok(v) = v.parse() {
                    src.headers.insert(k, v);
                }
            }
        }
        src
    }
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AddRequest {
    pub url: String,
    pub dir: String,
    pub name: String,
    pub hash: String,
    pub mirrors: Vec<String>,
    pub cookies: String,
    pub referrer: String,
    pub user_agent: String,
    pub priority: i64,
    pub start_at: i64,
    pub limit_bps: u64,
}
struct Running {
    cancel: Arc<AtomicBool>,
    live: Arc<Mutex<Snapshot>>,
    reason: String,
}
struct Entry {
    task: Task,
    running: Option<Running>,
    limiter: Arc<RateLimiter>,
}
struct Inner {
    db: Connection,
    tasks: BTreeMap<i64, Entry>,
    settings: Settings,
    version: u64,
    notifications: Vec<(String, String)>,
}
pub struct Manager {
    inner: Mutex<Inner>,
    stop: AtomicBool,
    global: Arc<RateLimiter>,
    hosts: Arc<HostLimiter>,
}
impl Manager {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        // Keep a SQLite-consistent copy before a new implementation touches an existing database.
        if path.exists() {
            let backup = path.with_extension("pre-rust.db");
            if !backup.exists() {
                chrome::snapshot_database(path, &backup)?;
            }
        }
        let db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS tasks(id INTEGER PRIMARY KEY AUTOINCREMENT,data TEXT NOT NULL,cookies BLOB); CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);")?;
        let settings: Settings = match db.query_row::<String, _, _>(
            "SELECT value FROM settings WHERE key='settings'",
            [],
            |r| r.get(0),
        ) {
            Ok(raw) => serde_json::from_str(&raw)?,
            Err(rusqlite::Error::QueryReturnedNoRows) => Settings::default(),
            Err(e) => return Err(e.into()),
        };
        let mut tasks = BTreeMap::new();
        {
            let mut stmt = db.prepare("SELECT id,data,cookies FROM tasks")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<Vec<u8>>>(2)?,
                ))
            })?;
            for row in rows {
                let (id, raw, cookies) = row?;
                let mut task: Task = serde_json::from_str(&raw)?;
                task.id = id;
                if let Some(bytes) = cookies {
                    task.cookies = String::from_utf8(system::crypt(&bytes, true)?)
                        .map_err(|_| Error::new("database", "invalid encrypted cookies"))?;
                }
                if matches!(task.state.as_str(), "downloading" | "waiting_network") {
                    task.state = "queued".into();
                }
                let limiter = Arc::new(RateLimiter::new(task.limit_bps));
                tasks.insert(
                    id,
                    Entry {
                        task,
                        running: None,
                        limiter,
                    },
                );
            }
        }
        let global = Arc::new(RateLimiter::new(settings.global_limit_bps));
        let hosts = Arc::new(HostLimiter::new(settings.max_conns as u64));
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner {
                db,
                tasks,
                settings,
                version: 1,
                notifications: Vec::new(),
            }),
            stop: AtomicBool::new(false),
            global,
            hosts,
        }))
    }
    pub fn start(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        system::watch_network(move || {
            let Some(m) = weak.upgrade() else {
                return false;
            };
            if m.stopped() {
                return false;
            }
            let mut inner = m.inner.lock().unwrap();
            for e in inner.tasks.values_mut() {
                if e.task.state == "waiting_network" {
                    e.task.retry_at = 0;
                }
            }
            true
        });
        let m = self.clone();
        std::thread::spawn(move || {
            let mut last_clip = String::new();
            while !m.stop.load(Ordering::Relaxed) {
                if let Err(e) = m.tick() {
                    let _ =
                        std::fs::write(system::data_dir().join("rust-error.log"), e.to_string());
                }
                let watch = m.settings().watch_clipboard;
                if watch {
                    if let Some(text) = system::clipboard() {
                        if text != last_clip {
                            last_clip = text.clone();
                            let _ = m.add_text(&text);
                        }
                    }
                }
                for _ in 0..5 {
                    if m.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            system::awake(false);
        });
    }
    pub fn settings(&self) -> Settings {
        self.inner.lock().unwrap().settings.clone()
    }
    fn persist(db: &Connection, t: &Task) -> Result<()> {
        let data = serde_json::to_string(t)?;
        let cookies = system::crypt(t.cookies.as_bytes(), false)?;
        db.execute("INSERT INTO tasks(id,data,cookies) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET data=excluded.data,cookies=excluded.cookies",params![t.id,data,cookies])?;
        Ok(())
    }
    pub fn list(&self) -> Value {
        let inner = self.inner.lock().unwrap();
        Value::Array(
            inner
                .tasks
                .values()
                .rev()
                .map(|e| {
                    let mut v = serde_json::to_value(&e.task).unwrap();
                    if let Some(r) = &e.running {
                        let s = r.live.lock().unwrap().clone();
                        v["done"] = json!(s.done);
                        v["total"] = json!(s.total);
                        if !s.name.is_empty() {
                            v["name"] = json!(s.name)
                        }
                        v["noResume"] = json!(!s.ranges && s.total >= 0);
                        v["live"] = serde_json::to_value(s).unwrap();
                    }
                    v
                })
                .collect(),
        )
    }
    fn tick(self: &Arc<Self>) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let allowed = inner.settings.schedule.allows(chrono::Local::now().time());
        let current = now();
        let mut active = inner.tasks.values().filter(|e| e.running.is_some()).count();
        let mut order: Vec<_> = inner
            .tasks
            .values()
            .map(|e| (e.task.id, e.task.priority))
            .collect();
        order.sort_by_key(|(id, p)| (std::cmp::Reverse(*p), *id));
        let settings = inner.settings.clone();
        for (id, _) in order {
            let e = inner.tasks.get_mut(&id).unwrap();
            if let Some(r) = e.running.as_mut() {
                if !allowed {
                    r.reason = "window".into();
                    r.cancel.store(true, Ordering::Relaxed);
                }
                let s = r.live.lock().unwrap();
                e.task.done = s.done;
                e.task.total = s.total;
                e.task.ranges = s.done_ranges.clone();
                if !s.journal_path.is_empty() {
                    e.task.name = s.name.clone();
                    e.task.final_path = s.final_path.clone();
                    e.task.part_path = s.part_path.clone();
                    e.task.journal_path = s.journal_path.clone();
                }
                continue;
            }
            if e.task.state == "waiting_network" && e.task.retry_at <= current {
                e.task.state = "queued".into();
            }
            if !matches!(e.task.state.as_str(), "queued" | "scheduled") {
                continue;
            }
            if !allowed || e.task.start_at > current {
                e.task.state = "scheduled".into();
                continue;
            }
            if active >= settings.max_active {
                continue;
            }
            e.task.state = "downloading".into();
            e.task.err_code.clear();
            e.task.message.clear();
            active += 1;
            let mut sources = vec![e.task.source()];
            for url in &e.task.mirrors {
                let mut src = e.task.source();
                src.url = url.clone();
                sources.push(src);
            }
            let cfg = Config {
                sources,
                dir: PathBuf::from(&e.task.dir),
                name: if e.task.user_name {
                    e.task.name.clone()
                } else {
                    String::new()
                },
                journal_path: PathBuf::from(&e.task.journal_path),
                part_path: PathBuf::from(&e.task.part_path),
                expected_hash: e.task.expected_hash.clone(),
                max_conns: settings.max_conns,
                limiters: vec![self.global.clone(), e.limiter.clone()],
                hosts: self.hosts.clone(),
                ..Config::default()
            };
            let dl = Download::new(cfg);
            e.running = Some(Running {
                cancel: dl.cancel.clone(),
                live: dl.live.clone(),
                reason: String::new(),
            });
            let m = self.clone();
            std::thread::spawn(move || {
                let result = dl.run();
                m.finished(id, &dl, result);
            });
        }
        system::awake(active > 0);
        inner.version += 1;
        for e in inner.tasks.values() {
            if e.running.is_some() {
                Self::persist(&inner.db, &e.task)?;
            }
        }
        Ok(())
    }
    fn finished(&self, id: i64, dl: &Download, result: Result<engine::DownloadResult>) {
        let mut inner = self.inner.lock().unwrap();
        let s = dl.live.lock().unwrap().clone();
        let notify = inner.settings.notify;
        let Some(e) = inner.tasks.get_mut(&id) else {
            return;
        };
        let reason = e.running.take().map(|r| r.reason).unwrap_or_default();
        e.task.done = s.done;
        e.task.total = s.total;
        e.task.ranges = s.done_ranges;
        e.task.no_resume = !s.ranges;
        if !s.journal_path.is_empty() {
            e.task.name = s.name;
            e.task.final_path = s.final_path;
            e.task.part_path = s.part_path;
            e.task.journal_path = s.journal_path;
        }
        let mut notification = None;
        match result {
            Ok(res) => {
                e.task.state = "done".into();
                e.task.final_path = res.path;
                e.task.done = res.size;
                e.task.total = res.size as i64;
                e.task.sha256 = res.sha256;
                e.task.check = res.check;
                e.task.finished = now();
                notification = Some(("Download complete".into(), e.task.name.clone()));
            }
            Err(err) => {
                if err.code == "paused" {
                    e.task.state = match reason.as_str() {
                        "window" => "scheduled",
                        "shutdown" => "queued",
                        _ => "paused",
                    }
                    .into();
                } else {
                    e.task.err_code = err.code.clone();
                    e.task.message = err.to_string();
                    e.task.state = match err.code.as_str() {
                        "network" => {
                            e.task.retry_at = now() + 30000;
                            "waiting_network"
                        }
                        "needs_link" => {
                            notification =
                                Some(("A new link is needed".into(), e.task.name.clone()));
                            "needs_link"
                        }
                        _ => "error",
                    }
                    .into();
                }
            }
        }
        let task = e.task.clone();
        if let Err(e) = Self::persist(&inner.db, &task) {
            let _ = std::fs::write(system::data_dir().join("rust-error.log"), e.to_string());
        }
        if notify {
            if let Some(n) = notification {
                inner.notifications.push(n)
            }
        }
        inner.version += 1;
    }
    pub fn notifications(&self) -> Vec<(String, String)> {
        std::mem::take(&mut self.inner.lock().unwrap().notifications)
    }
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        {
            let mut inner = self.inner.lock().unwrap();
            for e in inner.tasks.values_mut() {
                if let Some(r) = e.running.as_mut() {
                    r.reason = "shutdown".into();
                    r.cancel.store(true, Ordering::Relaxed);
                }
            }
        }
        loop {
            if self
                .inner
                .lock()
                .unwrap()
                .tasks
                .values()
                .all(|e| e.running.is_none())
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    pub fn add(&self, mut r: AddRequest) -> Result<Value> {
        r.url = r.url.trim().into();
        fetch::validate_url(&r.url)?;
        for mirror in &r.mirrors {
            fetch::validate_url(mirror)?;
        }
        if !r.hash.is_empty() {
            fetch::user_hash(&r.hash)?;
        }
        let candidates = {
            let inner = self.inner.lock().unwrap();
            if let Some(e) = inner.tasks.values().find(|e| {
                e.task.url == r.url
                    && !matches!(e.task.state.as_str(), "done" | "error" | "needs_link")
            }) {
                return Ok(json!({"id":e.task.id,"existing":true,"relinked":false}));
            }
            inner
                .tasks
                .values()
                .filter(|e| {
                    e.running.is_none()
                        && e.task.state == "needs_link"
                        && !e.task.journal_path.is_empty()
                })
                .map(|e| e.task.id)
                .collect::<Vec<_>>()
        };
        for id in candidates {
            let source = Task {
                url: r.url.clone(),
                cookies: r.cookies.clone(),
                referrer: r.referrer.clone(),
                user_agent: r.user_agent.clone(),
                ..Task::default()
            }
            .source();
            if self
                .relink_with(id, &source, Some((&r.cookies, &r.referrer, &r.user_agent)))
                .is_ok()
            {
                return Ok(json!({"id":id,"relinked":true,"existing":false}));
            }
        }
        let mut inner = self.inner.lock().unwrap();
        let dir = if r.dir.is_empty() {
            inner.settings.dir.clone()
        } else {
            r.dir
        };
        let task = Task {
            url: r.url,
            dir,
            name: r.name.clone(),
            user_name: !r.name.is_empty(),
            state: "queued".into(),
            priority: r.priority,
            limit_bps: r.limit_bps,
            expected_hash: r.hash,
            mirrors: r.mirrors,
            cookies: r.cookies,
            referrer: r.referrer,
            user_agent: r.user_agent,
            start_at: r.start_at,
            created: now(),
            total: -1,
            ..Task::default()
        };
        let bytes = system::crypt(task.cookies.as_bytes(), false)?;
        inner.db.execute(
            "INSERT INTO tasks(data,cookies) VALUES(?,?)",
            params![serde_json::to_string(&task)?, bytes],
        )?;
        let id = inner.db.last_insert_rowid();
        let mut task = task;
        task.id = id;
        let limiter = Arc::new(RateLimiter::new(task.limit_bps));
        inner.tasks.insert(
            id,
            Entry {
                task,
                running: None,
                limiter,
            },
        );
        inner.version += 1;
        Ok(json!({"id":id,"existing":false,"relinked":false}))
    }
    pub fn add_text(&self, text: &str) -> Result<Value> {
        let re = regex::Regex::new(r#"https?://[^\s\"'<>]+"#).unwrap();
        let mut out = Vec::new();
        for url in re.find_iter(text) {
            out.push(self.add(AddRequest {
                url: url.as_str().into(),
                ..AddRequest::default()
            })?);
        }
        Ok(json!(out))
    }
    pub fn pause(&self, id: i64, reason: &str) -> Result<()> {
        {
            let mut inner = self.inner.lock().unwrap();
            let e = inner
                .tasks
                .get_mut(&id)
                .ok_or_else(|| Error::new("task", "download not found"))?;
            if let Some(r) = &mut e.running {
                r.reason = reason.into();
                r.cancel.store(true, Ordering::Relaxed);
            } else if e.task.state != "done" {
                e.task.state = "paused".into();
            }
            let t = e.task.clone();
            Self::persist(&inner.db, &t)?;
            inner.version += 1;
        }
        loop {
            if self
                .inner
                .lock()
                .unwrap()
                .tasks
                .get(&id)
                .is_none_or(|e| e.running.is_none())
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }
    fn update(&self, id: i64, change: impl FnOnce(&mut Task) -> Result<()>) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let e = inner
            .tasks
            .get_mut(&id)
            .ok_or_else(|| Error::new("task", "download not found"))?;
        change(&mut e.task)?;
        e.limiter.set(e.task.limit_bps);
        let task = e.task.clone();
        Self::persist(&inner.db, &task)?;
        inner.version += 1;
        Ok(())
    }
    pub fn resume(&self, id: i64) -> Result<()> {
        self.update(id, |t| {
            if t.state == "done" {
                return Ok(());
            }
            if [
                "hash_mismatch",
                "seam_mismatch",
                "remote_changed",
                "bad_content_range",
                "not_same_file",
                "part_too_large",
            ]
            .contains(&t.err_code.as_str())
            {
                return Err(Error::new(
                    "integrity",
                    "start over or provide a link proven to be the same file",
                ));
            }
            if t.state != "downloading" {
                t.state = "queued".into();
                t.retry_at = 0;
                t.start_at = 0;
            }
            Ok(())
        })
    }
    pub fn all_action(&self, resume: bool) -> Result<()> {
        let ids: Vec<_> = self.inner.lock().unwrap().tasks.keys().copied().collect();
        for id in ids {
            if resume {
                let _ = self.resume(id);
            } else {
                self.pause(id, "pause")?;
            }
        }
        Ok(())
    }
    fn task(&self, id: i64) -> Result<Task> {
        self.inner
            .lock()
            .unwrap()
            .tasks
            .get(&id)
            .map(|e| e.task.clone())
            .ok_or_else(|| Error::new("task", "download not found"))
    }
    fn relink_with(
        &self,
        id: i64,
        src: &Source,
        credentials: Option<(&str, &str, &str)>,
    ) -> Result<()> {
        fetch::validate_url(&src.url)?;
        self.pause(id, "pause")?;
        let t = self.task(id)?;
        engine::relink(Path::new(&t.journal_path), Path::new(&t.part_path), src)?;
        self.update(id, |t| {
            t.url = src.url.clone();
            if let Some((c, r, u)) = credentials {
                t.cookies = c.into();
                t.referrer = r.into();
                t.user_agent = u.into();
            }
            t.state = "queued".into();
            t.err_code.clear();
            t.message.clear();
            Ok(())
        })
    }
    pub fn import(&self, items: Vec<Item>) -> Result<Value> {
        let mut errors = Vec::new();
        for item in items {
            let result = (|| -> Result<()> {
                let prepared = chrome::prepare(&item);
                let busy = matches!(&prepared,Err(e) if e.code=="browser_in_use");
                let message = prepared
                    .as_ref()
                    .err()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                if prepared.is_err() && !busy {
                    return Err(prepared.err().unwrap());
                }
                let target = PathBuf::from(if item.target_path.is_empty() {
                    item.current_path.trim_end_matches(".crdownload")
                } else {
                    &item.target_path
                });
                let cfg = prepared.unwrap_or_else(|_| Config {
                    dir: target.parent().unwrap_or(Path::new(".")).into(),
                    name: target
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into(),
                    part_path: PathBuf::from(format!("{}.dlmpart", target.display())),
                    journal_path: PathBuf::from(format!("{}.dlm", target.display())),
                    ..Config::default()
                });
                let mut inner = self.inner.lock().unwrap();
                let existing = inner
                    .tasks
                    .values()
                    .find(|e| {
                        e.task
                            .chrome
                            .as_ref()
                            .is_some_and(|c| c.current_path == item.current_path)
                    })
                    .map(|e| e.task.id);
                if existing.is_some_and(|id| inner.tasks[&id].running.is_some()) {
                    return Err(Error::new("chrome", "import already in progress"));
                }
                let mut task = existing
                    .and_then(|id| inner.tasks.get(&id).map(|e| e.task.clone()))
                    .unwrap_or_else(|| Task {
                        url: item.url.clone(),
                        created: now(),
                        user_name: true,
                        referrer: item.referrer.clone(),
                        ..Task::default()
                    });
                task.dir = cfg.dir.display().to_string();
                task.name = cfg.name;
                task.part_path = cfg.part_path.display().to_string();
                task.journal_path = cfg.journal_path.display().to_string();
                task.total = item.total;
                task.done = item.resume_at;
                task.chrome = Some(item);
                task.state = if busy { "browser_in_use" } else { "queued" }.into();
                task.err_code = if busy { "browser_in_use" } else { "" }.into();
                task.message = message.clone();
                if task.id == 0 {
                    inner.db.execute(
                        "INSERT INTO tasks(data,cookies) VALUES(?,?)",
                        params![serde_json::to_string(&task)?, Vec::<u8>::new()],
                    )?;
                    task.id = inner.db.last_insert_rowid();
                } else {
                    Self::persist(&inner.db, &task)?;
                }
                let limiter = Arc::new(RateLimiter::new(task.limit_bps));
                inner.tasks.insert(
                    task.id,
                    Entry {
                        task,
                        running: None,
                        limiter,
                    },
                );
                inner.version += 1;
                if busy {
                    return Err(Error::new("browser_in_use", message));
                }
                Ok(())
            })();
            if let Err(e) = result {
                errors.push(e.to_string());
            }
        }
        Ok(json!(errors))
    }
    pub fn rpc(&self, method: &str, args: &[Value]) -> Result<Value> {
        let id = args.first().and_then(Value::as_i64).unwrap_or(0);
        let arg = |i: usize| {
            args.get(i)
                .cloned()
                .ok_or_else(|| Error::new("rpc", format!("missing argument {i}")))
        };
        let text = |i: usize| -> Result<String> {
            arg(i)?
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| Error::new("rpc", "text expected"))
        };
        match method {
            "List" => return Ok(self.list()),
            "Watch" => {
                std::thread::sleep(Duration::from_millis(350));
                let version = self.inner.lock().unwrap().version;
                return Ok(json!({"ver":version,"tasks":self.list()}));
            }
            "GetSettings" => return Ok(serde_json::to_value(self.settings())?),
            "SaveSettings" => {
                let s: Settings = serde_json::from_value(arg(0)?)?;
                if !(1..=20).contains(&s.max_active)
                    || !(1..=32).contains(&s.max_conns)
                    || s.dir.is_empty()
                {
                    return Err(Error::new(
                        "settings",
                        "invalid folder, tasks or connections",
                    ));
                }
                if s.schedule.enabled {
                    for t in [&s.schedule.start, &s.schedule.end] {
                        chrono::NaiveTime::parse_from_str(t, "%H:%M")
                            .map_err(|_| Error::new("settings", "invalid time"))?;
                    }
                }
                system::autostart(s.autostart)?;
                self.global.set(s.global_limit_bps);
                self.hosts.max.store(s.max_conns as u64, Ordering::Relaxed);
                let mut inner = self.inner.lock().unwrap();
                inner.db.execute("INSERT INTO settings(key,value) VALUES('settings',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(&s)?])?;
                inner.settings = s;
                inner.version += 1;
            }
            "Add" => return self.add(serde_json::from_value(arg(0)?)?),
            "AddText" => return self.add_text(&text(0)?),
            "Pause" => self.pause(id, "pause")?,
            "Resume" => self.resume(id)?,
            "PauseAll" => self.all_action(false)?,
            "ResumeAll" => self.all_action(true)?,
            "SetPriority" => {
                let p = arg(1)?
                    .as_i64()
                    .ok_or_else(|| Error::new("rpc", "invalid priority"))?;
                self.update(id, |t| {
                    t.priority = p;
                    Ok(())
                })?;
            }
            "SetLimit" => {
                let bps = arg(1)?
                    .as_u64()
                    .ok_or_else(|| Error::new("rpc", "invalid limit"))?;
                self.update(id, |t| {
                    t.limit_bps = bps;
                    Ok(())
                })?;
            }
            "SetHash" => {
                let hash = text(1)?;
                if !hash.is_empty() {
                    fetch::user_hash(&hash)?;
                }
                self.pause(id, "pause")?;
                self.update(id, |t| {
                    t.expected_hash = hash;
                    Ok(())
                })?;
            }
            "Relink" => {
                let mut src = self.task(id)?.source();
                src.url = text(1)?;
                self.relink_with(id, &src, None)?;
            }
            "AddMirror" => {
                self.pause(id, "pause")?;
                let t = self.task(id)?;
                let url = text(1)?;
                let mut src = t.source();
                src.url = url.clone();
                let j = Journal::open(Path::new(&t.journal_path))?;
                let p = fetch::probe(&fetch::client()?, &src)?;
                if p.total != t.total
                    || !p.validator.strong()
                    || !engine::validator(&j.meta).strong()
                    || p.validator.etag != j.meta.etag
                {
                    return Err(Error::new(
                        "not_same_file",
                        "mirror must have the same strong ETag and size",
                    ));
                }
                drop(j);
                self.update(id, |t| {
                    t.mirrors.push(url);
                    t.state = "queued".into();
                    Ok(())
                })?;
            }
            "Restart" | "Remove" => {
                self.pause(id, "pause")?;
                let t = self.task(id)?;
                let delete =
                    method == "Restart" || args.get(1).and_then(Value::as_bool).unwrap_or(false);
                if delete && t.state != "done" {
                    for path in [&t.part_path, &t.journal_path] {
                        if !path.is_empty() && Path::new(path).exists() {
                            std::fs::remove_file(path)?;
                        }
                    }
                }
                if method == "Remove" {
                    let mut inner = self.inner.lock().unwrap();
                    inner.db.execute("DELETE FROM tasks WHERE id=?", [id])?;
                    inner.tasks.remove(&id);
                    inner.version += 1;
                } else {
                    self.update(id, |t| {
                        if t.state == "done" {
                            return Err(Error::new("task", "download already finished"));
                        }
                        t.state = "queued".into();
                        t.err_code.clear();
                        t.message.clear();
                        t.done = 0;
                        t.ranges.clear();
                        t.journal_path.clear();
                        t.part_path.clear();
                        Ok(())
                    })?;
                }
            }
            "ShowInFolder" => {
                let t = self.task(id)?;
                system::reveal(
                    Path::new(if t.state == "done" {
                        &t.final_path
                    } else {
                        &t.part_path
                    }),
                    Path::new(&t.dir),
                )?;
            }
            "ChromeList" => return Ok(serde_json::to_value(chrome::all()?)?),
            "ChromeImport" => return self.import(serde_json::from_value(arg(0)?)?),
            "RetryBrowser" => {
                let t = self.task(id)?;
                let item = t
                    .chrome
                    .ok_or_else(|| Error::new("chrome", "task has no import"))?;
                return self.import(vec![item]);
            }
            "GetInfo" => {
                return Ok(
                    json!({"version":env!("CARGO_PKG_VERSION"),"extensionId":native::EXTENSION_ID,"hostOk":native::registered(),"manifest":native::manifest_path(),"dataDir":system::data_dir()}),
                );
            }
            "Shutdown" => self.stop.store(true, Ordering::Relaxed),
            _ => return Err(Error::new("rpc", format!("unknown method: {method}"))),
        }
        Ok(Value::Null)
    }
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    pub fn handle(&self, request: Value) -> Value {
        let result = match request["type"].as_str() {
            Some("ping") => {
                let s = self.settings();
                Ok(
                    json!({"version":env!("CARGO_PKG_VERSION"),"capture":{"minSize":s.capture_min_size,"types":s.capture_types}}),
                )
            }
            Some("add") => {
                let name = request["filename"].as_str().unwrap_or("");
                self.add(AddRequest {
                    url: request["url"].as_str().unwrap_or("").into(),
                    name: if name.is_empty() {
                        String::new()
                    } else {
                        crate::sanitize::file_name(name)
                    },
                    cookies: request["cookies"].as_str().unwrap_or("").into(),
                    referrer: request["referrer"].as_str().unwrap_or("").into(),
                    user_agent: request["userAgent"].as_str().unwrap_or("").into(),
                    ..AddRequest::default()
                })
            }
            Some("rpc") => self.rpc(
                request["method"].as_str().unwrap_or(""),
                request["args"].as_array().map(Vec::as_slice).unwrap_or(&[]),
            ),
            _ => Err(Error::new("rpc", "invalid message")),
        };
        match result {
            Ok(v) => json!({"ok":true,"result":v}),
            Err(e) => json!({"ok":false,"error":e.to_string()}),
        }
    }
    pub fn serve(self: &Arc<Self>) -> Result<()> {
        let m = self.clone();
        ipc::serve(Arc::new(move |r| m.handle(r)))
    }
}
