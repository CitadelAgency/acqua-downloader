use crate::{
    error::{Error, Result},
    ranges::{Range, RangeSet},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    pub v: u32,
    pub url: String,
    pub mirrors: Vec<String>,
    pub name: String,
    pub total: i64,
    pub bs: u64,
    pub ranges: bool,
    pub etag: String,
    pub lm: String,
    pub hash: String,
    pub hash_src: String,
    pub imported: String,
    #[serde(rename = "ref")]
    pub referrer: String,
}
pub struct Journal {
    pub path: PathBuf,
    pub meta: Meta,
    pub done: RangeSet,
    pub blocks: BTreeMap<u64, u64>,
    pub final_record: String,
    file: File,
}
fn record(payload: &str) -> String {
    format!("{payload}\t{:08x}\n", crc32c::crc32c(payload.as_bytes()))
}
impl Journal {
    pub fn create(path: &Path, mut meta: Meta, done: RangeSet) -> Result<Self> {
        meta.v = 1;
        meta.bs = 4 << 20;
        let mut f = OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .open(path)?;
        f.write_all(record(&format!("H {}", serde_json::to_string(&meta)?)).as_bytes())?;
        for r in &done.0 {
            f.write_all(record(&format!("R {} {}", r.start, r.end)).as_bytes())?;
        }
        f.sync_all()?;
        Ok(Self {
            path: path.into(),
            meta,
            done,
            blocks: BTreeMap::new(),
            final_record: String::new(),
            file: f,
        })
    }
    pub fn open(path: &Path) -> Result<Self> {
        let f = OpenOptions::new().read(true).write(true).open(path)?;
        let mut reader = BufReader::new(f.try_clone()?);
        let mut valid = 0u64;
        let mut header = false;
        let mut j = Self {
            path: path.into(),
            file: f,
            meta: Meta::default(),
            done: RangeSet::default(),
            blocks: BTreeMap::new(),
            final_record: String::new(),
        };
        loop {
            let mut bytes = Vec::new();
            let n = reader.read_until(b'\n', &mut bytes)?;
            if n == 0 {
                break;
            }
            if !bytes.ends_with(b"\n") {
                break;
            }
            let Ok(line) = std::str::from_utf8(&bytes) else {
                break;
            };
            let Some((payload, crc)) = line.trim_end_matches('\n').rsplit_once('\t') else {
                break;
            };
            if u32::from_str_radix(crc, 16).ok() != Some(crc32c::crc32c(payload.as_bytes())) {
                break;
            }
            if j.replay(payload).is_err() {
                break;
            }
            if payload.starts_with("H ") {
                header = true;
            }
            valid += n as u64;
        }
        if !header || j.meta.v != 1 || j.meta.bs != 4 << 20 {
            return Err(Error::new(
                "journal",
                "cabeçalho inválido ou versão incompatível",
            ));
        }
        if j.meta.total >= 0 && j.done.0.iter().any(|r| r.end > j.meta.total as u64) {
            return Err(Error::new("journal", "intervalo fora do arquivo"));
        }
        j.file.set_len(valid)?;
        j.file.seek(SeekFrom::Start(valid))?;
        Ok(j)
    }
    fn replay(&mut self, s: &str) -> Result<()> {
        let (kind, rest) = s
            .split_once(' ')
            .ok_or_else(|| Error::new("journal", "registro inválido"))?;
        let nums = || -> Result<(u64, u64)> {
            let (a, b) = rest
                .split_once(' ')
                .ok_or_else(|| Error::new("journal", "intervalo inválido"))?;
            Ok((
                a.parse()
                    .map_err(|_| Error::new("journal", "offset inválido"))?,
                b.parse()
                    .map_err(|_| Error::new("journal", "offset inválido"))?,
            ))
        };
        match kind {
            "H" => self.meta = serde_json::from_str(rest)?,
            "R" => {
                let (a, b) = nums()?;
                if b < a {
                    return Err(Error::new("journal", "intervalo invertido"));
                }
                self.done.add(Range::new(a, b));
            }
            "X" => {
                let (a, b) = nums()?;
                self.retract_memory(Range::new(a, b));
            }
            "B" => {
                let (a, b) = rest
                    .split_once(' ')
                    .ok_or_else(|| Error::new("journal", "bloco inválido"))?;
                self.blocks.insert(
                    a.parse()
                        .map_err(|_| Error::new("journal", "bloco inválido"))?,
                    u64::from_str_radix(b, 16)
                        .map_err(|_| Error::new("journal", "hash inválido"))?,
                );
            }
            "F" => self.final_record = rest.into(),
            _ => return Err(Error::new("journal", "registro desconhecido")),
        }
        Ok(())
    }
    pub fn append(&mut self, payloads: &[String]) -> Result<()> {
        for p in payloads {
            self.file.write_all(record(p).as_bytes())?;
        }
        self.file.sync_all()?;
        for p in payloads {
            self.replay(p)?;
        }
        Ok(())
    }
    pub fn commit(&mut self, ranges: &[Range], blocks: &[(u64, u64)]) -> Result<()> {
        let mut p: Vec<_> = ranges
            .iter()
            .map(|r| format!("R {} {}", r.start, r.end))
            .collect();
        p.extend(blocks.iter().map(|(i, h)| format!("B {i} {h:016x}")));
        self.append(&p)
    }
    fn retract_memory(&mut self, r: Range) {
        self.done.remove(r);
        let bs = self.meta.bs.max(1);
        self.blocks
            .retain(|i, _| *i * bs >= r.end || (*i + 1) * bs <= r.start);
    }
    pub fn retract(&mut self, r: Range) -> Result<()> {
        self.append(&[format!("X {} {}", r.start, r.end)])
    }
    pub fn set_meta(&mut self, meta: Meta) -> Result<()> {
        self.append(&[format!("H {}", serde_json::to_string(&meta)?)])
    }
    pub fn set_final(&mut self, hash: &str, path: &Path) -> Result<()> {
        self.append(&[format!("F {hash} {}", path.display())])
    }
}
