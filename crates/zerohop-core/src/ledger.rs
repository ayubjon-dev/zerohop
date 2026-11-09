//! The signed event ledger that defines a circle of devices (protocol section 5).
//!
//! Membership is a pure function of the set of events held, so it cannot
//! depend on the order events arrive in. Revocation is monotonic: the steps
//! that decide who is revoked use nothing that can shrink as events arrive.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;
use sha2::{Digest, Sha256};

/// A device's Ed25519 public key.
pub type DeviceId = [u8; 32];
/// The identifier of a circle.
pub type CircleId = [u8; 32];

const SIGN_PREFIX: &[u8] = b"zerohop-v1/ledger\0";
const CIRCLE_PREFIX: &[u8] = b"zerohop-v1/circle\0";
const CHAIN_PREFIX: &[u8] = b"zerohop-v1/chain\0";

/// Highest usable sequence number for one author.
pub const MAX_SEQ: u32 = 128;
/// How many distinct sequence numbers holding an `Add` count per author.
pub const MAX_ADDS_PER_AUTHOR: usize = 32;
/// Most non-revoked devices in a circle.
pub const MAX_MEMBERS: usize = 64;
/// Most non-`Revoke` events held.
pub const MAX_EVENTS: usize = 4096;
/// Most `Revoke` events held.
pub const MAX_REVOKES: usize = 1024;
/// Room beyond the limits above, set aside for events from current members.
/// Without it, a flood could keep the owner from removing the device behind it.
pub const MAX_RESERVED: usize = MAX_MEMBERS * MAX_SEQ as usize;
/// Most events a ledger ever holds.
pub const MAX_HELD: usize = MAX_EVENTS + MAX_REVOKES + MAX_RESERVED;
/// Longest display name in bytes.
pub const MAX_NAME_BYTES: usize = 63;
/// Variants of one `(author, seq)` that are stored. Two are enough to prove a fork.
const MAX_VARIANTS: usize = 2;

/// The kind of device an entry describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    /// Windows.
    Windows,
    /// macOS.
    MacOs,
    /// iOS or iPadOS.
    Ios,
    /// Android.
    Android,
    /// Linux.
    Linux,
    /// Anything else.
    Other,
}

/// What a ledger event does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    /// Creates the circle.
    Genesis {
        /// Random value that makes the circle ID unique.
        nonce: [u8; 32],
        /// Display name of the first device.
        name: String,
    },
    /// Adds a device to the circle.
    Add {
        /// The device being added.
        device: DeviceId,
        /// Its display name.
        name: String,
        /// Its platform.
        platform: Platform,
    },
    /// Removes a device from the circle, permanently.
    Revoke {
        /// The device being removed.
        device: DeviceId,
        /// The highest sequence number from that device the signer had seen.
        last_seen_seq: u32,
    },
}

/// The signed part of an event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventBody {
    /// The signing device.
    pub author: DeviceId,
    /// Per-author sequence number, starting at 1.
    pub seq: u32,
    /// What the event does.
    pub kind: EventKind,
}

/// An event with its signature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedEvent {
    /// The signed content.
    pub body: EventBody,
    /// Ed25519 signature over the domain-separated body.
    #[serde(with = "BigArray")]
    pub signature: [u8; 64],
}

/// One author's position in the ledger, exchanged during sync.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorHead {
    /// The author.
    pub author: DeviceId,
    /// Highest contiguous sequence number held.
    pub seq: u32,
    /// Hash chain over the author's events, or all `0xFF` if a fork is held.
    pub chain: [u8; 32],
}

/// Errors from adding events to a ledger.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum LedgerError {
    /// The signature or key was invalid.
    #[error("invalid event signature")]
    BadSignature,
    /// Sequence numbers start at 1.
    #[error("invalid sequence number")]
    BadSequence,
    /// The event could not be encoded.
    #[error("could not encode event")]
    Encoding,
    /// The ledger has reached its limit for this kind of event.
    #[error("ledger is full")]
    Full,
    /// A display name was too long.
    #[error("display name is too long")]
    NameTooLong,
}

fn encode_body(body: &EventBody) -> Result<Vec<u8>, LedgerError> {
    postcard::to_allocvec(body).map_err(|_| LedgerError::Encoding)
}

fn signing_input(circle: &CircleId, encoded: &[u8]) -> Vec<u8> {
    [SIGN_PREFIX, circle.as_slice(), encoded].concat()
}

fn body_hash(body: &EventBody) -> Result<[u8; 32], LedgerError> {
    Ok(Sha256::digest(encode_body(body)?).into())
}

