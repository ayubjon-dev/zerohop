//! What a sender claims about its files.
#![no_main]

mod common;

use std::collections::{HashMap, HashSet};

use common::{RawItem, build_items};
use libfuzzer_sys::fuzz_target;
use zerohop_core::names::{TMP_DIR_NAME, TargetOs, fold_key, is_program_path};
use zerohop_core::receive::validate_manifest;
use zerohop_core::wire::ItemKind;

fuzz_target!(|input: (Vec<RawItem>, u8, u8)| {
    let (raw, os, extra) = input;
    let items = build_items(&raw);
    let os = [TargetOs::Windows, TargetOs::MacOs, TargetOs::Other][usize::from(os) % 3];
    let total: u64 = items.iter().map(|i| i.size).sum();
    let declared = total + u64::from(extra);
    let Ok(valid) = validate_manifest(&items, declared, os) else { return };

    assert_eq!(extra, 0, "accepted a false total");
    assert_eq!(valid.total_bytes, total);
    assert_eq!(valid.items.len(), items.len());
    assert_eq!(valid.has_program, valid.items.iter().any(|i| is_program_path(&i.path)));

    let mut whole = HashSet::new();
    let mut spelling: HashMap<String, String> = HashMap::new();
    let files: HashSet<String> = valid
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::File)
        .map(|i| fold_key(&i.path.join("/")))
        .collect();
    for item in &valid.items {
        assert!(whole.insert(fold_key(&item.path.join("/"))), "duplicate: {item:?}");
        for name in &item.path {
            assert_ne!(fold_key(name), TMP_DIR_NAME);
        }
        for depth in 1..item.path.len() {
            let parent = item.path[..depth].join("/");
            let key = fold_key(&parent);
            assert!(!files.contains(&key), "a file is also a folder: {item:?}");
            let known = spelling.entry(key).or_insert_with(|| parent.clone());
            assert_eq!(*known, parent, "two spellings of one folder");
        }
    }
});
