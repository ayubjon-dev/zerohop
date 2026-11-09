//! Two complete engines talking over the loopback interface, driven the same
//! way the interface drives them.
#![cfg(feature = "dev-loopback")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    missing_docs
)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use zerohop_app::{Config, Engine, KeySealing};
use zeroize::Zeroizing;

const PATIENCE: Duration = Duration::from_secs(20);
const ERROR_LOCK_VIOLATION: i32 = 33;

struct Side {
    engine: Engine,
    log: Arc<Mutex<Vec<Value>>>,
    next: AtomicU64,
    folder: PathBuf,
    _dirs: Option<(tempfile::TempDir, tempfile::TempDir)>,
}

impl Side {
    fn new(name: &str) -> Self {
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        Self::with_dirs(name, state, files)
    }

    fn with_dirs(name: &str, state: tempfile::TempDir, files: tempfile::TempDir) -> Self {
        let mut side = Self::open(name, state.path(), files.path());
        side._dirs = Some((state, files));
        side
    }

    /// A device whose folders outlive it, so it can be restarted.
    fn open(name: &str, state: &Path, files: &Path) -> Self {
        Self::open_sealed(name, state, files, &KeySealing::default())
    }

    fn open_sealed(name: &str, state: &Path, files: &Path, sealing: &KeySealing) -> Self {
        let side = Self::start(state, files, sealing);
        side.ok("set_name", json!({ "name": name }));
        side.ok("bootstrap", json!({ "allow_loopback": true }));
        side
    }