/// Derives a circle ID from the creator's key and a nonce.
#[must_use]
pub fn circle_id_for(creator: &DeviceId, nonce: &[u8; 32]) -> CircleId {
    let mut h = Sha256::new();
    h.update(CIRCLE_PREFIX);
    h.update(creator);
    h.update(nonce);
    h.finalize().into()
}

/// Signs an event for a circle.
///
/// # Errors
/// Returns an error if the event cannot be encoded.
pub fn sign_event(
    key: &SigningKey,
    circle: &CircleId,
    seq: u32,
    kind: EventKind,
) -> Result<SignedEvent, LedgerError> {
    let body = EventBody {
        author: key.verifying_key().to_bytes(),
        seq,
        kind,
    };
    let signature = key
        .sign(&signing_input(circle, &encode_body(&body)?))
        .to_bytes();
    Ok(SignedEvent { body, signature })
}

/// The result of computing membership from a ledger.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Membership {
    /// Devices currently in the circle.
    pub members: BTreeSet<DeviceId>,
    /// Every device that was ever validly added.
    pub ever: BTreeSet<DeviceId>,
    /// Devices that have been removed.
    pub revoked: BTreeSet<DeviceId>,
    /// Which device added each member.
    pub added_by: BTreeMap<DeviceId, DeviceId>,
}

/// The set of events that define a circle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    circle_id: CircleId,
    events: BTreeMap<DeviceId, BTreeMap<u32, Vec<SignedEvent>>>,
}

impl Ledger {
    /// An empty ledger for a known circle, used by a joining device.
    #[must_use]
    pub const fn empty(circle_id: CircleId) -> Self {
        Self {
            circle_id,
            events: BTreeMap::new(),
        }
    }

    /// Creates a circle whose first device is `key`.
    ///
    /// # Errors
    /// Returns an error if the genesis event cannot be built.
    pub fn create(key: &SigningKey, nonce: [u8; 32], name: String) -> Result<Self, LedgerError> {
        let circle_id = circle_id_for(&key.verifying_key().to_bytes(), &nonce);
        let mut ledger = Self::empty(circle_id);
        ledger.insert(sign_event(
            key,
            &circle_id,
            1,
            EventKind::Genesis { nonce, name },
        )?)?;
        Ok(ledger)
    }

    /// The circle's identifier.
    #[must_use]
    pub const fn circle_id(&self) -> &CircleId {
        &self.circle_id
    }

    fn count(&self, revokes: bool) -> usize {
        self.all_events()
            .filter(|e| matches!(e.body.kind, EventKind::Revoke { .. }) == revokes)
            .count()
    }

    /// Every event held, in a stable order.
    pub fn all_events(&self) -> impl Iterator<Item = &SignedEvent> {
        self.events.values().flat_map(BTreeMap::values).flatten()
    }

    /// Verifies and stores an event. Returns whether the ledger changed.
    ///
    /// # Errors
    /// Returns an error for an invalid event or when the ledger is full.
    pub fn insert(&mut self, event: SignedEvent) -> Result<bool, LedgerError> {
        self.insert_tracking(event, &mut None)
    }

