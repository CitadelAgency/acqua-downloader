fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let addr = args
        .first()
        .map(String::as_str)
        .unwrap_or("127.0.0.1:18765");
    let size = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(64 << 20);
    let server = dlm::testserver::Server::bind(addr, size).expect("HTTP server");
    println!(
        "{}; size={size}; sha256={}",
        server.url("/full.bin"),
        dlm::testserver::hash(size)
    );
    loop {
        std::thread::park();
    }
}
