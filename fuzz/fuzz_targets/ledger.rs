//! The rules of the device ledger, driven with validly signed events.
#![no_main]

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use ed25519_dalek::SigningKey;
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;
use zerohop_core::ledger::{DeviceId, EventKind, Ledger, Platform, sign_event};

const DEVICES: usize = 6;

fn keys() -> &'static Vec<SigningKey> {
    static KEYS: OnceLock<Vec<SigningKey>> = OnceLock::new();
    KEYS.get_or_init(|| {
        (0..DEVICES)
            .map(|i| SigningKey::from_bytes(&[u8::try_from(i).unwrap() + 1; 32]))
            .collect()
    })
}

fn id(i: u8) -> DeviceId {
    keys()[usize::from(i) % DEVICES].verifying_key().to_bytes()
}

#[derive(Arbitrary, Debug)]
enum Kind {
    Add { device: u8 },
    Revoke { device: u8, last_seen: u8 },
}

#[derive(Arbitrary, Debug)]
struct Op {
    author: u8,
    seq: u8,
    kind: Kind,
}

fuzz_target!(|ops: Vec<Op>| {
    let genesis = Ledger::create(&keys()[0], [7; 32], "first".to_string()).unwrap();
    let circle = *genesis.circle_id();
    let mut a = genesis.clone();
    let mut accepted = Vec::new();
    let mut revoked: BTreeSet<DeviceId> = BTreeSet::new();
    let mut versions: HashMap<(u8, u8), BTreeSet<[u8; 64]>> = HashMap::new();

    for op in ops.iter().take(48) {
        let kind = match op.kind {
            Kind::Add { device } => EventKind::Add {
                device: id(device),
                name: "d".to_string(),
                platform: Platform::Other,
            },
            Kind::Revoke { device, last_seen } => EventKind::Revoke {
                device: id(device),
                last_seen_seq: u32::from(last_seen % 8),
            },
        };
        let key = &keys()[usize::from(op.author) % DEVICES];
        let event = sign_event(key, &circle, u32::from(op.seq % 12) + 1, kind).unwrap();
        versions
            .entry((op.author % DEVICES as u8, op.seq % 12))
            .or_default()
            .insert(event.signature);
        if a.insert(event.clone()).is_ok() {
            accepted.push(event);
        }
        let view = a.membership();
        assert!(revoked.is_subset(&view.revoked), "a removal was undone");
        assert!(view.members.is_subset(&view.ever));
        assert!(view.members.is_disjoint(&view.revoked), "a removed device is still a member");
        assert!(view.members.contains(&id(0)) || view.revoked.contains(&id(0)));
        revoked = view.revoked;
    }

    // Past two versions of one number, the first ones to arrive are the ones kept,
    // so the rest of the checks only apply to authors that stayed below that.
    if versions.values().any(|v| v.len() > 2)
        || versions.get(&(0, 0)).is_some_and(|v| !v.is_empty())
    {
        return;
    }

    let mut reversed = genesis.clone();
    for event in accepted.iter().rev() {
        reversed.insert(event.clone()).unwrap();
    }
    assert_eq!(a.membership(), reversed.membership(), "the order of events changed the result");
    assert_eq!(a.heads(), reversed.heads());

    let mut synced = genesis.clone();
    for _ in 0..2 {
        for event in a.missing_for(&synced.heads()) {
            synced.insert(event).unwrap();
        }
    }
    assert_eq!(a.membership(), synced.membership(), "two sync rounds did not converge");
});
