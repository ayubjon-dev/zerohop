//! A whole receiving session against a paired device that sends anything at all.
#![no_main]

mod common;

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use common::{Fill, Off, RawItem, Sandbox, build_items, hash, segment_hashes};
use ed25519_dalek::SigningKey;
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Runtime;
use tokio::sync::watch;
use zerohop_core::circle::Circle;
use zerohop_core::clip::{ClipSink, MAX_CLIP_TTL, ReceivedClip};
use zerohop_core::ledger::{CircleId, DeviceId, Platform};
use zerohop_core::receive::{ReceiveFolder, SEGMENT_SIZE, file_root};
use zerohop_core::session::{CircleHandle, LocalInfo, Session};
use zerohop_core::transfer::{self, Approver, Budgets, OfferSummary, ReceiveConfig, ReceiveMode, ReceiveSettings};
use zerohop_core::wire::{
    Cancel, ClipData, ClipEnd, ClipFormat, ClipItem, ClipOffer, Complete, DiscoveryKey, FileEnd, FrameType, Hello,
    Item, LedgerEvents, LedgerHave, Offer, OfferItems, ResumeRequest, SegmentEnd, file_data_header, write_msg,
    write_raw,
};

struct World {
    _dir: tempfile::TempDir,
    circle: CircleHandle,
    circle_id: CircleId,
    peer: DeviceId,
    members: BTreeSet<DeviceId>,
}

fn world() -> &'static World {
    static WORLD: OnceLock<World> = OnceLock::new();
    WORLD.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let mut circle = Circle::create(dir.path(), "host").unwrap();
        let peer = SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes();
        circle.add_pending(peer, "peer", Platform::Other).unwrap();
        circle.confirm_pending().unwrap();
        let circle_id = *circle.ledger().circle_id();
        let members = circle.ledger().membership().members;
        World { _dir: dir, circle: Arc::new(Mutex::new(circle)), circle_id, peer, members }
    })
}

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    })
}

/// Takes every clip, so the fetch path is always exercised.
struct Taking;

impl ClipSink for Taking {
    fn accepts(&self, _: &DeviceId, _: ClipFormat, _: u64) -> bool {
        true
    }

    fn deliver(&self, _: &DeviceId, clip: ReceivedClip) {
        assert!(!clip.parts.is_empty());
        assert!(clip.ttl <= MAX_CLIP_TTL);
    }
}

struct Fixed(bool);

impl Approver for Fixed {
    fn approve(&self, _: OfferSummary) -> Pin<Box<dyn Future<Output = bool> + Send>> {
        let answer = self.0;
        Box::pin(async move { answer })
    }
}

#[derive(Arbitrary, Debug)]
enum Step {
    Raw { kind: u8, payload: Vec<u8> },
    Offer { id: u8, items: Vec<RawItem> },
    Data { file: u8, off: Off, fill: Fill, len: u8 },
    SegmentEnd { file: u8, segment: u8, good: bool },
    FileEnd { file: u8, good: bool },
    Whole { file: u8, fill: Fill },
    Complete,
    Cancel,
    Resume { good: bool },
    ClipOffer { ttl: u32, items: Vec<RawClip> },
    ClipData { item: u8, off: Off, fill: Fill, len: u8 },
    ClipEnd { item: u8, good: bool },
}

#[derive(Arbitrary, Debug)]
struct RawClip {
    format: u8,
    size: u32,
    inline: Option<Vec<u8>>,
}

#[derive(Arbitrary, Debug)]
struct Input {
    mode: u8,
    answer: bool,
    sessions: Vec<Vec<Step>>,
}

#[derive(Default)]
struct Current {
    id: [u8; 16],
    items: Vec<Item>,
    written: HashMap<u32, Vec<u8>>,
    clip_bytes: Vec<u8>,
}

fn manifest_hash(items: &[Item]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    for item in items {
        h.update(&postcard::to_stdvec(item).unwrap());
    }
    *h.finalize().as_bytes()
}

type Wr = tokio::io::WriteHalf<tokio::io::DuplexStream>;