    /// As [`Self::insert`], for a batch from a peer. `view` keeps the current
    /// membership, so a full ledger works it out only once per batch. A slightly
    /// stale view only delays an event until the next sync.
    ///
    /// # Errors
    /// Returns an error for an invalid event or when the ledger is full.
    pub fn insert_tracking(
        &mut self,
        event: SignedEvent,
        view: &mut Option<Membership>,
    ) -> Result<bool, LedgerError> {
        if event.body.seq == 0 {
            return Err(LedgerError::BadSequence);
        }
        // Past the limit an event can never become usable, so it earns no space.
        if event.body.seq > MAX_SEQ {
            return Ok(false);
        }
        if let EventKind::Genesis { name, .. } | EventKind::Add { name, .. } = &event.body.kind
            && name.len() > MAX_NAME_BYTES
        {
            return Err(LedgerError::NameTooLong);
        }
        let encoded = encode_body(&event.body)?;
        let key =
            VerifyingKey::from_bytes(&event.body.author).map_err(|_| LedgerError::BadSignature)?;
        key.verify_strict(
            &signing_input(&self.circle_id, &encoded),
            &Signature::from_bytes(&event.signature),
        )
        .map_err(|_| LedgerError::BadSignature)?;

        let hash = body_hash(&event.body)?;
        let existing = self
            .events
            .get(&event.body.author)
            .and_then(|m| m.get(&event.body.seq));
        if existing.is_some_and(|v| v.iter().any(|e| e.body == event.body)) {
            return Ok(false);
        }
        // Further variants are refused rather than swapped in: whatever has been
        // stored keeps its effect, so no signature can undo an earlier one. The
        // genesis event is always stored, since nothing has meaning without it.
        let held = existing.map_or(&[][..], Vec::as_slice);
        let needed_genesis = self.is_genesis(&event) && !held.iter().any(|e| self.is_genesis(e));
        if held.len() >= MAX_VARIANTS && !needed_genesis {
            return Ok(false);
        }
        let is_revoke = matches!(event.body.kind, EventKind::Revoke { .. });
        let limit = if is_revoke { MAX_REVOKES } else { MAX_EVENTS };
        if self.count(is_revoke) >= limit {
            let current = match view {
                Some(v) => v,
                None => view.insert(self.membership()),
            };
            let author = event.body.author;
            // Every current member can always finish its own history, which
            // is what lets the owner remove the device causing a flood.
            let reserved = current.members.contains(&author)
                && self.held_by(&author) < MAX_SEQ as usize
                && self.all_events().count() < MAX_HELD;
            // Events from authors nobody added are only a cache (section 5.4).
            let made_room = !reserved
                && current.ever.contains(&author)
                && self.evict_unknown(&current.ever, is_revoke);
            if !reserved && !made_room {
                return Err(LedgerError::Full);
            }
        }

        let slot = self
            .events
            .entry(event.body.author)
            .or_default()
            .entry(event.body.seq)
            .or_default();
        let position = slot.partition_point(|e| body_hash(&e.body).is_ok_and(|h| h < hash));
        slot.insert(position, event);
        Ok(true)
    }

    fn is_genesis(&self, event: &SignedEvent) -> bool {
        matches!(&event.body.kind, EventKind::Genesis { nonce, .. }
            if event.body.seq == 1 && circle_id_for(&event.body.author, nonce) == self.circle_id)
    }

    fn held_by(&self, author: &DeviceId) -> usize {
        self.events
            .get(author)
            .map_or(0, |seqs| seqs.values().map(Vec::len).sum())
    }

    /// Drops one cached event by an author that was never added.
    fn evict_unknown(&mut self, known: &BTreeSet<DeviceId>, revokes: bool) -> bool {
        let is_kind = |e: &SignedEvent| matches!(e.body.kind, EventKind::Revoke { .. }) == revokes;
        let victim = self
            .events
            .iter()
            .rev()
            .filter(|(author, _)| !known.contains(*author))
            .find_map(|(author, seqs)| {
                let (seq, _) = seqs.iter().rev().find(|(_, v)| v.iter().any(is_kind))?;
                Some((*author, *seq))
            });
        let Some((author, seq)) = victim else {
            return false;
        };
        let Some(seqs) = self.events.get_mut(&author) else {
            return false;
        };
        if let Some(slot) = seqs.get_mut(&seq) {
            if let Some(found) = slot.iter().position(is_kind) {
                slot.remove(found);
            }
            if slot.is_empty() {
                seqs.remove(&seq);
            }
        }
        if seqs.is_empty() {
            self.events.remove(&author);
        }
        true
    }

    /// The highest sequence number held for an author, contiguous from 1.
    fn top(&self, author: &DeviceId) -> u32 {
        let Some(seqs) = self.events.get(author) else {
            return 0;
        };
        let mut top = 0;
        while seqs.contains_key(&(top + 1)) {
            top += 1;
        }
        top
    }

    /// The next sequence number this device should sign with.
    #[must_use]
    pub fn next_seq(&self, author: &DeviceId) -> u32 {
        self.events
            .get(author)
            .and_then(|m| m.keys().next_back())
            .map_or(1, |s| s.saturating_add(1))
    }

    /// Finds the author of this circle's genesis event.
    #[must_use]
    pub fn genesis_author(&self) -> Option<DeviceId> {
        self.all_events().find_map(|e| match &e.body.kind {
            EventKind::Genesis { nonce, .. }
                if e.body.seq == 1 && circle_id_for(&e.body.author, nonce) == self.circle_id =>
            {
                Some(e.body.author)
            }
            _ => None,
        })
    }

