use dlm::{
    engine::{Config, Download},
    fetch::Source,
    testserver,
};
fn main() {
    let size = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(256 << 20);
    let server = testserver::Server::start(size).unwrap();
    let expected = testserver::hash(size);
    let dir = tempfile::tempdir().unwrap();
    for connections in [1, 4, 16] {
        let begin = std::time::Instant::now();
        let dl = Download::new(Config {
            sources: vec![Source::new(server.url("/full.bin"))],
            dir: dir.path().into(),
            name: format!("bench-{connections}.bin"),
            max_conns: connections,
            expected_hash: format!("sha256:{expected}"),
            skip_sidecar: true,
            ..Config::default()
        });
        let result = dl.run().unwrap();
        assert_eq!(result.sha256, expected);
        println!(
            "{connections:2} conexões: {:7.1} MiB/s · SHA-256 ok",
            size as f64 / 1048576. / begin.elapsed().as_secs_f64()
        );
        std::fs::remove_file(result.path).unwrap();
    }
}
