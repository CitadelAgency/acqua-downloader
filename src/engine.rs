use crate::{
    error::{Error, Result},
    fetch::{self, Source, Validator},
    journal::{Journal, Meta},
    ranges::{Range, RangeSet},
    sanitize, system,
};
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
pub const BLOCK: u64 = 4 << 20;
pub const SEAM: u64 = 64 << 10;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HashCheck {
    pub checked: bool,
    pub ok: bool,
    pub algo: String,
    pub expected: String,
    pub source: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadResult {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub check: HashCheck,
}
#[derive(Debug, Default, Clone, Serialize)]
pub struct Snapshot {
    pub state: String,
    pub name: String,
    pub total: i64,
    pub done: u64,
    pub verified: u64,
    pub speed: f64,
    pub eta: f64,
    pub conns: usize,
    pub ranges: bool,
    pub segments: Vec<Segment>,
    #[serde(rename = "doneRanges")]
    pub done_ranges: Vec<Range>,
    pub sources: Vec<serde_json::Value>,
    pub warnings: Vec<String>,
    #[serde(skip)]
    pub final_path: String,
    #[serde(skip)]
    pub part_path: String,
    #[serde(skip)]
    pub journal_path: String,
}
#[derive(Debug, Default, Clone, Serialize)]
pub struct Segment {
    pub s: u64,
    pub p: u64,
    pub e: u64,
    pub v: bool,
}
pub struct RateLimiter {
    bps: AtomicU64,
    next: Mutex<Instant>,
}
impl RateLimiter {
    pub fn new(bps: u64) -> Self {
        Self {
            bps: AtomicU64::new(bps),
            next: Mutex::new(Instant::now()),
        }
    }
    pub fn set(&self, bps: u64) {
        self.bps.store(bps, Ordering::Relaxed);
        *self.next.lock().unwrap() = Instant::now();
    }
    fn wait(&self, n: usize, cancel: &AtomicBool) -> Result<()> {
        let bps = self.bps.load(Ordering::Relaxed);
        if bps == 0 {
            return Ok(());
        }
        let deadline = {
            let mut next = self.next.lock().unwrap();
            *next = (*next).max(Instant::now()) + Duration::from_secs_f64(n as f64 / bps as f64);
            *next
        };
        while Instant::now() < deadline {
            check(cancel)?;
            std::thread::sleep((deadline - Instant::now()).min(Duration::from_millis(50)));
        }
        Ok(())
    }
}
#[derive(Default)]
pub struct HostLimiter {
    counts: Mutex<std::collections::HashMap<String, usize>>,
    pub max: AtomicU64,
}
impl HostLimiter {
    pub fn new(max: u64) -> Self {
        Self {
            max: AtomicU64::new(max),
            ..Self::default()
        }
    }
    fn acquire(self: &Arc<Self>, url: &str, cancel: &AtomicBool) -> Result<HostPermit> {
        let host = fetch::validate_url(url)?
            .host_str()
            .unwrap_or("")
            .to_lowercase();
        loop {
            check(cancel)?;
            {
                let mut counts = self.counts.lock().unwrap();
                let n = counts.entry(host.clone()).or_default();
                if *n < self.max.load(Ordering::Relaxed).max(1) as usize {
                    *n += 1;
                    return Ok(HostPermit(self.clone(), host));
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
struct HostPermit(Arc<HostLimiter>, String);
impl Drop for HostPermit {
    fn drop(&mut self) {
        if let Some(n) = self.0.counts.lock().unwrap().get_mut(&self.1) {
            *n = n.saturating_sub(1);
        }
    }
}
#[derive(Clone)]
pub struct Config {
    pub sources: Vec<Source>,
    pub dir: PathBuf,
    pub name: String,
    pub journal_path: PathBuf,
    pub part_path: PathBuf,
    pub expected_hash: String,
    pub max_conns: usize,
    pub limiters: Vec<Arc<RateLimiter>>,
    pub hosts: Arc<HostLimiter>,
    pub skip_sidecar: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            dir: PathBuf::from("."),
            name: String::new(),
            journal_path: PathBuf::new(),
            part_path: PathBuf::new(),
            expected_hash: String::new(),
            max_conns: 16,
            limiters: Vec::new(),
            hosts: Arc::new(HostLimiter::new(16)),
            skip_sidecar: false,
        }
    }
}
pub struct Download {
    pub cfg: Config,
    pub cancel: Arc<AtomicBool>,
    pub live: Arc<Mutex<Snapshot>>,
}
pub fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::new("paused", "download pausado"))
    } else {
        Ok(())
    }
}
pub fn read_at(file: &File, buf: &mut [u8], mut at: u64) -> Result<()> {
    let mut done = 0;
    while done < buf.len() {
        #[cfg(windows)]
        let n = {
            use std::os::windows::fs::FileExt;
            file.seek_read(&mut buf[done..], at)?
        };
        #[cfg(unix)]
        let n = {
            use std::os::unix::fs::FileExt;
            file.read_at(&mut buf[done..], at)?
        };
        if n == 0 {
            return Err(Error::new("other", "arquivo parcial truncado"));
        }
        done += n;
        at += n as u64;
    }
    Ok(())
}
fn write_at(file: &File, buf: &[u8], mut at: u64) -> Result<()> {
    let mut done = 0;
    while done < buf.len() {
        #[cfg(windows)]
        let n = {
            use std::os::windows::fs::FileExt;
            file.seek_write(&buf[done..], at)?
        };
        #[cfg(unix)]
        let n = {
            use std::os::unix::fs::FileExt;
            file.write_at(&buf[done..], at)?
        };
        if n == 0 {
            return Err(Error::new("no_space", "falha ao gravar arquivo parcial"));
        }
        done += n;
        at += n as u64;
    }
    Ok(())
}
fn block_hash(file: &File, index: u64, total: u64) -> Result<u64> {
    let start = index * BLOCK;
    let mut b = vec![0; (total - start).min(BLOCK) as usize];
    read_at(file, &mut b, start)?;
    Ok(xxhash_rust::xxh3::xxh3_64(&b))
}
impl Download {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            cancel: Arc::new(AtomicBool::new(false)),
            live: Arc::new(Mutex::new(Snapshot {
                state: "starting".into(),
                total: -1,
                eta: -1.,
                ..Snapshot::default()
            })),
        }
    }
    pub fn run(&self) -> Result<DownloadResult> {
        let r = self.run_inner();
        let mut s = self.live.lock().unwrap();
        s.state = if r.is_ok() {
            "done"
        } else if self.cancel.load(Ordering::Relaxed) {
            "paused"
        } else {
            "error"
        }
        .into();
        s.conns = 0;
        s.speed = 0.;
        r
    }
    fn run_inner(&self) -> Result<DownloadResult> {
        let src = self
            .cfg
            .sources
            .first()
            .ok_or_else(|| Error::new("usage", "nenhuma URL"))?;
        fetch::validate_url(&src.url)?;
        if !self.cfg.expected_hash.is_empty() {
            fetch::user_hash(&self.cfg.expected_hash)?;
        }
        std::fs::create_dir_all(&self.cfg.dir)?;
        self.live.lock().unwrap().state = "probing".into();
        let c = fetch::client()?;
        let mut jp = self.cfg.journal_path.clone();
        if jp.as_os_str().is_empty() && !self.cfg.name.is_empty() {
            jp = self
                .cfg
                .dir
                .join(format!("{}.dlm", sanitize::file_name(&self.cfg.name)));
        }
        let mut journal = if jp.exists() {
            Some(Journal::open(&jp)?)
        } else {
            None
        };
        let mut probe = None;
        if journal.is_none() {
            let mut attempt = 0;
            let p = loop {
                check(&self.cancel)?;
                match fetch::probe(&c, src) {
                    Ok(p) => break p,
                    Err(e) if e.code == "throttled" && attempt < 4 => {
                        attempt += 1;
                        pause(
                            e.retry_after.max(Duration::from_secs(attempt * 2)),
                            &self.cancel,
                        )?;
                    }
                    Err(e) => return Err(e),
                }
            };
            let name = if self.cfg.name.is_empty() {
                p.name.clone()
            } else {
                sanitize::file_name(&self.cfg.name)
            };
            if jp.as_os_str().is_empty() {
                jp = self.cfg.dir.join(format!("{name}.dlm"));
            }
            if jp.exists() {
                let j = Journal::open(&jp)?;
                if j.meta.total != p.total || validator(&j.meta).conflicts(&p.validator) {
                    return Err(Error::new(
                        "remote_changed",
                        "já existe um parcial deste nome de outra versão",
                    ));
                }
                journal = Some(j);
            } else {
                let mut meta = Meta {
                    url: src.url.clone(),
                    name,
                    total: p.total,
                    ranges: p.ranges,
                    etag: p.validator.etag.clone(),
                    lm: p.validator.lm.clone(),
                    mirrors: self
                        .cfg
                        .sources
                        .iter()
                        .skip(1)
                        .map(|s| s.url.clone())
                        .collect(),
                    ..Meta::default()
                };
                if let Some(d) = p.digests.first() {
                    meta.hash = d.value();
                    meta.hash_src = d.source.clone();
                }
                journal = Some(Journal::create(&jp, meta, RangeSet::default())?);
                probe = Some(p);
            }
        }
        let mut j = journal.unwrap();
        let final_path = PathBuf::from(
            jp.to_string_lossy()
                .strip_suffix(".dlm")
                .ok_or_else(|| Error::new("usage", "journal deve terminar em .dlm"))?,
        );
        let part = if self.cfg.part_path.as_os_str().is_empty() {
            PathBuf::from(format!("{}.dlmpart", final_path.display()))
        } else {
            self.cfg.part_path.clone()
        };
        if !j.final_record.is_empty() && !part.exists() {
            if let Some((sum, path)) = j.final_record.split_once(' ') {
                if Path::new(path).exists() {
                    let mut f = File::open(path)?;
                    let mut hasher = Sha256::new();
                    std::io::copy(&mut f, &mut hasher)?;
                    if hex::encode(hasher.finalize()) == sum {
                        let res = DownloadResult {
                            path: path.into(),
                            size: std::fs::metadata(path)?.len(),
                            sha256: sum.into(),
                            check: HashCheck::default(),
                        };
                        drop(j);
                        std::fs::remove_file(&jp)?;
                        return Ok(res);
                    }
                }
            }
            return Err(Error::new(
                "hash_mismatch",
                "arquivo final após crash não confere",
            ));
        }
        if !j.meta.ranges && probe.is_none() {
            let p = fetch::probe(&c, src)?;
            let meta = Meta {
                url: src.url.clone(),
                name: j.meta.name.clone(),
                total: p.total,
                ranges: p.ranges,
                etag: p.validator.etag.clone(),
                lm: p.validator.lm.clone(),
                ..j.meta.clone()
            };
            let done = j.done.0.clone();
            for r in done {
                j.retract(r)?;
            }
            j.set_meta(meta)?;
            probe = Some(p);
        }
        if !self.cfg.expected_hash.is_empty() {
            let mut m = j.meta.clone();
            let d = fetch::user_hash(&self.cfg.expected_hash)?;
            m.hash = d.value();
            m.hash_src = d.source;
            j.set_meta(m)?;
        }
        if j.done.covered() > 0 && !part.exists() {
            return Err(Error::new(
                "other",
                "arquivo parcial ausente; o journal não pode ser retomado",
            ));
        }
        if j.meta.total >= 0 {
            system::ensure_space(
                part.parent().unwrap_or(&self.cfg.dir),
                (j.meta.total as u64).saturating_sub(j.done.covered()),
            )?;
        }
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&part)?;
        if !j.meta.ranges {
            f.set_len(0)?;
        }
        if j.meta.total >= 0 {
            if f.metadata()?.len() > j.meta.total as u64 {
                return Err(Error::new(
                    "part_too_large",
                    "arquivo parcial maior que o total",
                ));
            }
            system::sparse(&f);
            f.set_len(j.meta.total as u64)?;
        } else if !j.meta.ranges {
            f.set_len(0)?;
        }
        {
            let mut s = self.live.lock().unwrap();
            s.name = j.meta.name.clone();
            s.total = j.meta.total;
            s.ranges = j.meta.ranges;
            s.done = j.done.covered();
            s.done_ranges = j.done.0.clone();
            s.final_path = final_path.display().to_string();
            s.part_path = part.display().to_string();
            s.journal_path = jp.display().to_string();
            s.sources=self.cfg.sources.iter().map(|s|serde_json::json!({"host":url::Url::parse(&s.url).ok().and_then(|u|u.host_str().map(str::to_string)).unwrap_or_default(),"dead":false})).collect();
            if !s.ranges {
                s.warnings.push("no_resume".into());
            }
        }
        if j.meta.ranges {
            let total = j.meta.total as u64;
            for (i, want) in j.blocks.clone() {
                check(&self.cancel)?;
                if block_hash(&f, i, total)? != want {
                    j.retract(Range::new(i * BLOCK, ((i + 1) * BLOCK).min(total)))?;
                }
            }
            // Prove every resume frontier before allowing any worker to write.
            for r in j.done.0.clone() {
                if r.end < total {
                    let start = r.start.max(r.end.saturating_sub(SEAM));
                    compare(
                        &c,
                        src,
                        &f,
                        Range::new(start, r.end),
                        total,
                        &validator(&j.meta),
                        "seam_mismatch",
                    )?;
                }
            }
            // Holes before existing bytes also need proof (a gap at zero has no seam).
            if j.done.covered() > 0 {
                prove_identity(&c, src, total, &validator(&j.meta), &j.done, &f)?;
            }
            let mut sources = vec![src.clone()];
            for mirror in self.cfg.sources.iter().skip(1) {
                let p = fetch::probe(&c, mirror)?;
                let v = validator(&j.meta);
                if p.total != j.meta.total
                    || !v.strong()
                    || !p.validator.strong()
                    || v.etag != p.validator.etag
                {
                    return Err(Error::new(
                        "not_same_file",
                        "espelho precisa ter mesmo tamanho e ETag forte",
                    ));
                }
                sources.push(mirror.clone());
            }
            drop(probe.take());
            for round in 0..3 {
                self.transfer_ranges(&f, &mut j, &sources)?;
                let bad = self.verify_blocks(&f, &j)?;
                if bad.is_empty() {
                    break;
                }
                if round == 2 {
                    return Err(Error::new(
                        "hash_mismatch",
                        "blocos locais continuam divergentes",
                    ));
                }
                for i in bad {
                    j.retract(Range::new(i * BLOCK, ((i + 1) * BLOCK).min(total)))?;
                }
            }
        } else {
            let p = probe
                .take()
                .ok_or_else(|| Error::new("other", "resposta inicial ausente"))?;
            self.stream(&f, &mut j, p.response)?;
        }
        check(&self.cancel)?;
        self.live.lock().unwrap().state = "verifying".into();
        let mut sha = Sha256::new();
        let mut md = Md5::new();
        let total = j.meta.total.max(0) as u64;
        let mut buf = vec![0; BLOCK as usize];
        for off in (0..total).step_by(BLOCK as usize) {
            check(&self.cancel)?;
            let n = (total - off).min(BLOCK) as usize;
            read_at(&f, &mut buf[..n], off)?;
            sha.update(&buf[..n]);
            md.update(&buf[..n]);
            self.live.lock().unwrap().verified = off + n as u64;
        }
        let sum = hex::encode(sha.finalize());
        let mut expected = if j.meta.hash.is_empty() {
            None
        } else {
            Some(fetch::user_hash(&j.meta.hash)?)
        };
        if let Some(d) = expected.as_mut() {
            d.source = j.meta.hash_src.clone();
        }
        if expected.is_none() && !self.cfg.skip_sidecar {
            expected = fetch::sidecar(&c, src, &j.meta.name);
        }
        let mut hash_check = HashCheck::default();
        if let Some(d) = expected {
            let got = if d.algo == "md5" {
                hex::encode(md.finalize())
            } else {
                sum.clone()
            };
            if got != d.sum {
                return Err(Error::new(
                    "hash_mismatch",
                    format!("{} diverge: esperado {}, obtido {}", d.algo, d.sum, got),
                ));
            }
            hash_check = HashCheck {
                checked: true,
                ok: true,
                algo: d.algo,
                expected: d.sum,
                source: d.source,
            };
        }
        f.sync_all()?;
        // MOTW belongs to the file before it becomes visible under its final name.
        system::motw(&part, &src.url)?;
        drop(f);
        let mut index = 0;
        let destination = loop {
            let dest = if index == 0 {
                final_path.clone()
            } else {
                let stem = final_path.file_stem().unwrap_or_default().to_string_lossy();
                let ext = final_path
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                final_path.with_file_name(format!("{stem} ({index}){ext}"))
            };
            j.set_final(&sum, &dest)?;
            match system::rename_no_replace(&part, &dest) {
                Ok(()) => break dest,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    index += 1;
                }
                Err(e) => return Err(e.into()),
            }
        };
        drop(j);
        std::fs::remove_file(&jp)?;
        Ok(DownloadResult {
            path: destination.display().to_string(),
            size: total,
            sha256: sum,
            check: hash_check,
        })
    }
    fn transfer_ranges(&self, f: &File, j: &mut Journal, sources: &[Source]) -> Result<()> {
        let total = j.meta.total as u64;
        let v = validator(&j.meta);
        let count = self.cfg.max_conns.clamp(1, 32);
        let mut tasks = VecDeque::new();
        // Small pieces are claimed dynamically; fast lanes take work left by slower ones.
        let expiring = sources.iter().any(|s| {
            fetch::url_expiry(&s.url).is_some_and(|t| t <= chrono::Utc::now().timestamp() + 60)
        });
        let chunk = if expiring {
            total.div_ceil(count as u64).max(BLOCK)
        } else {
            (total / count as u64).clamp(BLOCK, 8 * BLOCK)
        };
        for gap in j.done.missing(total) {
            let mut pos = gap.start;
            while pos < gap.end {
                let end = (pos + chunk).min(gap.end);
                tasks.push_back(Range::new(pos, end));
                pos = end;
            }
        }
        let work = Mutex::new(tasks);
        let journal = Mutex::new(j);
        let failure: Mutex<Option<Error>> = Mutex::new(None);
        let target = AtomicU64::new(count as u64);
        let grow_at = Mutex::new(Instant::now());
        let stop = AtomicBool::new(false);
        let start = Instant::now();
        let base = self.live.lock().unwrap().done;
        self.live.lock().unwrap().state = "downloading".into();
        std::thread::scope(|scope| {
            for lane in 0..count {
                let work = &work;
                let journal = &journal;
                let failure = &failure;
                let stop = &stop;
                let v = &v;
                let target = &target;
                let grow_at = &grow_at;
                scope.spawn(move || {
                    let result = (|| -> Result<()> {
                        let c = fetch::client()?;
                        while !stop.load(Ordering::Relaxed) {
                            check(&self.cancel)?;
                            if lane as u64 >= target.load(Ordering::Relaxed) {
                                if work.lock().unwrap().is_empty() {
                                    break;
                                }
                                pause(Duration::from_millis(100), &self.cancel)?;
                                continue;
                            }
                            let Some(r) = work.lock().unwrap().pop_front() else {
                                break;
                            };
                            let mut pos = r.start;
                            let mut attempts = 0;
                            let mut source_index = lane % sources.len();
                            {
                                let mut live = self.live.lock().unwrap();
                                live.conns += 1;
                                live.segments.push(Segment {
                                    s: r.start,
                                    p: pos,
                                    e: r.end,
                                    v: false,
                                });
                            }
                            let transfer = (|| -> Result<()> {
                                while pos < r.end {
                                    check(&self.cancel)?;
                                    if stop.load(Ordering::Relaxed) {
                                        return Ok(());
                                    }
                                    let src = &sources[source_index];
                                    let _permit = self.cfg.hosts.acquire(&src.url, &self.cancel)?;
                                    let response = fetch::get_range(&c, src, pos, r.end, total, v);
                                    let result = (|| -> Result<()> {
                                        let mut response = response?;
                                        let mut buf = vec![0; BLOCK as usize];
                                        while pos < r.end {
                                            check(&self.cancel)?;
                                            if stop.load(Ordering::Relaxed) {
                                                return Ok(());
                                            }
                                            let want = (r.end - pos).min(BLOCK) as usize;
                                            let mut n = 0;
                                            while n < want {
                                                check(&self.cancel)?;
                                                let size = response
                                                    .read(&mut buf[n..want.min(n + 262144)])
                                                    .map_err(|e| {
                                                        Error::new("network", e.to_string())
                                                    })?;
                                                if size == 0 {
                                                    return Err(Error::new(
                                                        "network",
                                                        "resposta interrompida",
                                                    ));
                                                }
                                                for limiter in &self.cfg.limiters {
                                                    limiter.wait(size, &self.cancel)?;
                                                }
                                                n += size;
                                            }
                                            write_at(f, &buf[..n], pos)?;
                                            let range = Range::new(pos, pos + n as u64);
                                            {
                                                let mut j = journal.lock().unwrap();
                                                f.sync_data()?;
                                                let mut done = j.done.clone();
                                                done.add(range);
                                                let mut hashes = Vec::new();
                                                for i in
                                                    range.start / BLOCK..=(range.end - 1) / BLOCK
                                                {
                                                    let block = Range::new(
                                                        i * BLOCK,
                                                        ((i + 1) * BLOCK).min(total),
                                                    );
                                                    if !j.blocks.contains_key(&i)
                                                        && done.contains(block)
                                                    {
                                                        hashes.push((i, block_hash(f, i, total)?));
                                                    }
                                                }
                                                j.commit(&[range], &hashes)?;
                                                let mut s = self.live.lock().unwrap();
                                                s.done = j.done.covered();
                                                s.done_ranges = j.done.0.clone();
                                                s.speed = (s.done.saturating_sub(base)) as f64
                                                    / start.elapsed().as_secs_f64().max(0.001);
                                                s.eta = (total.saturating_sub(s.done)) as f64
                                                    / s.speed.max(1.);
                                                if let Some(seg) = s
                                                    .segments
                                                    .iter_mut()
                                                    .find(|seg| seg.s == r.start)
                                                {
                                                    seg.p = range.end;
                                                }
                                            }
                                            pos = range.end;
                                            attempts = 0;
                                        }
                                        Ok(())
                                    })();
                                    match result {
                                        Ok(()) => {
                                            let mut last = grow_at.lock().unwrap();
                                            if last.elapsed() > Duration::from_secs(3) {
                                                target
                                                    .fetch_update(
                                                        Ordering::Relaxed,
                                                        Ordering::Relaxed,
                                                        |n| Some((n + 1).min(count as u64)),
                                                    )
                                                    .ok();
                                                *last = Instant::now();
                                            }
                                        }
                                        Err(e)
                                            if !e.integrity()
                                                && e.code != "paused"
                                                && e.code != "no_space"
                                                && attempts < 8 =>
                                        {
                                            attempts += 1;
                                            if e.code == "throttled" {
                                                target
                                                    .fetch_update(
                                                        Ordering::Relaxed,
                                                        Ordering::Relaxed,
                                                        |n| Some((n / 2).max(1)),
                                                    )
                                                    .ok();
                                                *grow_at.lock().unwrap() = Instant::now();
                                            }
                                            source_index = (source_index + 1) % sources.len();
                                            drop(_permit);
                                            pause(
                                                e.retry_after.max(Duration::from_millis(
                                                    if e.code == "needs_link" {
                                                        300
                                                    } else {
                                                        (500u64 << attempts.min(5)).min(15000)
                                                    },
                                                )),
                                                &self.cancel,
                                            )?;
                                        }
                                        Err(e) => return Err(e),
                                    }
                                }
                                Ok(())
                            })();
                            {
                                let mut s = self.live.lock().unwrap();
                                s.conns = s.conns.saturating_sub(1);
                                s.segments.retain(|seg| seg.s != r.start);
                            }
                            transfer?;
                        }
                        Ok(())
                    })();
                    if let Err(e) = result {
                        let mut err = failure.lock().unwrap();
                        if err.is_none() {
                            *err = Some(e);
                        }
                        stop.store(true, Ordering::Relaxed);
                    }
                });
            }
        });
        if let Some(e) = failure.into_inner().unwrap() {
            return Err(e);
        }
        Ok(())
    }
    fn stream(
        &self,
        f: &File,
        j: &mut Journal,
        mut response: reqwest::blocking::Response,
    ) -> Result<()> {
        {
            let mut s = self.live.lock().unwrap();
            s.state = "downloading".into();
            s.conns = 1;
        }
        let start = Instant::now();
        let mut pos = 0;
        let mut buf = vec![0; BLOCK as usize];
        loop {
            check(&self.cancel)?;
            let mut n = 0;
            while n < buf.len() {
                check(&self.cancel)?;
                let k = response
                    .read(&mut buf[n..])
                    .map_err(|e| Error::new("network", e.to_string()))?;
                if k == 0 {
                    break;
                }
                for limiter in &self.cfg.limiters {
                    limiter.wait(k, &self.cancel)?;
                }
                n += k;
            }
            if n == 0 {
                break;
            }
            write_at(f, &buf[..n], pos)?;
            f.sync_data()?;
            j.commit(&[Range::new(pos, pos + n as u64)], &[])?;
            pos += n as u64;
            let mut s = self.live.lock().unwrap();
            s.done = pos;
            s.speed = pos as f64 / start.elapsed().as_secs_f64().max(0.001);
            s.done_ranges = j.done.0.clone();
        }
        if j.meta.total >= 0 && pos != j.meta.total as u64 {
            return Err(Error::new("network", "tamanho da resposta incompleto"));
        }
        let mut meta = j.meta.clone();
        meta.total = pos as i64;
        j.set_meta(meta)?;
        self.live.lock().unwrap().total = pos as i64;
        Ok(())
    }
    fn verify_blocks(&self, f: &File, j: &Journal) -> Result<Vec<u64>> {
        self.live.lock().unwrap().state = "verifying".into();
        let mut bad = Vec::new();
        for (i, h) in &j.blocks {
            check(&self.cancel)?;
            if block_hash(f, *i, j.meta.total as u64)? != *h {
                bad.push(*i);
            }
        }
        Ok(bad)
    }
}
fn pause(duration: Duration, cancel: &AtomicBool) -> Result<()> {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        check(cancel)?;
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
pub fn validator(m: &Meta) -> Validator {
    Validator {
        etag: m.etag.clone(),
        lm: m.lm.clone(),
    }
}
fn compare(
    c: &reqwest::blocking::Client,
    src: &Source,
    f: &File,
    r: Range,
    total: u64,
    v: &Validator,
    code: &str,
) -> Result<()> {
    let mut resp = fetch::get_range(c, src, r.start, r.end, total, v)?;
    let mut remote = vec![0; r.len() as usize];
    resp.read_exact(&mut remote)
        .map_err(|e| Error::new("network", e.to_string()))?;
    let mut local = vec![0; remote.len()];
    read_at(f, &mut local, r.start)?;
    if local != remote {
        return Err(Error::new(code, format!("bytes diferentes em {}", r.start)));
    }
    Ok(())
}
pub fn prove_identity(
    c: &reqwest::blocking::Client,
    src: &Source,
    total: u64,
    v: &Validator,
    done: &RangeSet,
    f: &File,
) -> Result<Validator> {
    let p = fetch::probe(c, src)?;
    if p.total != total as i64 || !p.ranges || v.conflicts(&p.validator) {
        return Err(Error::new("not_same_file", "tamanho ou validador difere"));
    }
    let found = p.validator.clone();
    drop(p);
    let covered = done.covered();
    for at in [0, covered / 2, covered.saturating_sub(SEAM)] {
        let mut acc = 0;
        for r in &done.0 {
            if at < acc + r.len() {
                let offset = r.start + at - acc;
                compare(
                    c,
                    src,
                    f,
                    Range::new(offset, (offset + SEAM).min(r.end)),
                    total,
                    &found,
                    "not_same_file",
                )?;
                break;
            }
            acc += r.len();
        }
    }
    Ok(found)
}
pub fn relink(jp: &Path, part: &Path, src: &Source) -> Result<Meta> {
    let mut j = Journal::open(jp)?;
    let f = File::open(part)?;
    let v = prove_identity(
        &fetch::client()?,
        src,
        j.meta.total as u64,
        &validator(&j.meta),
        &j.done,
        &f,
    )?;
    let mut m = j.meta.clone();
    m.url = src.url.clone();
    m.etag = v.etag;
    m.lm = v.lm;
    j.set_meta(m.clone())?;
    Ok(m)
}