    /// Computes who belongs to the circle. See protocol section 5.2.
    #[must_use]
    pub fn membership(&self) -> Membership {
        let Some(genesis) = self.genesis_author() else {
            return Membership::default();
        };

        // Step 1: usable events.
        let tops: BTreeMap<DeviceId, u32> = self
            .events
            .keys()
            .map(|a| (*a, self.top(a).min(MAX_SEQ)))
            .collect();
        let usable = |e: &SignedEvent| {
            tops.get(&e.body.author)
                .is_some_and(|top| e.body.seq <= *top)
        };

        // Step 2: equivocation.
        let mut eq_cut: BTreeMap<DeviceId, u32> = BTreeMap::new();
        for (author, seqs) in &self.events {
            let top = tops.get(author).copied().unwrap_or(0);
            if let Some(seq) = seqs
                .iter()
                .find(|(seq, variants)| **seq <= top && variants.len() > 1)
                .map(|(seq, _)| *seq)
            {
                eq_cut.insert(*author, seq - 1);
            }
        }

        // Step 3: adds that count, per author, in seq order.
        let mut counted: BTreeMap<DeviceId, Vec<(u32, DeviceId)>> = BTreeMap::new();
        for (author, seqs) in &self.events {
            let mut distinct = 0;
            for (seq, variants) in seqs {
                let adds: Vec<DeviceId> = variants
                    .iter()
                    .filter(|e| usable(e))
                    .filter_map(|e| match &e.body.kind {
                        EventKind::Add { device, .. } => Some(*device),
                        _ => None,
                    })
                    .collect();
                if adds.is_empty() {
                    continue;
                }
                // A fork takes no place, or forking an earlier number could
                // push a later Add out of the count and shrink the set.
                if variants.len() == 1 {
                    distinct += 1;
                    if distinct > MAX_ADDS_PER_AUTHOR {
                        break;
                    }
                }
                let list = counted.entry(*author).or_default();
                list.extend(adds.into_iter().map(|d| (*seq, d)));
            }
        }

        // Step 4: ever-members. No cutoffs and forks included, so this only grows.
        let mut ever = BTreeSet::from([genesis]);
        let mut queue = vec![genesis];
        while let Some(author) = queue.pop() {
            for (_, device) in counted.get(&author).into_iter().flatten() {
                if ever.insert(*device) {
                    queue.push(*device);
                }
            }
        }

        // Steps 5 to 7: revocations and cutoffs.
        let mut revoked: BTreeSet<DeviceId> = eq_cut
            .keys()
            .filter(|a| ever.contains(*a))
            .copied()
            .collect();
        let mut cutoff: BTreeMap<DeviceId, u32> = eq_cut;
        for e in self.all_events().filter(|e| usable(e)) {
            if let EventKind::Revoke {
                device,
                last_seen_seq,
            } = &e.body.kind
                && ever.contains(&e.body.author)
                && ever.contains(device)
            {
                revoked.insert(*device);
                let value = if e.body.author == *device {
                    e.body.seq - 1
                } else {
                    *last_seen_seq
                };
                cutoff
                    .entry(*device)
                    .and_modify(|c| *c = (*c).min(value))
                    .or_insert(value);
            }
        }

        // Step 8: the live set, breadth first in a fixed order.
        let mut live = BTreeSet::from([genesis]);
        let mut added_by = BTreeMap::new();
        let mut non_revoked = usize::from(!revoked.contains(&genesis));
        let mut frontier = vec![genesis];
        while !frontier.is_empty() {
            frontier.sort_unstable();
            let mut next = Vec::new();
            for author in &frontier {
                let limit = cutoff.get(author).copied().unwrap_or(u32::MAX);
                for (seq, device) in counted.get(author).into_iter().flatten() {
                    if *seq > limit || live.contains(device) {
                        continue;
                    }
                    let is_revoked = revoked.contains(device);
                    if !is_revoked {
                        if non_revoked >= MAX_MEMBERS {
                            continue;
                        }
                        non_revoked += 1;
                    }
                    live.insert(*device);
                    added_by.insert(*device, *author);
                    next.push(*device);
                }
            }
            frontier = next;
        }

        // Step 9.
        let members = live.difference(&revoked).copied().collect();
        Membership {
            members,
            ever,
            revoked,
            added_by,
        }
    }