    fn start(state: &Path, files: &Path, sealing: &KeySealing) -> Self {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink_log = log.clone();
        let engine = Engine::start(
            Config {
                state_dir: state.to_path_buf(),
                default_folder: files.to_path_buf(),
                sealing: sealing.clone(),
            },
            Arc::new(move |text: String| {
                sink_log
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str::<Value>(&text).unwrap());
            }),
        )
        .unwrap();
        Self {
            engine,
            log,
            next: AtomicU64::new(1),
            folder: files.to_path_buf(),
            _dirs: None,
        }
    }

    fn call(&self, method: &str, params: Value) -> Value {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let text = json!({ "id": id, "method": method, "params": params }).to_string();
        self.engine.request(&text);
        self.wait(&format!("reply to {method}"), |log| {
            log.iter().find(|m| m["reply"] == json!(id)).cloned()
        })
    }

    fn ok(&self, method: &str, params: Value) -> Value {
        let reply = self.call(method, params);
        assert!(reply["error"].is_null(), "{method} failed: {reply}");
        reply["ok"].clone()
    }

    fn wait<T>(&self, what: &str, mut find: impl FnMut(&[Value]) -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(found) = find(&self.log.lock().unwrap()) {
                return found;
            }
            if start.elapsed() >= PATIENCE {
                panic!("timed out waiting for {what}; {}", self.latest_messages());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits until part of an incoming file has reached this side's disk.
    ///
    /// Progress reports come only a few times a second, and a fast machine
    /// can finish a whole test file between two of them.
    fn wait_for_data(&self) {
        let unfinished = self.folder.join(".zerohop-tmp");
        self.wait("data to reach the disk", |_| {
            let transfers = std::fs::read_dir(&unfinished).ok()?;
            transfers
                .flatten()
                .any(|transfer| holds_bytes(&transfer.path().join("files")))
                .then_some(())
        });
    }

    /// What this side said last, so a timeout on a machine nobody can log in
    /// to still explains itself.
    fn latest_messages(&self) -> String {
        let log = self.log.lock().unwrap();
        let latest: Vec<String> = log
            .iter()
            .rev()
            .filter(|m| m["event"] != "snapshot")
            .take(5)
            .map(ToString::to_string)
            .collect();
        format!("latest messages, newest first: {}", latest.join(" | "))
    }

    fn snapshot(&self) -> Value {
        self.log
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|m| m["event"] == "snapshot")
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn wait_snapshot(&self, what: &str, test: impl Fn(&Value) -> bool) -> Value {
        self.wait(what, |log| {
            log.iter()
                .rev()
                .find(|m| m["event"] == "snapshot")
                .filter(|s| test(s))
                .cloned()
        })
    }

    fn wait_event(&self, what: &str, test: impl Fn(&Value) -> bool) -> Value {
        self.wait(what, |log| log.iter().rev().find(|m| test(m)).cloned())
    }

    fn clear(&self) {
        self.log
            .lock()
            .unwrap()
            .retain(|m| m["event"] == "snapshot");
    }

    /// Reports this side's announcement to the other, as Bonjour would.
    fn announce_to(&self, other: &Side) {
        let snapshot = self.snapshot();
        let advert = &snapshot["advert"];
        assert!(!advert.is_null(), "no announcement: {snapshot}");
        other.ok(
            "discovery_seen",
            json!({
                "instance": advert["instance"],
                "addresses": ["127.0.0.1"],
                "port": advert["port"],
                "tag": advert["tag"],
            }),
        );
    }
}

fn pair(host: &Side, joiner: &Side) {
    host.ok("pair_host", Value::Null);
    let waiting = host.wait_snapshot("the code", |s| s["pairing"]["code"].is_string());
    let pairing = &waiting["pairing"];
    joiner.ok(
        "pair_join",
        json!({
            "address": format!("127.0.0.1:{}", pairing["port"].as_u64().unwrap()),
            "code": pairing["code"],
        }),
    );
    let a = host.wait_snapshot("host compare", |s| s["pairing"]["phase"] == "compare");
    let b = joiner.wait_snapshot("joiner compare", |s| s["pairing"]["phase"] == "compare");
    assert_eq!(a["pairing"]["comparison"], b["pairing"]["comparison"]);
    assert!(a["pairing"]["comparison"].as_str().unwrap().len() == 6);
    host.ok("pair_answer", json!({ "accept": true }));
    joiner.ok("pair_answer", json!({ "accept": true }));
    host.wait_event("host paired", |m| {
        m["event"] == "pairing_ended" && m["outcome"] == "paired"
    });
    joiner.wait_event("joiner paired", |m| {
        m["event"] == "pairing_ended" && m["outcome"] == "paired"
    });
    host.wait_snapshot("host device list", |s| {
        !s["devices"].as_array().unwrap().is_empty()
    });
    joiner.wait_snapshot("joiner device list", |s| {
        !s["devices"].as_array().unwrap().is_empty()
    });
    // Announcements are what Bonjour would deliver in the app.
    joiner.wait_snapshot("joiner's announcement", |s| !s["advert"].is_null());
    host.announce_to(joiner);
    joiner.announce_to(host);
}

fn device_of(side: &Side) -> String {
    side.snapshot()["devices"][0]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn write(dir: &Path, name: &str, data: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    path.display().to_string()
}

fn ended(side: &Side, outcome: &str) -> Value {
    side.wait_event(&format!("a transfer ending with {outcome}"), |m| {
        m["event"] == "transfer_ended" && m["outcome"] == outcome
    })
}

fn paired_sides() -> (Side, Side) {
    let (a, b) = (Side::new("Mac"), Side::new("Phone"));
    pair(&a, &b);
    (a, b)
}

#[test]
fn a_new_device_has_no_circle_until_it_pairs() {
    let side = Side::new("Mac");
    side.wait_snapshot("the first state", |s| {
        s["me"].is_null() && s["name"] == "Mac"
    });
    assert!(side.snapshot()["advert"].is_null());
}

#[test]
fn pairing_shows_each_device_the_same_number_and_lists_the_other() {
    let (a, b) = paired_sides();
    let host_view = a.snapshot();
    assert_eq!(host_view["devices"][0]["name"], "Phone");
    let joiner_view = b.snapshot();
    assert_eq!(joiner_view["devices"][0]["name"], "Mac");
    assert!(host_view["pairing"].is_null());
}

#[test]
fn refusing_the_comparison_adds_nobody() {
    let (a, b) = (Side::new("Mac"), Side::new("Phone"));
    a.ok("pair_host", Value::Null);
    let waiting = a.wait_snapshot("the code", |s| s["pairing"]["code"].is_string());
    b.ok(
        "pair_join",
        json!({
            "address": format!("127.0.0.1:{}", waiting["pairing"]["port"].as_u64().unwrap()),
            "code": waiting["pairing"]["code"],
        }),
    );
    a.wait_snapshot("compare", |s| s["pairing"]["phase"] == "compare");
    b.wait_snapshot("compare", |s| s["pairing"]["phase"] == "compare");
    a.ok("pair_answer", json!({ "accept": false }));
    b.ok("pair_answer", json!({ "accept": true }));
    a.wait_event("refusal", |m| {
        m["event"] == "pairing_ended" && m["outcome"] == "declined"
    });
    assert!(a.snapshot()["devices"].as_array().unwrap().is_empty());
    b.wait_snapshot("nothing joined", |s| {
        s["me"].is_null() && s["pairing"].is_null()
    });
}

#[test]
fn a_wrong_code_is_reported_and_the_code_stops_working() {
    let (a, b) = (Side::new("Mac"), Side::new("Phone"));
    a.ok("pair_host", Value::Null);
    let waiting = a.wait_snapshot("the code", |s| s["pairing"]["code"].is_string());
    let code = waiting["pairing"]["code"].as_str().unwrap();
    let wrong = if code == "000000" { "000001" } else { "000000" };
    b.ok(
        "pair_join",
        json!({
            "address": format!("127.0.0.1:{}", waiting["pairing"]["port"].as_u64().unwrap()),
            "code": wrong,
        }),
    );
    b.wait_event("joiner told", |m| {
        m["event"] == "pairing_ended" && m["outcome"] != "paired"
    });
    a.wait_event("host told", |m| {
        m["event"] == "pairing_ended" && m["outcome"] != "paired"
    });
    assert!(a.snapshot()["devices"].as_array().unwrap().is_empty());
}

#[test]
fn devices_recognize_each_other_after_their_first_session() {
    let (a, b) = paired_sides();
    a.wait_snapshot("the other device nearby", |s| {
        s["devices"][0]["nearby"] == true
    });
    b.wait_snapshot("the other device nearby", |s| {
        s["devices"][0]["nearby"] == true
    });
}

#[test]
fn a_file_goes_across_and_is_remembered_on_both_sides() {
    let (a, b) = paired_sides();
    let source = tempfile::tempdir().unwrap();
    let path = write(source.path(), "report.txt", b"quarterly numbers");

    b.ok("send", json!({ "device": device_of(&b), "paths": [path] }));

    ended(&b, "done");
    ended(&a, "done");
    assert_eq!(
        std::fs::read(a.folder.join("report.txt")).unwrap(),
        b"quarterly numbers"
    );
    let sent = b.wait_snapshot("history", |s| s["history"][0]["outcome"] == "done");
    assert_eq!(sent["history"][0]["direction"], "sent");
    assert_eq!(sent["history"][0]["name"], "report.txt");
    assert_eq!(sent["history"][0]["peer_name"], "Mac");
    let received = a.wait_snapshot("history", |s| s["history"][0]["outcome"] == "done");
    assert_eq!(received["history"][0]["direction"], "received");
}

#[test]
fn the_owner_is_asked_and_can_accept() {
    let (a, b) = paired_sides();
    a.ok(
        "set_device_mode",
        json!({ "device": device_of(&a), "mode": "ask" }),
    );
    let source = tempfile::tempdir().unwrap();
    let path = write(source.path(), "photo.png", b"pixels");

    b.ok("send", json!({ "device": device_of(&b), "paths": [path] }));

    let asked = a.wait_snapshot("the question", |s| {
        !s["offers"].as_array().unwrap().is_empty()
    });
    let offer = &asked["offers"][0];
    assert_eq!(offer["peer_name"], "Phone");
    assert_eq!(offer["names"][0], "photo.png");
    assert_eq!(offer["files"], 1);
    // A transfer waiting for an answer is reported, with nothing moved yet.
    let report = b.wait_event("a progress report", |m| m["event"] == "progress");
    assert_eq!(report["items"][0]["done"], 0);
    a.ok(
        "answer_offer",
        json!({ "offer": offer["id"], "accept": true }),
    );
    ended(&b, "done");
    assert_eq!(
        std::fs::read(a.folder.join("photo.png")).unwrap(),
        b"pixels"
    );
}

#[test]
fn the_owner_can_decline_and_the_sender_is_told() {
    let (a, b) = paired_sides();
    a.ok("set_settings", json!({ "default_mode": "ask" }));
    let source = tempfile::tempdir().unwrap();
    let path = write(source.path(), "photo.png", b"pixels");

    b.ok("send", json!({ "device": device_of(&b), "paths": [path] }));
    let asked = a.wait_snapshot("the question", |s| {
        !s["offers"].as_array().unwrap().is_empty()
    });
    a.ok(
        "answer_offer",
        json!({ "offer": asked["offers"][0]["id"], "accept": false }),
    );

    ended(&b, "declined");
    assert!(!a.folder.join("photo.png").exists());
    let history = a.wait_snapshot("history", |s| s["history"][0]["outcome"] == "declined");
    assert_eq!(history["history"][0]["name"], "photo.png");
}

#[test]
fn a_question_disappears_when_the_sender_gives_up() {
    let (a, b) = paired_sides();
    a.ok("set_settings", json!({ "default_mode": "ask" }));
    let source = tempfile::tempdir().unwrap();
    let path = write(source.path(), "photo.png", b"pixels");
    let sent = b.ok("send", json!({ "device": device_of(&b), "paths": [path] }));

    a.wait_snapshot("the question", |s| {
        !s["offers"].as_array().unwrap().is_empty()
    });
    b.ok("cancel", json!({ "transfer": sent["transfer"] }));

    ended(&b, "cancelled");
    a.wait_snapshot("the question to go", |s| {
        s["offers"].as_array().unwrap().is_empty()
    });
}

#[test]
fn a_program_is_flagged_in_the_question() {
    let (a, b) = paired_sides();
    let source = tempfile::tempdir().unwrap();
    let path = write(source.path(), "setup.exe", b"MZ program");

    b.ok("send", json!({ "device": device_of(&b), "paths": [path] }));

    let asked = a.wait_snapshot("the question", |s| {
        !s["offers"].as_array().unwrap().is_empty()
    });
    assert_eq!(asked["offers"][0]["has_program"], true);
    assert_eq!(asked["offers"][0]["first_program"], "setup.exe");
    a.ok(
        "answer_offer",
        json!({ "offer": asked["offers"][0]["id"], "accept": false }),
    );
    ended(&b, "declined");
}

#[test]
fn a_disguised_program_waits_for_the_owner() {
    let (a, b) = paired_sides();
    let source = tempfile::tempdir().unwrap();
    let path = write(source.path(), "cat.jpg", b"MZ-disguised-program");

    b.ok("send", json!({ "device": device_of(&b), "paths": [path] }));

    let done = ended(&b, "done");
    assert_eq!(done["held"], 1);
    assert!(!a.folder.join("cat.jpg").exists());
    let held = a.wait_snapshot("the held list", |s| {
        !s["held"].as_array().unwrap().is_empty()
    });
    assert_eq!(held["held"][0]["name"], "cat.jpg");
    assert_eq!(held["history"][0]["held"], 1);

    a.ok(
        "release_held",
        json!({ "transfer": held["held"][0]["transfer"], "name": held["held"][0]["raw_name"] }),
    );
    a.wait_snapshot("the list to empty", |s| {
        s["held"].as_array().unwrap().is_empty()
    });
    assert_eq!(
        std::fs::read(a.folder.join("cat.jpg")).unwrap(),
        b"MZ-disguised-program"
    );
}

#[test]
fn cancelling_a_transfer_leaves_nothing_behind() {
    let (a, b) = paired_sides();
    let source = tempfile::tempdir().unwrap();
    let path = write(source.path(), "big.bin", &vec![1u8; 300 * 1024 * 1024]);

    let sent = b.ok("send", json!({ "device": device_of(&b), "paths": [path] }));
    a.wait_for_data();
    b.ok("cancel", json!({ "transfer": sent["transfer"] }));

    ended(&b, "cancelled");
    ended(&a, "cancelled_by_peer");
    assert!(!a.folder.join("big.bin").exists());
    assert!(b.snapshot()["unfinished"].as_array().unwrap().is_empty());
}

#[test]
fn a_removed_device_is_gone_and_can_no_longer_deliver() {
    let (a, b) = paired_sides();
    a.ok("remove_device", json!({ "device": device_of(&a) }));
    a.wait_snapshot("the list to empty", |s| {
        s["devices"].as_array().unwrap().is_empty()
    });
    let source = tempfile::tempdir().unwrap();
    let path = write(source.path(), "late.txt", b"too late");

    b.ok("send", json!({ "device": device_of(&b), "paths": [path] }));

    let refused = b.wait_event("the refusal", |m| {
        m["event"] == "transfer_ended" && m["outcome"] != "done"
    });
    assert_eq!(refused["outcome"], "removed");
    assert!(!a.folder.join("late.txt").exists());
    b.wait_snapshot("the removal to be noticed", |s| s["me"]["removed"] == true);
}

#[test]
fn a_device_added_elsewhere_waits_for_approval_when_asked_to() {
    let (a, b) = paired_sides();
    a.ok("set_settings", json!({ "confirm_new": true }));
    let c = Side::new("Tablet");
    pair(&b, &c);
    // `b` carries the news of `c` to `a`.
    b.announce_to(&a);
    a.announce_to(&b);
    let seen = a.wait_snapshot("the new device", |s| {
        s["devices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["name"] == "Tablet")
    });
    let tablet = seen["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "Tablet")
        .unwrap()
        .clone();
    assert_eq!(tablet["awaiting_confirmation"], true);
    let tablet_id = tablet["id"].clone();

    a.ok(
        "confirm_device",
        json!({ "device": tablet_id, "accept": true }),
    );
    let after = a.wait_snapshot("approval", |s| {
        s["devices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["name"] == "Tablet" && d["awaiting_confirmation"] == false)
    });
    assert_eq!(after["devices"].as_array().unwrap().len(), 2);
}

#[test]
fn settings_and_history_survive_a_restart() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let original = state.path().to_path_buf();
    let later = tempfile::tempdir().unwrap();
    let side = Side::with_dirs("Mac", state, files);
    side.ok(
        "set_settings",
        json!({ "visible": false, "ui": { "palette": "mint" } }),
    );
    // The directory goes away with `side`, so keep what a restart would find.
    std::fs::copy(
        original.join("settings.json"),
        later.path().join("settings.json"),
    )
    .unwrap();
    drop(side);

    let log: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = log.clone();
    let engine = Engine::start(
        Config {
            state_dir: later.path().to_path_buf(),
            default_folder: std::env::temp_dir(),
            sealing: KeySealing::default(),
        },
        Arc::new(move |text: String| {
            sink.lock()
                .unwrap()
                .push(serde_json::from_str::<Value>(&text).unwrap());
        }),
    )
    .unwrap();
    engine.request(&json!({ "id": 1, "method": "load_settings" }).to_string());
    let start = Instant::now();
    loop {
        if let Some(reply) = log.lock().unwrap().iter().find(|m| m["reply"] == 1) {
            assert_eq!(reply["ok"]["visible"], false);
            assert_eq!(reply["ok"]["ui"]["palette"], "mint");
            assert_eq!(reply["ok"]["name"], "Mac");
            break;
        }
        assert!(start.elapsed() < PATIENCE);
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn an_interrupted_transfer_can_be_continued_from_the_interface() {
    let a_state = tempfile::tempdir().unwrap();
    let a_files = tempfile::tempdir().unwrap();
    let b = Side::new("Phone");
    let a = Side::open("Mac", a_state.path(), a_files.path());
    pair(&a, &b);
    let source = tempfile::tempdir().unwrap();
    let data: Vec<u8> = (0..120_000_000u32).map(|i| (i % 251) as u8).collect();
    let path = write(source.path(), "big.bin", &data);

    let sent = b.ok("send", json!({ "device": device_of(&b), "paths": [path] }));
    a.wait_for_data();
    // The receiving device goes away part way through.
    drop(a);
    let end = ended(&b, "connection_lost");
    assert_eq!(end["can_resume"], true);
    let waiting = b.wait_snapshot("the unfinished list", |s| {
        !s["unfinished"].as_array().unwrap().is_empty()
    });
    assert_eq!(waiting["unfinished"][0]["id"], sent["transfer"]);

    let a = Side::open("Mac", a_state.path(), a_files.path());
    a.wait_snapshot("the device to come back", |s| !s["advert"].is_null());
    a.announce_to(&b);
    b.announce_to(&a);
    b.clear();
    b.ok("resume", json!({ "transfer": sent["transfer"] }));

    ended(&b, "done");
    assert_eq!(std::fs::read(a_files.path().join("big.bin")).unwrap(), data);
    b.wait_snapshot("the list to empty", |s| {
        s["unfinished"].as_array().unwrap().is_empty()
    });
}

#[test]
fn a_transfer_that_cannot_continue_says_so_and_is_forgotten() {
    let a_state = tempfile::tempdir().unwrap();
    let a_files = tempfile::tempdir().unwrap();
    let b = Side::new("Phone");
    let a = Side::open("Mac", a_state.path(), a_files.path());
    pair(&a, &b);
    let source = tempfile::tempdir().unwrap();
    let path = write(source.path(), "big.bin", &vec![3u8; 120 * 1024 * 1024]);

    let sent = b.ok("send", json!({ "device": device_of(&b), "paths": [path] }));
    a.wait_for_data();
    drop(a);
    ended(&b, "connection_lost");
    b.wait_snapshot("the unfinished list", |s| {
        !s["unfinished"].as_array().unwrap().is_empty()
    });
    // What the receiver had kept is gone.
    std::fs::remove_dir_all(a_files.path().join(".zerohop-tmp")).unwrap();
    let a = Side::open("Mac", a_state.path(), a_files.path());
    a.wait_snapshot("the device to come back", |s| !s["advert"].is_null());
    a.announce_to(&b);
    b.clear();
    b.ok("resume", json!({ "transfer": sent["transfer"] }));

    ended(&b, "cannot_resume");
    b.wait_snapshot("the list to empty", |s| {
        s["unfinished"].as_array().unwrap().is_empty()
    });
}

#[test]
fn nothing_is_sent_to_a_device_that_cannot_be_found() {
    let (a, b) = paired_sides();
    drop(a);
    std::thread::sleep(Duration::from_millis(100));
    let source = tempfile::tempdir().unwrap();
    let path = write(source.path(), "x.txt", b"x");
    b.clear();
    b.ok("send", json!({ "device": device_of(&b), "paths": [path] }));
    let end = b.wait_event("a failure", |m| m["event"] == "transfer_ended");
    assert_ne!(end["outcome"], "done");
    assert!(b.snapshot()["transfers"].as_array().unwrap().is_empty());
}

// ---- The shared clipboard ----

fn copy_text(side: &Side, text: &str) -> Value {
    side.ok("clip_changed", json!({ "format": "text", "text": text }))
}

fn turn_on(side: &Side) {
    side.ok("set_clipboard", json!({ "enabled": true }));
}

fn applied(side: &Side) -> Value {
    side.wait_event("a clip", |m| m["event"] == "clipboard_apply")
}

fn nothing_applied(side: &Side) -> bool {
    std::thread::sleep(Duration::from_millis(400));
    !side
        .log
        .lock()
        .unwrap()
        .iter()
        .any(|m| m["event"] == "clipboard_apply")
}

fn set_rule(side: &Side, rule: &str, value: bool) {
    let mut params = json!({ "device": device_of(side) });
    params[rule] = json!(value);
    side.ok("set_clipboard_device", params);
}

/// Whether any file under `dir` has bytes in it. The receiver is working here,
/// so anything may vanish while this looks.
fn holds_bytes(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        // Not `entry.metadata()`: on Windows a directory listing can show a
        // file that is still open as empty.
        std::fs::metadata(&path).is_ok_and(|m| {
            if m.is_dir() {
                holds_bytes(&path)
            } else {
                m.len() > 0
            }
        })
    })
}

fn files_under(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(&path, found);
        } else {
            found.push(path);
        }
    }
}

#[test]
fn the_clipboard_is_off_until_it_is_turned_on() {
    let (a, b) = paired_sides();
    assert_eq!(a.snapshot()["settings"]["clipboard"], false);
    assert_eq!(copy_text(&a, "secret")["state"], "off");
    assert!(nothing_applied(&b));
}

#[test]
fn a_device_that_has_not_turned_it_on_takes_nothing() {
    let (a, b) = paired_sides();
    turn_on(&a);
    assert_eq!(copy_text(&a, "secret")["state"], "sent");
    assert!(nothing_applied(&b));
}

#[test]
fn copied_text_reaches_the_other_device() {
    let (a, b) = paired_sides();
    turn_on(&a);
    turn_on(&b);
    let copied = "Привет, \"мир\"\n\u{1F600} https://example.com";
    let reply = copy_text(&a, copied);
    assert_eq!(reply["state"], "sent");
    assert_eq!(reply["delivered"], 1);
    let event = applied(&b);
    assert_eq!(event["format"], "text");
    assert_eq!(event["text"], copied);
    assert_eq!(event["from"], "Mac");
    assert!(event["ttl_ms"].as_u64().unwrap() <= 120_000);
    assert!(event["ttl_ms"].as_u64().unwrap() > 100_000);
}

#[test]
fn a_copied_picture_arrives_byte_for_byte() {
    use data_encoding::BASE64;
    let (a, b) = paired_sides();
    turn_on(&a);
    turn_on(&b);
    let image: Vec<u8> = (0..900_000u32).map(|n| (n % 253) as u8).collect();
    let encoded = BASE64.encode(&image);
    let reply = a.ok(
        "clip_changed",
        json!({ "format": "png", "data": encoded.clone() }),
    );
    assert_eq!(reply["state"], "sent");
    let event = applied(&b);
    assert_eq!(event["format"], "png");
    assert_eq!(event["data"], encoded);
    assert!(event["text"].is_null());
}

#[test]
fn a_device_can_be_left_out_of_sending() {
    let (a, b) = paired_sides();
    turn_on(&a);
    turn_on(&b);
    set_rule(&a, "send", false);
    assert_eq!(copy_text(&a, "secret")["state"], "no_devices");
    assert!(nothing_applied(&b));
}

#[test]
fn the_receiver_decides_for_itself() {
    let (a, b) = paired_sides();
    turn_on(&a);
    turn_on(&b);
    set_rule(&b, "receive", false);
    assert_eq!(copy_text(&a, "secret")["state"], "sent");
    assert!(nothing_applied(&b));
}

#[test]
fn text_can_be_refused_while_pictures_are_taken() {
    use data_encoding::BASE64;
    let (a, b) = paired_sides();
    turn_on(&a);
    turn_on(&b);
    set_rule(&b, "text", false);
    copy_text(&a, "secret");
    assert!(nothing_applied(&b));
    a.ok(
        "clip_changed",
        json!({ "format": "png", "data": BASE64.encode(b"picture") }),
    );
    assert_eq!(applied(&b)["format"], "png");
}

#[test]
fn pictures_can_be_refused_while_text_is_taken() {
    use data_encoding::BASE64;
    let (a, b) = paired_sides();
    turn_on(&a);
    turn_on(&b);
    set_rule(&b, "images", false);
    a.ok(
        "clip_changed",
        json!({ "format": "jpeg", "data": BASE64.encode(b"picture") }),
    );
    assert!(nothing_applied(&b));
    copy_text(&a, "words");
    assert_eq!(applied(&b)["text"], "words");
}

#[test]
fn what_arrived_is_not_sent_back() {
    let (a, b) = paired_sides();
    turn_on(&a);
    turn_on(&b);
    copy_text(&a, "travelling");
    assert_eq!(applied(&b)["text"], "travelling");
    assert_eq!(copy_text(&b, "travelling")["state"], "same");
    assert!(nothing_applied(&a));
    assert_eq!(copy_text(&b, "something else")["state"], "sent");
    assert_eq!(applied(&a)["text"], "something else");
}

#[test]
fn a_copy_with_nothing_paired_has_nobody_to_go_to() {
    let side = Side::new("Mac");
    turn_on(&side);
    assert_eq!(copy_text(&side, "alone")["state"], "no_devices");
}

#[test]
fn clips_nobody_could_use_are_stopped_early() {
    let (a, _b) = paired_sides();
    turn_on(&a);
    assert_eq!(copy_text(&a, "")["state"], "empty");
    let long = "x".repeat(8 * 1024 * 1024 + 1);
    assert_eq!(copy_text(&a, &long)["state"], "too_large");
    let huge = "A".repeat(40 * 1024 * 1024);
    let reply = a.ok("clip_changed", json!({ "format": "png", "data": huge }));
    assert_eq!(reply["state"], "too_large");
    let reply = a.call(
        "clip_changed",
        json!({ "format": "png", "data": "not base64!" }),
    );
    assert_eq!(reply["error"]["kind"], "bad_clip");
    let reply = a.call("clip_changed", json!({ "format": "exe", "text": "x" }));
    assert_eq!(reply["error"]["kind"], "bad_clip");
}

#[test]
fn clipboard_rules_survive_a_restart() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let peer = "ab".repeat(32);
    {
        let side = Side::open("Mac", state.path(), files.path());
        turn_on(&side);
        side.ok(
            "set_clipboard_device",
            json!({ "device": peer, "images": false }),
        );
    }
    std::thread::sleep(Duration::from_millis(200));
    let side = Side::open("Mac", state.path(), files.path());
    side.wait_snapshot("the saved switch", |s| s["settings"]["clipboard"] == true);
    let saved = side.ok("load_settings", Value::Null);
    assert_eq!(saved["clipboard_devices"][&peer]["images"], false);
    assert_eq!(saved["clipboard_devices"][&peer]["text"], true);
}

#[test]
fn a_bad_device_id_is_refused() {
    let side = Side::new("Mac");
    let reply = side.call(
        "set_clipboard_device",
        json!({ "device": "nonsense", "send": false }),
    );
    assert!(!reply["error"].is_null());
}

#[test]
fn copied_content_never_reaches_the_disk_or_the_snapshots() {
    let marker = "ZH-CLIP-MARKER-7f3a91";
    let (a, b) = paired_sides();
    turn_on(&a);
    turn_on(&b);
    copy_text(&a, marker);
    assert_eq!(applied(&b)["text"], marker);
    copy_text(&b, &format!("{marker}-back"));
    applied(&a);
    a.ok("clear_history", Value::Null);
    // Long enough for anything queued to have been saved.
    std::thread::sleep(Duration::from_millis(600));
    for side in [&a, &b] {
        let dirs = side._dirs.as_ref().unwrap();
        let mut files = Vec::new();
        files_under(dirs.0.path(), &mut files);
        files_under(dirs.1.path(), &mut files);
        assert!(!files.is_empty());
        for file in files {
            let bytes = match std::fs::read(&file) {
                Ok(bytes) => bytes,
                // Windows refuses to read a file while it is locked. Only
                // empty files are used as locks, so there is nothing to find.
                Err(e) if cfg!(windows) && e.raw_os_error() == Some(ERROR_LOCK_VIOLATION) => {
                    let size = std::fs::metadata(&file).unwrap().len();
                    assert_eq!(size, 0, "locked file with content: {}", file.display());
                    continue;
                }
                Err(e) => panic!("cannot read {}: {e}", file.display()),
            };
            assert!(
                !bytes.windows(marker.len()).any(|w| w == marker.as_bytes()),
                "copied content found in {}",
                file.display()
            );
        }
        for message in side.log.lock().unwrap().iter() {
            if message["event"] != "clipboard_apply" {
                assert!(!message.to_string().contains(marker), "{message}");
            }
        }
        assert!(!side.snapshot()["history"].to_string().contains(marker));
    }
}

// ---- Protecting the device key ----

fn sealing(byte: u8) -> KeySealing {
    KeySealing::with_secret(Zeroizing::new([byte; 32]))
}

fn device_keys(state: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    files_under(state, &mut files);
    files.retain(|f| f.file_name().is_some_and(|n| n == "device.key"));
    files
}

/// Creates a circle on this side and returns this device's id.
fn make_circle(side: &Side) -> String {
    side.ok("pair_host", Value::Null);
    side.wait_snapshot("the code", |s| s["pairing"]["code"].is_string());
    side.ok("pair_cancel", Value::Null);
    let view = side.wait_snapshot("the identity", |s| s["me"]["id"].is_string());
    view["me"]["id"].as_str().unwrap().to_string()
}

#[test]
fn a_new_device_key_is_sealed_and_the_device_survives_a_restart() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let id = {
        let side = Side::open_sealed("Mac", state.path(), files.path(), &sealing(7));
        make_circle(&side)
    };
    std::thread::sleep(Duration::from_millis(300));
    let keys = device_keys(state.path());
    assert_eq!(keys.len(), 1);
    assert!(std::fs::read(&keys[0]).unwrap().starts_with(b"ZHSEAL1"));
    let again = Side::open_sealed("Mac", state.path(), files.path(), &sealing(7));
    let view = again.wait_snapshot("the same device", |s| s["me"]["id"].is_string());
    assert_eq!(view["me"]["id"].as_str().unwrap(), id);
}

