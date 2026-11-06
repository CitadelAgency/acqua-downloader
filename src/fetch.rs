use crate::{
    error::{Error, Result},
    sanitize,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    blocking::{Client, RequestBuilder, Response},
    header::HeaderMap,
};
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct Source {
    pub url: String,
    pub headers: HeaderMap,
}
impl Source {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Self::default()
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct Validator {
    pub etag: String,
    pub lm: String,
}
impl Validator {
    pub fn from_headers(h: &HeaderMap) -> Self {
        Self {
            etag: header(h, "etag").into(),
            lm: header(h, "last-modified").into(),
        }
    }
    pub fn strong(&self) -> bool {
        !self.etag.is_empty() && !self.etag.starts_with("W/")
    }
    pub fn if_range(&self) -> &str {
        if self.strong() { &self.etag } else { &self.lm }
    }
    pub fn conflicts(&self, other: &Self) -> bool {
        (self.strong() && other.strong() && self.etag != other.etag)
            || (!self.lm.is_empty() && !other.lm.is_empty() && self.lm != other.lm)
    }
}
#[derive(Debug, Clone)]
pub struct Digest {
    pub algo: String,
    pub sum: String,
    pub source: String,
}
impl Digest {
    pub fn value(&self) -> String {
        format!("{}:{}", self.algo, self.sum)
    }
}
pub fn user_hash(text: &str) -> Result<Digest> {
    let text = text.trim().to_lowercase();
    let (algo, sum) = text
        .split_once(':')
        .unwrap_or((if text.len() == 64 { "sha256" } else { "md5" }, &text));
    let algo = algo.replace('-', "");
    let bytes = hex::decode(sum).map_err(|_| Error::new("usage", "hash hexadecimal inválido"))?;
    if !matches!((algo.as_str(), bytes.len()), ("sha256", 32) | ("md5", 16)) {
        return Err(Error::new(
            "usage",
            "informe SHA-256 (64 caracteres) ou MD5 (32)",
        ));
    }
    Ok(Digest {
        algo,
        sum: sum.into(),
        source: "informado pelo usuário".into(),
    })
}
pub fn header<'a>(h: &'a HeaderMap, key: &str) -> &'a str {
    h.get(key).and_then(|v| v.to_str().ok()).unwrap_or("")
}
pub fn client() -> Result<Client> {
    Ok(Client::builder()
        .http1_only()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(60))
        .user_agent("dlm/2.0")
        .build()?)
}
pub fn validate_url(raw: &str) -> Result<url::Url> {
    let u = url::Url::parse(raw).map_err(|_| Error::new("usage", "URL inválida"))?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
        return Err(Error::new("usage", "somente URLs HTTP(S) são aceitas"));
    }
    Ok(u)
}
pub fn request(c: &Client, src: &Source) -> Result<RequestBuilder> {
    validate_url(&src.url)?;
    Ok(c.get(&src.url)
        .headers(src.headers.clone())
        .header("Accept-Encoding", "identity"))
}
pub fn content_range(raw: &str) -> Result<(u64, u64, u64)> {
    let parse = || -> Option<(u64, u64, u64)> {
        let rest = raw.trim().strip_prefix("bytes ")?;
        let (r, t) = rest.split_once('/')?;
        let (a, b) = r.split_once('-')?;
        Some((a.parse().ok()?, b.parse().ok()?, t.parse().ok()?))
    };
    let (a, b, t) = parse().filter(|(a, b, t)| a <= b && b < t).ok_or_else(|| {
        Error::new(
            "bad_content_range",
            format!("Content-Range inválido: {raw}"),
        )
    })?;
    Ok((a, b, t))
}
pub fn status(r: &Response) -> Error {
    let n = r.status().as_u16();
    let mut error = Error::new(
        if matches!(n, 401 | 403 | 404 | 410) {
            "needs_link"
        } else if matches!(n, 429 | 503) {
            "throttled"
        } else {
            "http_status"
        },
        format!(
            "HTTP {n}; Retry-After={}",
            header(r.headers(), "retry-after")
        ),
    );
    let raw = header(r.headers(), "retry-after");
    let seconds = raw
        .parse::<u64>()
        .ok()
        .or_else(|| {
            chrono::DateTime::parse_from_rfc2822(raw)
                .ok()
                .map(|t| (t.timestamp() - chrono::Utc::now().timestamp()).max(0) as u64)
        })
        .unwrap_or(0)
        .min(600);
    error.retry_after = Duration::from_secs(seconds);
    error
}
pub fn identity(r: &Response) -> Result<()> {
    let enc = header(r.headers(), "content-encoding");
    if !enc.is_empty() && !enc.eq_ignore_ascii_case("identity") {
        return Err(Error::new(
            "bad_content_range",
            "resposta comprimida em transferência de bytes",
        ));
    }
    Ok(())
}
pub struct Probe {
    pub response: Response,
    pub total: i64,
    pub ranges: bool,
    pub validator: Validator,
    pub name: String,
    pub digests: Vec<Digest>,
}
pub const PROBE_LEN: u64 = 8 << 20;
pub fn probe(c: &Client, src: &Source) -> Result<Probe> {
    let mut r = request(c, src)?
        .header("Range", format!("bytes=0-{}", PROBE_LEN - 1))
        .send()?;
    if !r.status().is_success() {
        return Err(status(&r));
    }
    let path = validate_url(&src.url)?.path().to_lowercase();
    if header(r.headers(), "content-type")
        .to_lowercase()
        .contains("text/html")
        && ![".html", ".htm", ".xhtml"]
            .iter()
            .any(|s| path.ends_with(s))
    {
        return Err(Error::new(
            "needs_link",
            "página HTML recebida no lugar do arquivo",
        ));
    }
    identity(&r)?;
    let mut ranges = false;
    let mut total = r.content_length().map(|n| n as i64).unwrap_or(-1);
    if r.status().as_u16() == 206 {
        if let Ok((0, b, t)) = content_range(header(r.headers(), "content-range")) {
            if b + 1 == t.min(PROBE_LEN) && r.content_length() == Some(b + 1) {
                ranges = true;
                total = t as i64;
            }
        }
        if !ranges {
            r = request(c, src)?.send()?;
            if r.status().as_u16() != 200 {
                return Err(status(&r));
            }
            identity(&r)?;
            total = r.content_length().map(|n| n as i64).unwrap_or(-1);
        }
    } else if r.status().as_u16() != 200 {
        return Err(status(&r));
    }
    let digests = header_digests(r.headers(), !ranges);
    Ok(Probe {
        validator: Validator::from_headers(r.headers()),
        name: sanitize::response_name(header(r.headers(), "content-disposition"), r.url()),
        response: r,
        total,
        ranges,
        digests,
    })
}
pub fn get_range(
    c: &Client,
    src: &Source,
    a: u64,
    b: u64,
    total: u64,
    v: &Validator,
) -> Result<Response> {
    if a >= b || b > total {
        return Err(Error::new("bad_content_range", "intervalo inválido"));
    }
    let mut req = request(c, src)?.header("Range", format!("bytes={a}-{}", b - 1));
    if !v.if_range().is_empty() {
        req = req.header("If-Range", v.if_range());
    }
    let r = req.send()?;
    if matches!(r.status().as_u16(), 200 | 412 | 416) {
        return Err(Error::new(
            "remote_changed",
            "servidor não confirmou a versão e o intervalo solicitados",
        ));
    }
    if r.status().as_u16() != 206 {
        return Err(status(&r));
    }
    identity(&r)?;
    if content_range(header(r.headers(), "content-range"))? != (a, b - 1, total)
        || r.content_length() != Some(b - a)
    {
        return Err(Error::new(
            "bad_content_range",
            "intervalo ou tamanho da resposta difere do pedido",
        ));
    }
    if v.conflicts(&Validator::from_headers(r.headers())) {
        return Err(Error::new("remote_changed", "ETag ou Last-Modified mudou"));
    }
    Ok(r)
}
pub fn header_digests(h: &HeaderMap, full: bool) -> Vec<Digest> {
    let mut out = Vec::new();
    for field in ["repr-digest", "digest", "content-md5"] {
        if field == "content-md5" && !full {
            continue;
        }
        for item in header(h, field).split(',') {
            let (algo, value) = if field == "content-md5" {
                ("md5", item)
            } else {
                match item.trim().split_once('=') {
                    Some(v) => v,
                    None => continue,
                }
            };
            let algo = algo.trim().to_lowercase().replace('-', "");
            if let Ok(bytes) = STANDARD.decode(value.trim().trim_matches(':')) {
                if matches!((algo.as_str(), bytes.len()), ("sha256", 32) | ("md5", 16)) {
                    out.push(Digest {
                        algo,
                        sum: hex::encode(bytes),
                        source: field.into(),
                    });
                }
            }
        }
    }
    out.sort_by_key(|d| if d.algo == "sha256" { 0 } else { 1 });
    out
}
pub fn sidecar(c: &Client, src: &Source, name: &str) -> Option<Digest> {
    use std::io::Read;
    let mut url = validate_url(&src.url).ok()?;
    if url.query().is_some() || url.path().ends_with('/') {
        return None;
    }
    url.set_path(&format!("{}.sha256", url.path()));
    url.set_fragment(None);
    let r = request(
        c,
        &Source {
            url: url.into(),
            headers: src.headers.clone(),
        },
    )
    .ok()?
    .timeout(Duration::from_secs(10))
    .send()
    .ok()?;
    if r.status().as_u16() != 200 {
        return None;
    }
    let mut text = String::new();
    r.take(65536).read_to_string(&mut text).ok()?;
    let mut candidates = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else { continue };
        if hash.len() != 64 || hex::decode(hash).is_err() {
            continue;
        }
        let d = Digest {
            algo: "sha256".into(),
            sum: hash.to_lowercase(),
            source: "sidecar .sha256".into(),
        };
        if parts
            .next()
            .is_some_and(|s| s.trim_start_matches('*').eq_ignore_ascii_case(name))
        {
            return Some(d);
        }
        candidates.push(d);
    }
    if candidates.len() == 1 {
        candidates.pop()
    } else {
        None
    }
}
pub fn url_expiry(raw: &str) -> Option<i64> {
    let url = url::Url::parse(raw).ok()?;
    let q: std::collections::HashMap<_, _> = url
        .query_pairs()
        .map(|(k, v)| (k.to_lowercase(), v.into_owned()))
        .collect();
    for key in ["expires", "expiration"] {
        if let Some(n) = q
            .get(key)
            .and_then(|s| s.parse::<i64>().ok())
            .filter(|n| (1_000_000_000..10_000_000_000).contains(n))
        {
            return Some(n);
        }
    }
    for prefix in ["x-amz-", "x-goog-"] {
        if let (Some(date), Some(n)) = (
            q.get(&format!("{prefix}date")),
            q.get(&format!("{prefix}expires"))
                .and_then(|s| s.parse::<i64>().ok()),
        ) {
            if let Ok(t) = chrono::NaiveDateTime::parse_from_str(date, "%Y%m%dT%H%M%SZ") {
                return Some(t.and_utc().timestamp() + n);
            }
        }
    }
    None
}