    /// This device's view of every author, for sync.
    #[must_use]
    pub fn heads(&self) -> Vec<AuthorHead> {
        self.events
            .iter()
            .map(|(author, seqs)| {
                let seq = self.top(author);
                let forked = seqs.range(..=seq).any(|(_, variants)| variants.len() > 1);
                let chain = if forked {
                    [0xFF; 32]
                } else {
                    seqs.range(..=seq)
                        .filter_map(|(_, v)| v.first())
                        .fold([0u8; 32], |prev, e| {
                            let mut h = Sha256::new();
                            h.update(CHAIN_PREFIX);
                            h.update(prev);
                            h.update(body_hash(&e.body).unwrap_or([0; 32]));
                            h.finalize().into()
                        })
                };
                AuthorHead {
                    author: *author,
                    seq,
                    chain,
                }
            })
            .collect()
    }

    /// The events a peer with these heads is missing.
    #[must_use]
    pub fn missing_for(&self, peer: &[AuthorHead]) -> Vec<SignedEvent> {
        let peers: BTreeMap<DeviceId, &AuthorHead> = peer.iter().map(|h| (h.author, h)).collect();
        let forked = [0xFF; 32];
        let mut out = Vec::new();
        for head in self.heads() {
            let Some(seqs) = self.events.get(&head.author) else {
                continue;
            };
            let from = match peers.get(&head.author) {
                None => 1,
                Some(p) if head.chain == forked || p.chain == forked => 1,
                Some(p) if p.seq == head.seq && p.chain != head.chain => 1,
                Some(p) => p.seq.saturating_add(1),
            };
            out.extend(seqs.range(from..).flat_map(|(_, v)| v.iter().cloned()));
        }
        out
    }

    /// Everything a device needs to verify its own removal: every event by
    /// every author, since a `Revoke` is meaningless without its author's
    /// earlier events and the `Add` path back to the genesis event.
    #[must_use]
    pub fn proof_of_removal(&self) -> Vec<SignedEvent> {
        self.all_events().cloned().collect()
    }

    /// The devices this device has added to the circle.
    #[must_use]
    pub fn devices_added_by(&self, device: &DeviceId) -> Vec<DeviceId> {
        let seen: HashSet<_> = self
            .all_events()
            .filter(|e| e.body.author == *device)
            .filter_map(|e| match &e.body.kind {
                EventKind::Add { device, .. } => Some(*device),
                _ => None,
            })
            .collect();
        seen.into_iter().collect()
    }

    /// The platform recorded when a device was added. The first device in a
    /// circle has none, because nothing vouches for it at that point.
    #[must_use]
    pub fn platform_of(&self, device: &DeviceId) -> Option<Platform> {
        self.all_events().find_map(|e| match &e.body.kind {
            EventKind::Add {
                device: d,
                platform,
                ..
            } if d == device => Some(*platform),
            _ => None,
        })
    }

