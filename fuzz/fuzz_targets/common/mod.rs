//! Shared input shapes and the invariants every file-writing target checks.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use libfuzzer_sys::arbitrary::{self, Arbitrary};
use zerohop_core::names::{self, TMP_DIR_NAME};
use zerohop_core::receive::{SEGMENT_SIZE, SafeItem, segment_count};
use zerohop_core::wire::{Item, ItemKind};

/// Names picked to reach the interesting rules quickly.
pub const NAMES: &[&str] = &[
    "a", "A", "b", "B", "c.txt", "C.TXT", "cat.jpg", "run.py", "setup.exe", "keep.txt", "keepdir",
    "..", ".", "..\u{200E}", ".\u{200C}.", "\u{202E}gpj.exe", "CON", "con.txt", "NUL ", "trail.",
    "x/y", "x\\y", "\u{DF}", "SS", "\u{E9}", "e\u{301}", ".zerohop-tmp", ".ZEROHOP-tmp", "a:b",
];

#[derive(Arbitrary, Debug, Clone)]
pub enum Part {
    Known(u8),
    Raw(String),
}

impl Part {
    pub fn text(&self) -> String {
        match self {
            Self::Known(i) => NAMES[usize::from(*i) % NAMES.len()].to_string(),
            Self::Raw(s) => s.clone(),
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
pub struct RawItem {
    pub path: Vec<Part>,
    pub is_dir: bool,
    pub size: u8,
}

pub fn build_items(raw: &[RawItem]) -> Vec<Item> {
    raw.iter()
        .take(8)
        .enumerate()
        .map(|(i, r)| Item {
            index: i as u32,
            path: r.path.iter().take(3).map(Part::text).collect(),
            kind: if r.is_dir { ItemKind::Directory } else { ItemKind::File },
            size: if r.is_dir { 0 } else { u64::from(r.size) },
            mtime_ms: None,
            created_ms: None,
            group: None,
        })
        .collect()
}

#[derive(Arbitrary, Debug, Clone)]
pub enum Off {
    Next,
    Zero,
    Raw(u64),
}

#[derive(Arbitrary, Debug, Clone)]
pub enum Fill {
    Bytes(Vec<u8>),
    Mz,
    Script,
}

impl Fill {
    pub fn bytes(&self, size: usize) -> Vec<u8> {
        let seed: &[u8] = match self {
            Self::Bytes(b) if b.is_empty() => &[0],
            Self::Bytes(b) => b,
            Self::Mz => b"MZ\x90\0",
            Self::Script => b"#!/bin/sh\n",
        };
        seed.iter().copied().cycle().take(size).collect()
    }
}

pub fn hash(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

/// The hash of each segment of `data`, as the sender would report them.
pub fn segment_hashes(data: &[u8], size: u64) -> Vec<[u8; 32]> {
    let seg = SEGMENT_SIZE as usize;
    (0..segment_count(size) as usize)
        .map(|i| {
            let start = (i * seg).min(data.len());
            let end = ((i + 1) * seg).min(data.len());
            hash(&data[start..end])
        })
        .collect()
}

/// A receive folder inside an otherwise empty directory, holding things that must survive.
pub struct Sandbox {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
    pub recv: PathBuf,
}

impl Sandbox {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let recv = root.join("recv");
        fs::create_dir(&recv).unwrap();
        fs::write(recv.join("keep.txt"), b"keep").unwrap();
        fs::create_dir(recv.join("keepdir")).unwrap();
        fs::write(recv.join("keepdir").join("mine"), b"mine").unwrap();
        Self { _dir: dir, root, recv }
    }

    /// Every published file, as path components below the receive folder.
    pub fn published(&self) -> Vec<(Vec<String>, PathBuf)> {
        fn walk(dir: &Path, prefix: &[String], out: &mut Vec<(Vec<String>, PathBuf)>) {
            let Ok(entries) = fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if prefix.is_empty() && name == TMP_DIR_NAME {
                    continue;
                }
                let Ok(meta) = fs::symlink_metadata(entry.path()) else { continue };
                assert!(!meta.file_type().is_symlink(), "a symlink was created: {name:?}");
                let mut path = prefix.to_vec();
                path.push(name);
                if meta.is_dir() {
                    walk(&entry.path(), &path, out);
                } else {
                    out.push((path, entry.path()));
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.recv, &[], &mut out);
        out
    }

    /// Nothing escaped, nothing was overwritten or merged, and no disguised program got out.
    pub fn check(&self, hold: bool) {
        let outside: Vec<_> = fs::read_dir(&self.root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(outside.len(), 1, "something was written outside the receive folder: {outside:?}");
        assert_eq!(fs::read(self.recv.join("keep.txt")).unwrap(), b"keep");
        assert_eq!(fs::read(self.recv.join("keepdir").join("mine")).unwrap(), b"mine");
        assert_eq!(fs::read_dir(self.recv.join("keepdir")).unwrap().count(), 1, "merged into a folder");
        if !hold {
            return;
        }
        for (rel, path) in self.published() {
            let Ok(content) = fs::read(&path) else { continue };
            if names::looks_executable(&content) && !is_preexisting(&rel) {
                assert!(names::declared_as_program(&rel), "disguised program published: {rel:?}");
            }
        }
    }
}

fn is_preexisting(rel: &[String]) -> bool {
    matches!(rel, [a] if a == "keep.txt") || matches!(rel, [a, b] if a == "keepdir" && b == "mine")
}

/// Undoes the " (n)" a collision with the files already in the folder adds.
fn original_top(name: &str) -> String {
    let folded = names::fold_key(name);
    if folded.starts_with("keep (") && folded.ends_with(").txt") {
        "keep.txt".to_string()
    } else if folded.starts_with("keepdir (") {
        "keepdir".to_string()
    } else {
        name.to_string()
    }
}

/// Every published file is one the manifest named, complete, holding exactly
/// the bytes of a version of that file that passed its root hash.
pub fn check_matches_manifest(
    sandbox: &Sandbox,
    items: &[SafeItem],
    verified: &HashMap<u32, Vec<Vec<u8>>>,
) {
    let by_key: HashMap<String, &SafeItem> = items
        .iter()
        .map(|i| (names::fold_key(&i.path.join("/")), i))
        .collect();
    for (mut rel, path) in sandbox.published() {
        if is_preexisting(&rel) {
            continue;
        }
        rel[0] = original_top(&rel[0]);
        let key = names::fold_key(&rel.join("/"));
        let item = by_key
            .get(&key)
            .unwrap_or_else(|| panic!("published something that was never offered: {rel:?}"));
        assert_eq!(item.kind, ItemKind::File, "a file appeared where a folder was offered: {rel:?}");
        let content = fs::read(&path).unwrap();
        assert_eq!(content.len() as u64, item.size, "published a file of the wrong size: {rel:?}");
        let versions = verified.get(&item.index).map_or(&[][..], Vec::as_slice);
        assert!(versions.contains(&content), "published bytes that were never verified: {rel:?}");
    }
}
