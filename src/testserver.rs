//! Deterministic HTTP fixture and fault injection, shared by tests and benchmarks.
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread,
    time::Duration,
};
pub struct Server {
    pub addr: SocketAddr,
    pub version: Arc<AtomicU8>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    pub size: u64,
}
pub fn bytes(offset: u64, len: usize, version: u8) -> Vec<u8> {
    (offset..offset + len as u64)
        .map(|i| ((i.wrapping_mul(31) + i / 4096) % 251) as u8 ^ version)
        .collect()
}
pub fn hash(size: u64) -> String {
    let mut h = Sha256::new();
    for off in (0..size).step_by(1048576) {
        h.update(bytes(off, (size - off).min(1048576) as usize, 0));
    }
    hex::encode(h.finalize())
}
impl Server {
    pub fn start(size: u64) -> std::io::Result<Self> {
        Self::bind("127.0.0.1:0", size)
    }
    pub fn bind(addr: &str, size: u64) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let version = Arc::new(AtomicU8::new(0));
        let flag = stop.clone();
        let state = version.clone();
        let counts = Arc::new(Mutex::new(HashMap::new()));
        let worker = thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let version = state.clone();
                        let counts = counts.clone();
                        thread::spawn(move || {
                            let _ = serve(stream, size, version, counts);
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            addr,
            version,
            stop,
            worker: Some(worker),
            size,
        })
    }
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.worker.take() {
            let _ = t.join();
        }
    }
}
fn serve(
    mut stream: TcpStream,
    size: u64,
    version: Arc<AtomicU8>,
    counts: Arc<Mutex<HashMap<String, usize>>>,
) -> std::io::Result<()> {
    // No Windows o socket aceito herda o modo não bloqueante do listener.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut raw = Vec::new();
    let mut buf = [0; 2048];
    while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        raw.extend_from_slice(&buf[..n]);
        if raw.len() > 65536 {
            return Ok(());
        }
    }
    let text = String::from_utf8_lossy(&raw);
    let mut lines = text.lines();
    let request = lines.next().unwrap_or("");
    let raw_path = request.split_whitespace().nth(1).unwrap_or("/");
    let path = raw_path.split('?').next().unwrap_or(raw_path);
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.to_lowercase(), v.trim().to_string());
        }
    }
    let count = {
        let mut c = counts.lock().unwrap();
        let n = c.entry(path.into()).or_default();
        *n += 1;
        *n
    };
    if path == "/redirect.bin" {
        write!(
            stream,
            "HTTP/1.1 302 Found\r\nLocation: /full.bin\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )?;
        return Ok(());
    }
    if path.ends_with(".sha256") {
        let body = hash(size);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        return Ok(());
    }
    if path == "/html" {
        let body = "<html>link expired</html>";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        return Ok(());
    }
    if path == "/expired.bin" || (path == "/retry.bin" && count <= 2) {
        let status = if path == "/expired.bin" { 403 } else { 429 };
        write!(
            stream,
            "HTTP/1.1 {status} Error\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )?;
        return Ok(());
    }
    if let Some(expiry) = url::Url::parse(&format!("http://localhost{raw_path}"))
        .ok()
        .and_then(|u| crate::fetch::url_expiry(u.as_str()))
    {
        if chrono::Utc::now().timestamp() >= expiry {
            write!(
                stream,
                "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
            return Ok(());
        }
    }
    let v = version.load(Ordering::Relaxed);
    let validator = if path == "/changed.bin" && count > 1 {
        "\"changed\"".into()
    } else {
        format!("\"v{v}\"")
    };
    let range = if path == "/no-range.bin" || path == "/unknown.bin" {
        None
    } else {
        headers
            .get("range")
            .and_then(|r| r.strip_prefix("bytes="))
            .and_then(|r| r.split_once('-'))
            .and_then(|(a, b)| {
                Some((
                    a.parse::<u64>().ok()?,
                    b.parse::<u64>().ok()?.saturating_add(1).min(size),
                ))
            })
    };
    let (start, end) = range.unwrap_or((0, size));
    if start >= size && size > 0 {
        write!(
            stream,
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )?;
        return Ok(());
    }
    let force_full = path == "/range-200.bin" && headers.contains_key("if-range");
    write!(
        stream,
        "HTTP/1.1 {} OK\r\nConnection: close\r\nContent-Type: application/octet-stream\r\n",
        if range.is_some() && !force_full {
            206
        } else {
            200
        }
    )?;
    if path != "/unknown.bin" {
        write!(stream, "Content-Length: {}\r\n", end - start)?;
    }
    if path != "/no-validator.bin" {
        write!(stream, "ETag: {validator}\r\n")?;
    }
    if range.is_some() && !force_full {
        let a = if path == "/bad-range.bin" && headers.contains_key("if-range") {
            start + 1
        } else {
            start
        };
        write!(
            stream,
            "Content-Range: bytes {a}-{}/{size}\r\n",
            end.saturating_sub(1)
        )?;
    }
    if path == "/hash.bin" {
        use base64::Engine;
        write!(
            stream,
            "Repr-Digest: sha-256=:{}:\r\n",
            base64::engine::general_purpose::STANDARD.encode(hex::decode(hash(size)).unwrap())
        )?;
    }
    write!(stream, "\r\n")?;
    let end = if path == "/drop.bin" && count == 2 {
        start + (end - start) / 2
    } else {
        end
    };
    for off in (start..end).step_by(65536) {
        if path == "/slow.bin" || path == "/no-validator.bin" {
            thread::sleep(Duration::from_millis(8));
        }
        stream.write_all(&bytes(off, (end - off).min(65536) as usize, v))?;
    }
    Ok(())
}