    /// The display name recorded for a device.
    #[must_use]
    pub fn name_of(&self, device: &DeviceId) -> Option<&str> {
        self.all_events().find_map(|e| match &e.body.kind {
            EventKind::Add {
                device: d, name, ..
            } if d == device => Some(name.as_str()),
            EventKind::Genesis { name, .. } if e.body.author == *device => Some(name.as_str()),
            _ => None,
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::indexing_slicing,
    clippy::unwrap_used
)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    struct World {
        keys: BTreeMap<&'static str, SigningKey>,
        ledger: Ledger,
    }

    fn key(name: &str) -> SigningKey {
        let mut seed = [0u8; 32];
        for (i, b) in name.bytes().enumerate() {
            seed[i % 32] ^= b.wrapping_add(i as u8);
        }
        seed[31] ^= 0x5A;
        SigningKey::from_bytes(&seed)
    }

    fn id(name: &str) -> DeviceId {
        key(name).verifying_key().to_bytes()
    }

    impl World {
        fn new() -> Self {
            let g = key("G");
            let ledger = Ledger::create(&g, [7; 32], "G".to_string()).unwrap();
            Self {
                keys: BTreeMap::from([("G", g)]),
                ledger,
            }
        }

        fn sign(&mut self, who: &'static str, seq: u32, kind: EventKind) -> SignedEvent {
            let k = self.keys.entry(who).or_insert_with(|| key(who));
            sign_event(k, self.ledger.circle_id(), seq, kind).unwrap()
        }

        fn add(&mut self, who: &'static str, seq: u32, device: &str) {
            let e = self.sign(
                who,
                seq,
                EventKind::Add {
                    device: id(device),
                    name: device.to_string(),
                    platform: Platform::Other,
                },
            );
            self.ledger.insert(e).unwrap();
        }

        fn revoke(&mut self, who: &'static str, seq: u32, device: &str, last: u32) {
            let e = self.sign(
                who,
                seq,
                EventKind::Revoke {
                    device: id(device),
                    last_seen_seq: last,
                },
            );
            self.ledger.insert(e).unwrap();
        }

        fn members(&self) -> BTreeSet<DeviceId> {
            self.ledger.membership().members
        }
    }

    fn set(names: &[&str]) -> BTreeSet<DeviceId> {
        names.iter().map(|n| id(n)).collect()
    }

    fn base() -> World {
        let mut w = World::new();
        w.add("G", 2, "M");
        w.add("G", 3, "P");
        w.add("P", 1, "Y");
        w
    }

    #[test]
    fn computes_members_from_adds() {
        assert_eq!(base().members(), set(&["G", "M", "P", "Y"]));
    }

    #[test]
    fn revoked_device_cannot_return_after_signing_a_late_revoke() {
        let mut w = base();
        w.revoke("Y", 1, "M", 0);
        w.revoke("M", 1, "P", 0);
        let view = w.ledger.membership();
        assert!(view.revoked.contains(&id("M")));
        assert_eq!(view.members, set(&["G"]));
    }

    #[test]
    fn ignores_a_revoke_from_an_outsider() {
        let mut w = base();
        w.revoke("X", 1, "G", 0);
        assert_eq!(w.members(), set(&["G", "M", "P", "Y"]));
    }

    #[test]
    fn revoked_device_cannot_add_afterwards() {
        let mut w = base();
        w.revoke("G", 4, "M", 0);
        w.add("M", 1, "ROGUE");
        assert_eq!(w.members(), set(&["G", "P", "Y"]));
    }

    #[test]
    fn an_add_the_revoker_saw_survives() {
        let mut w = base();
        w.add("M", 1, "T");
        w.revoke("G", 4, "M", 1);
        assert_eq!(w.members(), set(&["G", "P", "Y", "T"]));
    }

    #[test]
    fn an_add_the_revoker_had_not_seen_is_dropped() {
        let mut w = base();
        w.add("M", 1, "T");
        w.revoke("G", 4, "M", 0);
        assert_eq!(w.members(), set(&["G", "P", "Y"]));
    }

    #[test]
    fn equivocation_removes_the_author_and_voids_the_forks() {
        let mut w = base();
        w.add("M", 1, "R1");
        w.add("M", 1, "R2");
        assert_eq!(w.members(), set(&["G", "P", "Y"]));
    }

    #[test]
    fn equivocation_cannot_reinstate_a_revoked_device() {
        let mut w = World::new();
        w.add("G", 2, "M");
        w.add("G", 3, "Z");
        w.add("M", 1, "Y");
        w.revoke("Y", 1, "Z", 0);
        w.add("M", 1, "FORK");
        let view = w.ledger.membership();
        assert!(view.revoked.contains(&id("Z")));
        assert_eq!(view.members, set(&["G"]));
    }

    #[test]
    fn mutual_revocation_removes_both() {
        let mut w = World::new();
        w.add("G", 2, "A");
        w.add("G", 3, "B");
        w.revoke("A", 1, "B", 0);
        w.revoke("B", 1, "A", 0);
        assert_eq!(w.members(), set(&["G"]));
    }

    #[test]
    fn the_genesis_device_can_be_revoked() {
        let mut w = World::new();
        w.add("G", 2, "A");
        w.add("G", 3, "B");
        w.revoke("A", 1, "G", 3);
        assert_eq!(w.members(), set(&["A", "B"]));
    }

    #[test]
    fn events_after_a_gap_are_unusable_until_filled() {
        let mut w = base();
        w.add("M", 2, "T");
        assert_eq!(w.members(), set(&["G", "M", "P", "Y"]));
        w.add("M", 1, "S");
        assert_eq!(w.members(), set(&["G", "M", "P", "Y", "S", "T"]));
    }

    #[test]
    fn an_inflated_self_revoke_cannot_keep_adding() {
        let mut w = base();
        w.revoke("M", 1, "M", u32::MAX);
        w.add("M", 2, "R");
        assert_eq!(w.members(), set(&["G", "P", "Y"]));
    }

    #[test]
    fn revoking_again_removes_devices_kept_by_a_colluding_cutoff() {
        let mut w = base();
        w.add("M", 1, "X");
        w.revoke("X", 1, "M", u32::MAX);
        w.add("M", 2, "R");
        assert!(w.members().contains(&id("R")));
        w.revoke("G", 4, "M", 0);
        assert_eq!(w.members(), set(&["G", "P", "Y"]));
    }

    #[test]
    fn a_fork_cannot_push_a_later_add_out_of_the_count() {
        let mut w = World::new();
        w.add("G", 2, "M");
        w.add("G", 3, "A");
        let names: Vec<&'static str> = (0..31)
            .map(|i| &*Box::leak(format!("D{i:02}").into_boxed_str()))
            .collect();
        for (i, n) in names.iter().enumerate() {
            w.add("A", i as u32 + 1, n);
        }
        w.add("A", 32, "Y");
        w.revoke("Y", 1, "M", 0);
        assert!(w.ledger.membership().revoked.contains(&id("M")));
        w.add("A", 1, "FORK");
        let view = w.ledger.membership();
        assert!(view.revoked.contains(&id("M")));
        assert!(!view.members.contains(&id("M")));
    }

    #[test]
    fn rejects_a_tampered_event() {
        let mut w = base();
        let mut e = w.sign(
            "G",
            4,
            EventKind::Add {
                device: id("N"),
                name: "N".to_string(),
                platform: Platform::Other,
            },
        );
        e.body.seq = 5;
        assert_eq!(w.ledger.insert(e), Err(LedgerError::BadSignature));
    }

    #[test]
    fn rejects_an_event_signed_for_another_circle() {
        let mut w = base();
        let e = sign_event(
            &key("G"),
            &[9; 32],
            4,
            EventKind::Add {
                device: id("N"),
                name: "N".to_string(),
                platform: Platform::Other,
            },
        )
        .unwrap();
        assert_eq!(w.ledger.insert(e), Err(LedgerError::BadSignature));
    }

    #[test]
    fn sync_surfaces_a_fork_hidden_at_equal_heads() {
        let mut a = base();
        let mut b = base();
        a.add("M", 1, "R1");
        b.add("M", 1, "R2");
        let to_b = a.ledger.missing_for(&b.ledger.heads());
        for e in to_b {
            b.ledger.insert(e).unwrap();
        }
        assert!(b.ledger.membership().revoked.contains(&id("M")));
    }

    #[test]
    fn extra_variants_cannot_undo_a_removal() {
        let mut w = base();
        w.add("M", 1, "A");
        w.revoke("A", 1, "P", 0);
        assert!(w.ledger.membership().revoked.contains(&id("P")));
        // Whoever holds M's key grinds out more versions of the Add that gave A its authority.
        for i in 0..16 {
            w.add("M", 1, &format!("junk{i}"));
        }
        let view = w.ledger.membership();
        assert!(view.revoked.contains(&id("P")));
        assert!(!view.members.contains(&id("P")));
    }

    #[test]
    fn extra_variants_cannot_remove_the_genesis_event() {
        let mut w = base();
        for i in 0..16 {
            w.add("G", 1, &format!("junk{i}"));
        }
        assert_eq!(w.ledger.genesis_author(), Some(id("G")));
        assert!(w.ledger.membership().ever.contains(&id("G")));
    }

    #[test]
    fn two_variants_agree_in_any_order() {
        let mut a = base();
        let mut b = base();
        let v: Vec<SignedEvent> = ["R1", "R2"]
            .iter()
            .map(|d| {
                a.sign(
                    "M",
                    1,
                    EventKind::Add {
                        device: id(d),
                        name: (*d).to_string(),
                        platform: Platform::Other,
                    },
                )
            })
            .collect();
        for e in [&v[0], &v[1]] {
            a.ledger.insert((*e).clone()).unwrap();
        }
        for e in [&v[1], &v[0]] {
            b.ledger.insert((*e).clone()).unwrap();
        }
        assert_eq!(a.ledger, b.ledger);
    }

    #[test]
    fn a_third_variant_is_refused_and_replaces_nothing() {
        let mut w = base();
        w.add("M", 1, "A");
        w.add("M", 1, "B");
        let held: Vec<_> = w.ledger.all_events().cloned().collect();
        w.add("M", 1, "C");
        assert_eq!(w.ledger.all_events().cloned().collect::<Vec<_>>(), held);
    }

    #[test]
    fn forking_a_number_without_an_add_cannot_push_a_later_add_out() {
        let mut w = base();
        w.revoke("M", 1, "NOBODY", 0);
        let adds = u32::try_from(MAX_ADDS_PER_AUTHOR).unwrap();
        for seq in 2..=adds {
            w.add("M", seq, &format!("d{seq}"));
        }
        w.add("M", adds + 1, "A");
        w.revoke("A", 1, "P", 0);
        assert!(w.ledger.membership().revoked.contains(&id("P")));
        // A second version of seq 1 now holds an Add, ahead of all the others.
        w.add("M", 1, "LATE");
        let view = w.ledger.membership();
        assert!(
            view.ever.contains(&id("A")),
            "the set of ever-members shrank"
        );
        assert!(view.revoked.contains(&id("P")), "a removal was undone");
    }

    #[test]
    fn events_past_the_sequence_limit_are_not_stored() {
        let mut w = base();
        let e = w.sign(
            "M",
            MAX_SEQ + 1,
            EventKind::Add {
                device: id("LATE"),
                name: String::new(),
                platform: Platform::Other,
            },
        );
        assert_eq!(w.ledger.insert(e), Ok(false));
    }

    #[test]
    fn missing_for_sends_only_what_a_peer_lacks() {
        let a = base();
        let mut b = World::new();
        b.add("G", 2, "M");
        let missing = a.ledger.missing_for(&b.ledger.heads());
        assert_eq!(missing.len(), 2);
    }

    #[test]
    fn a_current_member_can_still_add_and_remove_after_others_fill_the_ledger() {
        let mut w = World::new();
        w.add("G", 2, "M");
        // M creates devices of its own and has each one sign as much as it can.
        let adds = u32::try_from(MAX_ADDS_PER_AUTHOR).unwrap();
        let helpers: Vec<&'static str> = (0..adds)
            .map(|i| &*Box::leak(format!("H{i}").into_boxed_str()))
            .collect();
        for (i, h) in helpers.iter().enumerate() {
            w.add("M", u32::try_from(i).unwrap() + 1, h);
        }
        let mut made = 0;
        'flood: for h in &helpers {
            for seq in 1..=MAX_SEQ {
                let e = w.sign(
                    h,
                    seq,
                    EventKind::Add {
                        device: [u8::try_from(seq).unwrap(); 32],
                        name: String::new(),
                        platform: Platform::Other,
                    },
                );
                w.ledger.insert(e).unwrap();
                made += 1;
                if w.ledger.count(false) >= MAX_EVENTS {
                    break 'flood;
                }
            }
        }
        assert!(made > 0 && w.ledger.count(false) >= MAX_EVENTS);
        w.add("G", 3, "LATE");
        w.revoke("G", 4, "M", 0);
        let view = w.ledger.membership();
        assert!(view.members.contains(&id("LATE")));
        assert!(!view.members.contains(&id("M")));
    }

    #[test]
    fn rejects_an_over_long_display_name() {
        let mut w = World::new();
        let e = w.sign(
            "G",
            2,
            EventKind::Add {
                device: id("N"),
                name: "x".repeat(MAX_NAME_BYTES + 1),
                platform: Platform::Other,
            },
        );
        assert_eq!(w.ledger.insert(e), Err(LedgerError::NameTooLong));
    }

    fn scenario() -> Vec<SignedEvent> {
        let mut w = base();
        w.revoke("Y", 1, "M", 0);
        w.revoke("M", 1, "P", 0);
        w.add("G", 4, "Q");
        w.add("Q", 1, "W");
        w.ledger.all_events().cloned().collect()
    }

    proptest! {
        #[test]
        fn membership_does_not_depend_on_arrival_order(
            order in Just(scenario()).prop_shuffle()
        ) {
            let circle = *base().ledger.circle_id();
            let mut l = Ledger::empty(circle);
            for e in order {
                l.insert(e).unwrap();
            }
            let reference = {
                let mut r = Ledger::empty(circle);
                for e in scenario() {
                    r.insert(e).unwrap();
                }
                r
            };
            prop_assert_eq!(l.membership(), reference.membership());
        }

        #[test]
        fn revoked_device_never_returns_whatever_else_arrives(mask in 0u32..64) {
            let mut w = World::new();
            w.add("G", 2, "M");
            w.add("G", 3, "P");
            w.revoke("P", 1, "M", 0);
            if mask & 1 != 0 { w.revoke("M", 1, "P", 0); }
            if mask & 2 != 0 { w.add("M", 2, "X"); }
            if mask & 4 != 0 { w.add("M", 1, "X2"); }
            if mask & 8 != 0 { w.revoke("M", 2, "G", 0); }
            if mask & 16 != 0 { w.add("P", 2, "K"); }
            if mask & 32 != 0 { w.revoke("K", 1, "P", 0); }
            let view = w.ledger.membership();
            prop_assert!(view.revoked.contains(&id("M")));
            prop_assert!(!view.members.contains(&id("M")));
        }
    }
}
