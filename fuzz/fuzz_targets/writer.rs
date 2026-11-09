//! The receive path driven directly, in any order, including interruptions.
#![no_main]

mod common;

use std::collections::HashMap;

use common::{Fill, Off, RawItem, Sandbox, build_items, hash, segment_hashes};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;
use zerohop_core::names::{TMP_DIR_NAME, TargetOs};
use zerohop_core::receive::{ReceiveFolder, SEGMENT_SIZE, TransferWriter, file_root, validate_manifest};

const ID: [u8; 16] = [1; 16];
const PEER: [u8; 32] = [2; 32];
const MANIFEST: [u8; 32] = [3; 32];

#[derive(Arbitrary, Debug)]
enum Op {
    Write { file: u8, off: Off, fill: Fill, len: u8 },
    SegmentEnd { file: u8, segment: u8, good: bool },
    FileEnd { file: u8, good: bool },
    Whole { file: u8, fill: Fill },
    Checkpoint,
    Reopen,
    Finish,
    Cancel,
}

#[derive(Arbitrary, Debug)]
struct Input {
    items: Vec<RawItem>,
    hold: bool,
    ops: Vec<Op>,
}

fuzz_target!(|input: Input| {
    let items = build_items(&input.items);
    if items.is_empty() {
        return;
    }
    let total = items.iter().map(|i| i.size).sum();
    let Ok(valid) = validate_manifest(&items, total, TargetOs::current()) else { return };
    let sandbox = Sandbox::new();
    let folder = ReceiveFolder::open(&sandbox.recv).unwrap();
    let Ok(writer) = TransferWriter::begin(&folder, &ID, PEER, MANIFEST, &valid, input.hold) else {
        sandbox.check(input.hold);
        return;
    };
    let mut writer = Some(writer);
    let mut written: HashMap<u32, Vec<u8>> = HashMap::new();
    let mut verified: HashMap<u32, Vec<Vec<u8>>> = HashMap::new();
    let count = valid.items.len();
    let size_of = |i: u32| valid.items[i as usize].size;
    let seg = SEGMENT_SIZE as usize;

    for op in input.ops.iter().take(48) {
        match op {
            Op::Reopen => {
                drop(writer.take());
                let Ok((w, point)) = TransferWriter::resume(&folder, &ID, PEER, MANIFEST) else {
                    break;
                };
                for (index, data) in &mut written {
                    if *index > point.next_file {
                        data.clear();
                    } else if *index == point.next_file {
                        data.truncate(point.next_segment as usize * seg);
                    }
                }
                writer = Some(w);
            }
            Op::Finish => {
                let Some(w) = writer.take() else { continue };
                if let Ok(counts) = w.finish() {
                    if counts.held == 0 {
                        assert!(!sandbox.recv.join(TMP_DIR_NAME).exists(), "working files left behind");
                    }
                }
            }
            Op::Cancel => {
                let Some(w) = writer.take() else { continue };
                let _ = w.cancel();
            }
            Op::Checkpoint => {
                if let Some(w) = writer.as_mut() {
                    let _ = w.checkpoint();
                }
            }
            Op::Write { file, off, fill, len } => {
                let Some(w) = writer.as_mut() else { continue };
                let index = u32::from(*file) % count as u32;
                let have = written.get(&index).map_or(0, Vec::len) as u64;
                let offset = match off {
                    Off::Next => have,
                    Off::Zero => 0,
                    Off::Raw(v) => *v,
                };
                let data = fill.bytes(usize::from(*len));
                if w.write(index, offset, &data).is_ok() {
                    let shadow = written.entry(index).or_default();
                    if offset == 0 {
                        shadow.clear();
                    }
                    shadow.extend_from_slice(&data);
                } else {
                    writer = None;
                }
            }
            Op::SegmentEnd { file, segment, good } => {
                let Some(w) = writer.as_mut() else { continue };
                let index = u32::from(*file) % count as u32;
                let shadow = written.get(&index).map_or(&[][..], Vec::as_slice);
                let start = (usize::from(*segment) * seg).min(shadow.len());
                let end = (start + seg).min(shadow.len());
                let h = if *good { hash(&shadow[start..end]) } else { [9; 32] };
                if w.segment_end(index, u32::from(*segment), h).is_err() {
                    writer = None;
                }
            }
            Op::FileEnd { file, good } => {
                let Some(w) = writer.as_mut() else { continue };
                let index = u32::from(*file) % count as u32;
                let size = size_of(index);
                let shadow = written.get(&index).map_or(&[][..], Vec::as_slice);
                let root = if *good { file_root(size, &segment_hashes(shadow, size)) } else { [9; 32] };
                let bytes = shadow.to_vec();
                if w.file_end(index, root).is_ok() {
                    verified.entry(index).or_default().push(bytes);
                } else {
                    writer = None;
                }
            }
            Op::Whole { file, fill } => {
                let Some(w) = writer.as_mut() else { continue };
                let index = u32::from(*file) % count as u32;
                let size = size_of(index);
                let data = fill.bytes(size as usize);
                let hashes = segment_hashes(&data, size);
                let mut ok = true;
                for (i, h) in hashes.iter().enumerate() {
                    let start = (i * seg).min(data.len());
                    let end = (start + seg).min(data.len());
                    if end > start {
                        ok = ok && w.write(index, start as u64, &data[start..end]).is_ok();
                    }
                    ok = ok && w.segment_end(index, i as u32, *h).is_ok();
                }
                ok = ok && w.file_end(index, file_root(size, &hashes)).is_ok();
                if ok {
                    verified.entry(index).or_default().push(data.clone());
                    written.insert(index, data);
                } else {
                    writer = None;
                }
            }
        }
    }
    drop(writer);
    sandbox.check(input.hold);
    common::check_matches_manifest(&sandbox, &valid.items, &verified);
});