#[test]
fn an_existing_key_is_sealed_without_changing_the_device() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let id = {
        let side = Side::open("Mac", state.path(), files.path());
        make_circle(&side)
    };
    std::thread::sleep(Duration::from_millis(300));
    let keys = device_keys(state.path());
    assert_eq!(std::fs::read(&keys[0]).unwrap().len(), 32);
    let side = Side::open_sealed("Mac", state.path(), files.path(), &sealing(7));
    let view = side.wait_snapshot("the same device", |s| s["me"]["id"].is_string());
    assert_eq!(view["me"]["id"].as_str().unwrap(), id);
    assert!(std::fs::read(&keys[0]).unwrap().starts_with(b"ZHSEAL1"));
}

#[test]
fn a_key_that_cannot_be_opened_is_reported_and_the_device_can_start_over() {
    let state = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let old = {
        let side = Side::open_sealed("Mac", state.path(), files.path(), &sealing(7));
        make_circle(&side)
    };
    std::thread::sleep(Duration::from_millis(300));
    let side = Side::start(state.path(), files.path(), &sealing(8));
    let refused = side.call("bootstrap", json!({ "allow_loopback": true }));
    assert_eq!(refused["error"]["kind"], "key_locked");
    assert!(side.snapshot()["me"].is_null());
    side.ok("start_over", Value::Null);
    side.ok("bootstrap", json!({ "allow_loopback": true }));
    let new = make_circle(&side);
    assert_ne!(new, old);
}