async fn send_whole(w: &mut Wr, cur: &mut Current, index: u32, fill: &Fill) -> Option<()> {
    let size = cur.items.get(index as usize)?.size;
    let seg = SEGMENT_SIZE as usize;
    let data = fill.bytes(size as usize);
    let hashes = segment_hashes(&data, size);
    for (i, h) in hashes.iter().enumerate() {
        let start = (i * seg).min(data.len());
        let end = (start + seg).min(data.len());
        if end > start {
            let mut payload = file_data_header(index, start as u64).to_vec();
            payload.extend_from_slice(&data[start..end]);
            write_raw(w, FrameType::FileData, &payload).await.ok()?;
        }
        let msg = SegmentEnd { file_index: index, segment: i as u32, hash: *h };
        write_msg(w, FrameType::SegmentEnd, &msg).await.ok()?;
    }
    let msg = FileEnd { file_index: index, root: file_root(size, &hashes) };
    write_msg(w, FrameType::FileEnd, &msg).await.ok()?;
    cur.written.insert(index, data);
    Some(())
}

async fn send_step(w: &mut Wr, cur: &mut Current, step: &Step) -> Option<()> {
    let count = cur.items.len().max(1) as u32;
    match step {
        Step::Raw { kind, payload } => {
            let len = payload.len().min(0xFF_FFFF) as u32;
            let [_, a, b, c] = len.to_be_bytes();
            w.write_all(&[*kind, a, b, c]).await.ok()?;
            w.write_all(payload).await.ok()?;
        }
        Step::Offer { id, items } => {
            let items = build_items(items);
            let offer = Offer {
                transfer_id: [*id; 16],
                item_count: items.len() as u32,
                total_bytes: items.iter().map(|i| i.size).sum(),
            };
            write_msg(w, FrameType::Offer, &offer).await.ok()?;
            if !items.is_empty() {
                write_msg(w, FrameType::OfferItems, &OfferItems { items: items.clone() }).await.ok()?;
            }
            *cur = Current { id: [*id; 16], items, written: HashMap::new(), clip_bytes: Vec::new() };
        }
        Step::Data { file, off, fill, len } => {
            let index = u32::from(*file) % count;
            let have = cur.written.get(&index).map_or(0, Vec::len) as u64;
            let offset = match off {
                Off::Next => have,
                Off::Zero => 0,
                Off::Raw(v) => *v,
            };
            let data = fill.bytes(usize::from(*len));
            let mut payload = file_data_header(index, offset).to_vec();
            payload.extend_from_slice(&data);
            write_raw(w, FrameType::FileData, &payload).await.ok()?;
            let shadow = cur.written.entry(index).or_default();
            if offset == 0 {
                shadow.clear();
            }
            shadow.extend_from_slice(&data);
        }
        Step::SegmentEnd { file, segment, good } => {
            let index = u32::from(*file) % count;
            let seg = SEGMENT_SIZE as usize;
            let shadow = cur.written.get(&index).map_or(&[][..], Vec::as_slice);
            let start = (usize::from(*segment) * seg).min(shadow.len());
            let end = (start + seg).min(shadow.len());
            let h = if *good { hash(&shadow[start..end]) } else { [9; 32] };
            let msg = SegmentEnd { file_index: index, segment: u32::from(*segment), hash: h };
            write_msg(w, FrameType::SegmentEnd, &msg).await.ok()?;
        }
        Step::FileEnd { file, good } => {
            let index = u32::from(*file) % count;
            let size = cur.items.get(index as usize).map_or(0, |i| i.size);
            let shadow = cur.written.get(&index).map_or(&[][..], Vec::as_slice);
            let root = if *good { file_root(size, &segment_hashes(shadow, size)) } else { [9; 32] };
            write_msg(w, FrameType::FileEnd, &FileEnd { file_index: index, root }).await.ok()?;
        }
        Step::Whole { file, fill } => {
            let index = u32::from(*file) % count;
            send_whole(w, cur, index, fill).await?;
        }
        Step::Complete => {
            let msg = Complete {
                transfer_id: cur.id,
                files: cur.items.len() as u32,
                bytes: cur.items.iter().map(|i| i.size).sum(),
            };
            write_msg(w, FrameType::Complete, &msg).await.ok()?;
        }
        Step::Cancel => {
            let msg = Cancel { transfer_id: cur.id, reason: 28 };
            write_msg(w, FrameType::Cancel, &msg).await.ok()?;
        }
        Step::Resume { good } => {
            let manifest = if *good { manifest_hash(&cur.items) } else { [1; 32] };
            let msg = ResumeRequest { transfer_id: cur.id, manifest };
            write_msg(w, FrameType::ResumeRequest, &msg).await.ok()?;
        }
        Step::ClipOffer { ttl, items } => {
            const FORMATS: [ClipFormat; 6] = [
                ClipFormat::Text,
                ClipFormat::Html,
                ClipFormat::Rtf,
                ClipFormat::Png,
                ClipFormat::Jpeg,
                ClipFormat::Files,
            ];
            let items: Vec<ClipItem> = items
                .iter()
                .take(10)
                .map(|raw| ClipItem {
                    format: FORMATS[usize::from(raw.format) % FORMATS.len()],
                    size: u64::from(raw.size),
                    inline: raw.inline.clone(),
                })
                .collect();
            // Short lifetimes, so a clip left unfinished cannot stall the run.
            let offer = ClipOffer { clip_id: [4; 16], ttl_ms: *ttl % 2000, items };
            write_msg(w, FrameType::ClipOffer, &offer).await.ok()?;
            cur.clip_bytes.clear();
        }
        Step::ClipData { item, off, fill, len } => {
            let offset = match off {
                Off::Next => cur.clip_bytes.len() as u64,
                Off::Zero => 0,
                Off::Raw(v) => *v,
            };
            let bytes = fill.bytes(usize::from(*len));
            let msg = ClipData { clip_id: [4; 16], item: u32::from(*item) % 4, offset, bytes };
            write_msg(w, FrameType::ClipData, &msg).await.ok()?;
            cur.clip_bytes.extend_from_slice(&msg.bytes);
        }
        Step::ClipEnd { item, good } => {
            let digest = if *good { hash(&cur.clip_bytes) } else { [9; 32] };
            let msg = ClipEnd { clip_id: [4; 16], item: u32::from(*item) % 4, hash: digest };
            write_msg(w, FrameType::ClipEnd, &msg).await.ok()?;
            cur.clip_bytes.clear();
        }
    }
    Some(())
}

