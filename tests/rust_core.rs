use dlm::{
    engine::{Config, Download},
    fetch::{self, Source},
    journal::{Journal, Meta},
    manager::{Manager, Schedule},
    ranges::{Range, RangeSet},
    sanitize,
    testserver::{self, Server},
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::atomic::Ordering,
    time::Duration,
};
fn config(s: &Server, dir: &Path, path: &str) -> Config {
    Config {
        sources: vec![Source::new(s.url(path))],
        dir: dir.into(),
        name: "file.bin".into(),
        max_conns: 4,
        skip_sidecar: true,
        ..Config::default()
    }
}
fn verify(path: &str, size: u64) {
    assert_eq!(
        fs::read(path).unwrap(),
        testserver::bytes(0, size as usize, 0)
    );
}
fn partially_download(s: &Server, dir: &Path, path: &str) -> std::path::PathBuf {
    let dl = Download::new(config(s, dir, path));
    let cancel = dl.cancel.clone();
    let live = dl.live.clone();
    let t = std::thread::spawn(move || dl.run());
    for _ in 0..1000 {
        if live.lock().unwrap().done >= 4 << 20 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    cancel.store(true, Ordering::Relaxed);
    assert_eq!(t.join().unwrap().unwrap_err().code, "paused");
    let jp = dir.join("file.bin.dlm");
    assert!(jp.exists());
    jp
}
#[test]
fn ranges_match_bitmap() {
    let mut set = RangeSet::default();
    let mut bitmap = [false; 128];
    let mut seed = 12345u64;
    for i in 0..5000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let a = (seed % 128) as usize;
        let b = ((seed >> 12) % 129) as usize;
        let (a, b) = (a.min(b), a.max(b));
        if i % 3 == 0 {
            set.remove(Range::new(a as u64, b as u64));
            bitmap[a..b].fill(false);
        } else {
            set.add(Range::new(a as u64, b as u64));
            bitmap[a..b].fill(true);
        }
        assert_eq!(set.covered(), bitmap.iter().filter(|b| **b).count() as u64);
        for (j, b) in bitmap.iter().enumerate() {
            assert_eq!(set.contains(Range::new(j as u64, j as u64 + 1)), *b);
        }
    }
}
#[test]
fn windows_names_are_safe() {
    for (name, want) in [
        ("../../evil.exe", "evil.exe"),
        (r"C:\folder\CON.txt", "_CON.txt"),
        ("NUL", "_NUL"),
        ("COM¹.txt", "_COM¹.txt"),
        ("x:ads.exe", "x_ads.exe"),
        (". ", "download"),
        ("safe\u{202e}exe.txt", "safeexe.txt"),
    ] {
        assert_eq!(sanitize::file_name(name), want);
    }
    assert!(
        sanitize::file_name(&"😀".repeat(300))
            .encode_utf16()
            .count()
            <= 240
    );
}
#[test]
fn filename_star_is_decoded() {
    let u = url::Url::parse("http://localhost/file").unwrap();
    assert_eq!(
        sanitize::response_name("attachment; filename=x; filename*=UTF-8''caf%C3%A9.zip", &u),
        "café.zip"
    );
}
#[test]
fn strict_content_range() {
    assert_eq!(fetch::content_range("bytes 2-9/10").unwrap(), (2, 9, 10));
    for text in [
        "bytes 2-10/10",
        "bytes */10",
        "items 0-1/10",
        "bytes 9-2/10",
        "bytes -1-1/10",
    ] {
        assert!(fetch::content_range(text).is_err());
    }
}
#[test]
fn hash_parser_rejects_bad_input() {
    assert!(fetch::user_hash(&"a".repeat(64)).is_ok());
    assert!(fetch::user_hash(&format!("md5:{}", "f".repeat(32))).is_ok());
    for h in ["sha256:abc", "sha1:fff", "zzzz", ""] {
        assert!(fetch::user_hash(h).is_err());
    }
}
#[test]
fn expiry_parses_signed_urls() {
    assert_eq!(
        fetch::url_expiry("https://host/file?expiration=1800000000"),
        Some(1800000000)
    );
    assert_eq!(
        fetch::url_expiry("https://host/file?X-Amz-Date=20260101T000000Z&X-Amz-Expires=60"),
        Some(1767225660)
    );
    assert_eq!(fetch::url_expiry("https://host/file?expires=10"), None);
}
#[test]
fn schedule_handles_midnight() {
    let s = Schedule {
        enabled: true,
        start: "22:00".into(),
        end: "06:00".into(),
    };
    for time in ["23:00", "01:00"] {
        assert!(s.allows(chrono::NaiveTime::parse_from_str(time, "%H:%M").unwrap()));
    }
    assert!(!s.allows(chrono::NaiveTime::parse_from_str("12:00", "%H:%M").unwrap()));
}
#[test]
fn journal_replays_go_format_and_discards_torn_tail() {
    let temp = tempfile::tempdir().unwrap();
    let p = temp.path().join("go.dlm");
    let header = r#"H {"v":1,"url":"http://localhost/file","name":"file","total":100,"bs":4194304,"ranges":true}"#;
    let mut f = fs::File::create(&p).unwrap();
    for text in [header, "R 0 50", "B 0 0123456789abcdef"] {
        writeln!(f, "{text}\t{:08x}", crc32c::crc32c(text.as_bytes())).unwrap();
    }
    let valid = f.metadata().unwrap().len();
    write!(f, "R 50 100\t00000000\nR 50").unwrap();
    drop(f);
    let j = Journal::open(&p).unwrap();
    assert_eq!(j.done.covered(), 50);
    assert_eq!(j.blocks[&0], 0x0123456789abcdef);
    assert_eq!(fs::metadata(&p).unwrap().len(), valid);
}
#[test]
fn journal_retraction_survives_reopen() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("f.dlm");
    let mut j = Journal::create(
        &p,
        Meta {
            total: 8 << 20,
            ..Meta::default()
        },
        RangeSet::default(),
    )
    .unwrap();
    j.commit(&[Range::new(0, 8 << 20)], &[(0, 1), (1, 2)])
        .unwrap();
    j.retract(Range::new(0, 4 << 20)).unwrap();
    drop(j);
    let j = Journal::open(&p).unwrap();
    assert_eq!(j.done.covered(), 4 << 20);
    assert_eq!(j.blocks.len(), 1);
}
#[test]
fn journal_discards_tail_with_torn_utf8() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.dlm");
    drop(
        Journal::create(
            &path,
            Meta {
                total: 100,
                ..Meta::default()
            },
            RangeSet::default(),
        )
        .unwrap(),
    );
    let valid = fs::metadata(&path).unwrap().len();
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(b"F hash caf\xc3\n").unwrap();
    drop(f);
    let j = Journal::open(&path).unwrap();
    assert!(j.final_record.is_empty());
    assert_eq!(fs::metadata(path).unwrap().len(), valid);
}
#[test]
fn complete_segmented_download() {
    let s = Server::start(9 << 20).unwrap();
    let t = tempfile::tempdir().unwrap();
    let r = Download::new(config(&s, t.path(), "/full.bin"))
        .run()
        .unwrap();
    verify(&r.path, s.size);
    assert!(!t.path().join("file.bin.dlm").exists());
}
#[test]
fn server_without_ranges() {
    let s = Server::start(150000).unwrap();
    let t = tempfile::tempdir().unwrap();
    let r = Download::new(config(&s, t.path(), "/no-range.bin"))
        .run()
        .unwrap();
    verify(&r.path, s.size);
}
#[test]
fn unknown_total() {
    let s = Server::start(150001).unwrap();
    let t = tempfile::tempdir().unwrap();
    let r = Download::new(config(&s, t.path(), "/unknown.bin"))
        .run()
        .unwrap();
    assert_eq!(r.size, s.size);
    verify(&r.path, s.size);
}
#[test]
fn redirected_download() {
    let s = Server::start(300000).unwrap();
    let t = tempfile::tempdir().unwrap();
    let r = Download::new(config(&s, t.path(), "/redirect.bin"))
        .run()
        .unwrap();
    verify(&r.path, s.size);
}
#[test]
fn response_200_never_appended() {
    let s = Server::start(9 << 20).unwrap();
    let t = tempfile::tempdir().unwrap();
    let e = Download::new(config(&s, t.path(), "/range-200.bin"))
        .run()
        .unwrap_err();
    assert_eq!(e.code, "remote_changed");
    assert!(!t.path().join("file.bin").exists());
}
#[test]
fn wrong_range_rejected() {
    let s = Server::start(9 << 20).unwrap();
    let t = tempfile::tempdir().unwrap();
    assert_eq!(
        Download::new(config(&s, t.path(), "/bad-range.bin"))
            .run()
            .unwrap_err()
            .code,
        "bad_content_range"
    );
}
#[test]
fn changed_etag_rejected() {
    let s = Server::start(9 << 20).unwrap();
    let t = tempfile::tempdir().unwrap();
    assert_eq!(
        Download::new(config(&s, t.path(), "/changed.bin"))
            .run()
            .unwrap_err()
            .code,
        "remote_changed"
    );
}
#[test]
fn expired_link_and_html_are_classified() {
    let s = Server::start(10000).unwrap();
    for path in ["/expired.bin", "/html"] {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(
            Download::new(config(&s, t.path(), path))
                .run()
                .unwrap_err()
                .code,
            "needs_link"
        );
    }
}
#[test]
fn no_overwrite() {
    let s = Server::start(40000).unwrap();
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("file.bin"), b"keep").unwrap();
    let r = Download::new(config(&s, t.path(), "/full.bin"))
        .run()
        .unwrap();
    assert!(r.path.ends_with("file (1).bin"));
    assert_eq!(fs::read(t.path().join("file.bin")).unwrap(), b"keep");
    verify(&r.path, s.size);
}
#[test]
fn published_hash_is_verified() {
    let s = Server::start(100000).unwrap();
    let t = tempfile::tempdir().unwrap();
    let r = Download::new(config(&s, t.path(), "/hash.bin"))
        .run()
        .unwrap();
    assert!(r.check.checked && r.check.ok);
    assert_eq!(r.sha256, testserver::hash(s.size));
}
#[test]
fn wrong_hash_blocks_final_delivery() {
    let s = Server::start(100000).unwrap();
    let t = tempfile::tempdir().unwrap();
    let mut cfg = config(&s, t.path(), "/full.bin");
    cfg.expected_hash = "0".repeat(64);
    assert_eq!(Download::new(cfg).run().unwrap_err().code, "hash_mismatch");
    assert!(!t.path().join("file.bin").exists());
    assert!(t.path().join("file.bin.dlmpart").exists());
}
#[test]
fn sidecar_hash_is_verified() {
    let s = Server::start(100000).unwrap();
    let t = tempfile::tempdir().unwrap();
    let mut cfg = config(&s, t.path(), "/full.bin");
    cfg.skip_sidecar = false;
    let r = Download::new(cfg).run().unwrap();
    assert!(r.check.checked);
    assert_eq!(r.sha256, testserver::hash(s.size));
}
#[test]
fn pause_resume_preserves_bytes() {
    let s = Server::start(20 << 20).unwrap();
    let t = tempfile::tempdir().unwrap();
    partially_download(&s, t.path(), "/slow.bin");
    let r = Download::new(config(&s, t.path(), "/slow.bin"))
        .run()
        .unwrap();
    verify(&r.path, s.size);
}
#[test]
fn corrupted_committed_block_is_redownloaded() {
    let s = Server::start(20 << 20).unwrap();
    let t = tempfile::tempdir().unwrap();
    partially_download(&s, t.path(), "/slow.bin");
    let mut f = OpenOptions::new()
        .write(true)
        .open(t.path().join("file.bin.dlmpart"))
        .unwrap();
    f.write_all(&[255; 100]).unwrap();
    f.sync_all().unwrap();
    drop(f);
    let r = Download::new(config(&s, t.path(), "/slow.bin"))
        .run()
        .unwrap();
    verify(&r.path, s.size);
}
#[test]
fn seam_protects_resume_without_validator() {
    let s = Server::start(20 << 20).unwrap();
    let t = tempfile::tempdir().unwrap();
    let jp = partially_download(&s, t.path(), "/no-validator.bin");
    let before = Journal::open(&jp).unwrap().done.covered();
    s.version.store(1, Ordering::Relaxed);
    let err = Download::new(config(&s, t.path(), "/no-validator.bin"))
        .run()
        .unwrap_err();
    assert!(matches!(
        err.code.as_str(),
        "seam_mismatch" | "not_same_file"
    ));
    assert_eq!(Journal::open(&jp).unwrap().done.covered(), before);
}
#[test]
fn relink_proves_samples() {
    let s = Server::start(20 << 20).unwrap();
    let t = tempfile::tempdir().unwrap();
    let jp = partially_download(&s, t.path(), "/slow.bin");
    let part = t.path().join("file.bin.dlmpart");
    dlm::engine::relink(&jp, &part, &Source::new(s.url("/full.bin"))).unwrap();
    let mut cfg = config(&s, t.path(), "/full.bin");
    cfg.journal_path = jp;
    let r = Download::new(cfg).run().unwrap();
    verify(&r.path, s.size);
}
#[test]
fn mirror_requires_same_etag() {
    let s = Server::start(400000).unwrap();
    let mirror = Server::start(400000).unwrap();
    mirror.version.store(1, Ordering::Relaxed);
    let t = tempfile::tempdir().unwrap();
    let mut cfg = config(&s, t.path(), "/full.bin");
    cfg.sources.push(Source::new(mirror.url("/full.bin")));
    assert_eq!(Download::new(cfg).run().unwrap_err().code, "not_same_file");
}
#[test]
fn manager_persists_and_restarts_queue() {
    let s = Server::start(100000).unwrap();
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("dlm.db");
    let m = Manager::open(&path).unwrap();
    let result=m.rpc("Add",&[serde_json::json!({"url":s.url("/full.bin"),"dir":t.path(),"name":"managed.bin","cookies":"session=test"})]).unwrap();
    let id = result["id"].as_i64().unwrap();
    m.start();
    for _ in 0..500 {
        if m.list()[0]["state"] == "done" {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(m.list()[0]["state"], "done");
    m.shutdown();
    drop(m);
    let m = Manager::open(&path).unwrap();
    assert_eq!(m.list()[0]["id"], id);
    assert_eq!(m.list()[0]["state"], "done");
    let db = rusqlite::Connection::open(&path).unwrap();
    let enc: Vec<u8> = db
        .query_row("SELECT cookies FROM tasks LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert!(!enc.windows(12).any(|w| w == b"session=test"));
    assert_eq!(dlm::system::crypt(&enc, true).unwrap(), b"session=test");
    assert!(path.with_extension("pre-rust.db").exists());
}
#[test]
fn chromium_real_schema_fixture() {
    let t = tempfile::tempdir().unwrap();
    let partial = t.path().join("browser.crdownload");
    fs::write(&partial, testserver::bytes(0, 10000, 0)).unwrap();
    let fixture = include_str!("fixtures/history_v70.sql");
    let db = rusqlite::Connection::open(t.path().join("History")).unwrap();
    db.execute_batch(fixture).unwrap();
    db.execute("INSERT INTO downloads VALUES(1,'fixture',?1,?2,13300000000000000,8000,20000,4,0,0,X'',0,0,0,0,'','','','','','GET','','','','\"v0\"','','application/octet-stream','application/octet-stream')",rusqlite::params![partial.display().to_string(),t.path().join("browser.bin").display().to_string()]).unwrap();
    db.execute(
        "INSERT INTO downloads_url_chains VALUES(1,0,'http://localhost/file')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO downloads_slices VALUES(1,0,7000,0)", [])
        .unwrap();
    drop(db);
    let items = dlm::chrome::list_history(t.path(), "Chrome", "Default").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].resume_at, 7000);
    assert_eq!(items[0].file_size, 10000);
    assert!(items[0].sliced);
    assert!(items[0].problem.is_empty());
}
#[test]
fn no_range_restart_retains_expected_hash() {
    let s = Server::start(200000).unwrap();
    let t = tempfile::tempdir().unwrap();
    let mut cfg = config(&s, t.path(), "/no-range.bin");
    cfg.expected_hash = "0".repeat(64);
    assert_eq!(Download::new(cfg).run().unwrap_err().code, "hash_mismatch");
    let cfg = config(&s, t.path(), "/no-range.bin");
    assert_eq!(Download::new(cfg).run().unwrap_err().code, "hash_mismatch");
    assert!(!t.path().join("file.bin").exists());
}
#[test]
fn md5_expected_hash_is_verified() {
    use md5::{Digest, Md5};
    let s = Server::start(200000).unwrap();
    let t = tempfile::tempdir().unwrap();
    let mut cfg = config(&s, t.path(), "/full.bin");
    cfg.expected_hash = format!(
        "md5:{}",
        hex::encode(Md5::digest(testserver::bytes(0, s.size as usize, 0)))
    );
    assert!(Download::new(cfg).run().unwrap().check.ok);
}
#[test]
fn interrupted_body_is_retried() {
    let s = Server::start(9 << 20).unwrap();
    let t = tempfile::tempdir().unwrap();
    let r = Download::new(config(&s, t.path(), "/drop.bin"))
        .run()
        .unwrap();
    verify(&r.path, s.size);
}
#[test]
fn throttled_probe_is_retried() {
    let s = Server::start(200000).unwrap();
    let t = tempfile::tempdir().unwrap();
    let r = Download::new(config(&s, t.path(), "/retry.bin"))
        .run()
        .unwrap();
    verify(&r.path, s.size);
}
#[test]
fn signed_response_keeps_streaming_after_expiry() {
    let s = Server::start(40 << 20).unwrap();
    let t = tempfile::tempdir().unwrap();
    let mut cfg = config(
        &s,
        t.path(),
        &format!("/slow.bin?expires={}", chrono::Utc::now().timestamp() + 2),
    );
    cfg.max_conns = 2;
    let r = Download::new(cfg).run().unwrap();
    verify(&r.path, s.size);
}
#[test]
fn chrome_import_adopts_only_matching_bytes() {
    let s = Server::start(200000).unwrap();
    let t = tempfile::tempdir().unwrap();
    let partial = t.path().join("browser.crdownload");
    fs::write(&partial, testserver::bytes(0, 100000, 0)).unwrap();
    let item = dlm::chrome::Item {
        url: s.url("/full.bin"),
        current_path: partial.display().to_string(),
        target_path: t.path().join("browser.bin").display().to_string(),
        total: s.size as i64,
        prefix: 100000,
        resume_at: 100000,
        etag: "\"v0\"".into(),
        ..Default::default()
    };
    s.version.store(1, Ordering::Relaxed);
    assert!(dlm::chrome::prepare(&item).is_err());
    assert!(partial.exists());
    s.version.store(0, Ordering::Relaxed);
    let mut cfg = dlm::chrome::prepare(&item).unwrap();
    cfg.skip_sidecar = true;
    assert!(!partial.exists());
    let result = Download::new(cfg).run().unwrap();
    verify(&result.path, s.size);
}