async fn run_session(sandbox: &Sandbox, mode: ReceiveMode, answer: bool, steps: &[Step], cur: &mut Current) {
    let w = world();
    let (server_io, client_io) = tokio::io::duplex(1 << 16);
    let (mut reader, mut writer) = tokio::io::split(client_io);
    let cfg = ReceiveConfig {
        settings: ReceiveSettings::new(ReceiveFolder::open(&sandbox.recv).unwrap(), mode),
        approver: Arc::new(Fixed(answer)),
        auto_limit: 1 << 30,
        budgets: Arc::new(Budgets::default()),
        clip: Some(Arc::new(Taking)),
    };
    let (members_tx, members_rx) = watch::channel(w.members.clone());
    let server = tokio::spawn(async move {
        let info = LocalInfo { name: "host".to_string(), platform: Platform::Other };
        if let Ok(mut session) = Session::establish(server_io, w.peer, &w.circle, &info, members_rx).await {
            let _ = transfer::serve_session(&mut session, &cfg).await;
        }
    });
    let drain = tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        while matches!(reader.read(&mut buf).await, Ok(n) if n > 0) {}
    });

    let hello = Hello {
        software: "fuzz".to_string(),
        features: 0,
        name: "peer".to_string(),
        platform: Platform::Other,
        circle_id: w.circle_id,
    };
    let mut alive = write_msg(&mut writer, FrameType::Hello, &hello).await.is_ok();
    for _ in 0..2 {
        alive = alive && write_msg(&mut writer, FrameType::LedgerHave, &LedgerHave { authors: Vec::new() }).await.is_ok();
        alive = alive && write_msg(&mut writer, FrameType::LedgerEvents, &LedgerEvents { events: Vec::new() }).await.is_ok();
    }
    let key = DiscoveryKey { generation: 1, key: [3; 32] };
    alive = alive && write_msg(&mut writer, FrameType::DiscoveryKey, &key).await.is_ok();
    if alive {
        for step in steps.iter().take(24) {
            if send_step(&mut writer, cur, step).await.is_none() {
                break;
            }
        }
    }
    let _ = writer.flush().await;
    let _ = writer.shutdown().await;
    drop(writer);
    let finished = tokio::time::timeout(Duration::from_secs(20), server).await;
    assert!(finished.is_ok(), "the session hung after the peer closed the connection");
    drain.abort();
    drop(members_tx);
}

fuzz_target!(|input: Input| {
    let mode = [ReceiveMode::AlwaysAsk, ReceiveMode::Automatic, ReceiveMode::AutomaticExceptPrograms, ReceiveMode::Refuse]
        [usize::from(input.mode) % 4];
    let sandbox = Sandbox::new();
    let mut current = Current::default();
    runtime().block_on(async {
        for steps in input.sessions.iter().take(3) {
            run_session(&sandbox, mode, input.answer, steps, &mut current).await;
        }
    });
    sandbox.check(mode != ReceiveMode::Automatic);
});
