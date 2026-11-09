//! A device's persistent membership in a circle.
//!
//! This file-based key store is for the command-line tool. The apps will keep
//! the device key in the platform's protected storage (protocol section 4).

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use aws_lc_rs::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use zeroize::Zeroizing;

use crate::ledger::{DeviceId, EventKind, Ledger, LedgerError, Platform, SignedEvent, sign_event};
use crate::wire::DiscoveryKey;

const KEY_FILE: &str = "device.key";
const CIRCLE_FILE: &str = "circle.bin";
const LOCK_FILE: &str = "state.lock";
const SEALED_MAGIC: &[u8; 8] = b"ZHSEAL1\0";
const SEAL_CONTEXT: &[u8] = b"zerohop device key";
const SEALED_BODY: usize = 32 + 16;
const SEALED_LEN: usize = SEALED_MAGIC.len() + NONCE_LEN + SEALED_BODY;

/// Errors from managing circle state.
#[derive(Debug, thiserror::Error)]
pub enum CircleError {
    /// Reading or writing state failed.
    #[error("state file error: {0}")]
    Io(#[from] io::Error),
    /// A state file was corrupt.
    #[error("state file is corrupt")]
    Corrupt,
    /// A ledger operation failed.
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    /// No random data was available.
    #[error("no secure random source available")]
    Random,
    /// This device's state was lost and it must not sign anything else.
    #[error("this device lost its state and must be paired again")]
    StateLost,
    /// A device with this name is not in the circle.
    #[error("no such device in the circle")]
    NoSuchDevice,
    /// Another process owns this device's state.
    #[error(
        "another Zerohop process is using this device's state: stop `zerohop listen` and try again"
    )]
    InUse,
    /// This view of the circle cannot change it.
    #[error("this view of the circle is read-only")]
    ReadOnly,
    /// The device key is sealed and this secret does not open it.
    #[error("the device key cannot be opened with the secret this system holds")]
    KeyLocked,
}

/// What protects the device key on disk.
///
/// The interface asks the operating system's key store for a secret and hands
/// it over at start. Without one, as for the command line tool, the key stays
/// in a file only its owner can read. The key cannot live in the key store
/// itself: neither the Secure Enclave nor StrongBox offers Ed25519.
#[derive(Clone, Default)]
pub struct KeySealing(Option<Arc<Zeroizing<[u8; 32]>>>);

impl KeySealing {
    /// Seals the device key with a secret kept by the operating system.
    #[must_use]
    pub fn with_secret(secret: Zeroizing<[u8; 32]>) -> Self {
        Self(Some(Arc::new(secret)))
    }

    fn secret(&self) -> Option<&[u8; 32]> {
        self.0.as_deref().map(|secret| &**secret)
    }

    fn encode(&self, seed: &[u8; 32]) -> Result<Zeroizing<Vec<u8>>, CircleError> {
        let Some(secret) = self.secret() else {
            return Ok(Zeroizing::new(seed.to_vec()));
        };
        let nonce: [u8; NONCE_LEN] = random_bytes()?;
        let mut body = Zeroizing::new(Vec::with_capacity(SEALED_BODY));
        body.extend_from_slice(seed);
        seal_cipher(secret)?
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(SEAL_CONTEXT),
                &mut *body,
            )
            .map_err(|_| CircleError::Corrupt)?;
        let mut out = Zeroizing::new(Vec::with_capacity(SEALED_LEN));
        out.extend_from_slice(SEALED_MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&body);
        Ok(out)
    }
}

impl std::fmt::Debug for KeySealing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "KeySealing(sealed)"
        } else {
            "KeySealing(none)"
        })
    }
}

fn seal_cipher(secret: &[u8; 32]) -> Result<LessSafeKey, CircleError> {
    let key = UnboundKey::new(&CHACHA20_POLY1305, secret).map_err(|_| CircleError::Corrupt)?;
    Ok(LessSafeKey::new(key))
}

fn is_sealed(stored: &[u8]) -> bool {
    stored.len() == SEALED_LEN && stored.starts_with(SEALED_MAGIC)
}

fn unseal(secret: &[u8; 32], stored: &[u8]) -> Result<Zeroizing<[u8; 32]>, CircleError> {
    let rest = stored
        .strip_prefix(SEALED_MAGIC)
        .ok_or(CircleError::KeyLocked)?;
    let (nonce, body) = rest
        .split_first_chunk::<NONCE_LEN>()
        .ok_or(CircleError::KeyLocked)?;
    let mut buffer = Zeroizing::new(body.to_vec());
    let plain = seal_cipher(secret)?
        .open_in_place(
            Nonce::assume_unique_for_key(*nonce),
            Aad::from(SEAL_CONTEXT),
            &mut buffer,
        )
        .map_err(|_| CircleError::KeyLocked)?;
    let seed: [u8; 32] = (&*plain).try_into().map_err(|_| CircleError::KeyLocked)?;
    Ok(Zeroizing::new(seed))
}

/// Fills an array with secure random bytes.
///
/// # Errors
/// Returns [`CircleError::Random`] if the operating system cannot supply them.
pub fn random_bytes<const N: usize>() -> Result<[u8; N], CircleError> {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).map_err(|_| CircleError::Random)?;
    Ok(out)
}

/// Generates a new device key.
///
/// # Errors
/// Returns [`CircleError::Random`] if no secure randomness is available.
pub fn generate_key() -> Result<SigningKey, CircleError> {
    let seed = Zeroizing::new(random_bytes::<32>()?);
    Ok(SigningKey::from_bytes(&seed))
}

/// Keys for recognizing devices on the network (protocol section 6).
#[derive(Clone, Serialize, Deserialize)]
struct DiscoveryKeys {
    own: DiscoveryKey,
    peers: BTreeMap<DeviceId, DiscoveryKey>,
}

#[derive(Serialize, Deserialize)]
struct CircleFile {
    ledger: Ledger,
    next_seq: u32,
    pending_add: Option<DeviceId>,
    state_lost: bool,
    discovery: DiscoveryKeys,
}

/// How this process may use the saved state.
enum Access {
    /// The only writer. The file holds the operating system's lock until dropped.
    Owner(#[allow(dead_code)] fs::File),
    /// A read-only copy. Whatever it learns is forgotten when it is dropped.
    Snapshot,
}

/// Takes the lock that makes this process the only writer.
///
/// Two writers could each sign the same sequence number, which the protocol
/// treats as a compromised device, and a stale writer could undo a removal.
fn acquire(dir: &Path) -> Result<Access, CircleError> {
    fs::create_dir_all(dir)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(LOCK_FILE))?;
    match file.try_lock() {
        Ok(()) => Ok(Access::Owner(file)),
        Err(fs::TryLockError::WouldBlock) => Err(CircleError::InUse),
        Err(fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

/// One device's view of its circle, saved on disk.
pub struct Circle {
    dir: PathBuf,
    access: Access,
    key: SigningKey,
    ledger: Ledger,
    next_seq: u32,
    pending_add: Option<DeviceId>,
    state_lost: bool,
    discovery: DiscoveryKeys,
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)?;
    sync_parent(path)
}

/// Makes a rename durable. Without it a power cut can bring back the old file,
/// and with it a sequence number that has already been used.
#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    path.parent()
        .map_or(Ok(()), |dir| fs::File::open(dir)?.sync_all())
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
const fn sync_parent(_: &Path) -> io::Result<()> {
    Ok(())
}

impl Circle {
    fn id(&self) -> DeviceId {
        self.key.verifying_key().to_bytes()
    }

    /// Creates a new circle with this device as its first member.
    ///
    /// # Errors
    /// Returns an error if the state cannot be created.
    pub fn create(dir: &Path, name: &str) -> Result<Self, CircleError> {
        Self::create_sealed(dir, name, &KeySealing::default())
    }

    /// Like [`Self::create`], sealing the device key on disk.
    ///
    /// # Errors
    /// Returns an error if the state cannot be created.
    pub fn create_sealed(
        dir: &Path,
        name: &str,
        sealing: &KeySealing,
    ) -> Result<Self, CircleError> {
        let key = generate_key()?;
        let ledger = Ledger::create(&key, random_bytes()?, name.to_string())?;
        Self::adopt_sealed(dir, key, ledger, 2, sealing)
    }

    /// Stores a circle this device has just joined.
    ///
    /// # Errors
    /// Returns an error if the state cannot be saved.
    pub fn adopt(
        dir: &Path,
        key: SigningKey,
        ledger: Ledger,
        next_seq: u32,
    ) -> Result<Self, CircleError> {
        Self::adopt_sealed(dir, key, ledger, next_seq, &KeySealing::default())
    }

    /// Like [`Self::adopt`], sealing the device key on disk.
    ///
    /// # Errors
    /// Returns an error if the state cannot be saved.
    pub fn adopt_sealed(
        dir: &Path,
        key: SigningKey,
        ledger: Ledger,
        next_seq: u32,
        sealing: &KeySealing,
    ) -> Result<Self, CircleError> {
        let access = acquire(dir)?;
        let seed = Zeroizing::new(key.to_bytes());
        write_private(&dir.join(KEY_FILE), &sealing.encode(&seed)?)?;
        let circle = Self {
            dir: dir.to_path_buf(),
            access,
            key,
            ledger,
            next_seq,
            pending_add: None,
            state_lost: false,
            discovery: DiscoveryKeys {
                own: DiscoveryKey {
                    generation: 1,
                    key: random_bytes()?,
                },
                peers: BTreeMap::new(),
            },
        };
        circle.save()?;
        Ok(circle)
    }

    /// Loads saved state as its only writer and finishes any interrupted pairing.
    ///
    /// # Errors
    /// Returns [`CircleError::InUse`] if another process owns the state, or an
    /// error if it cannot be read.
    pub fn load(dir: &Path) -> Result<Self, CircleError> {
        Self::load_sealed(dir, &KeySealing::default())
    }

    /// Like [`Self::load`], for a device key sealed on disk. A key still in
    /// its old readable form is sealed on the way.
    ///
    /// # Errors
    /// Returns [`CircleError::KeyLocked`] if the secret does not open the key,
    /// [`CircleError::InUse`] if another process owns the state, or an error if
    /// it cannot be read.
    pub fn load_sealed(dir: &Path, sealing: &KeySealing) -> Result<Self, CircleError> {
        let access = acquire(dir)?;
        let mut circle = Self::read(dir, access, sealing)?;
        if let Some(device) = circle.pending_add {
            circle.abandon_pending(device)?;
        }
        Ok(circle)
    }

    /// Reads saved state without taking ownership. Nothing is ever written.
    ///
    /// # Errors
    /// Returns an error if the state cannot be read.
    pub fn snapshot(dir: &Path) -> Result<Self, CircleError> {
        Self::read(dir, Access::Snapshot, &KeySealing::default())
    }

    /// Takes ownership if nothing else holds it, and otherwise reads a snapshot.
    ///
    /// # Errors
    /// Returns an error if the state cannot be read.
    pub fn open(dir: &Path) -> Result<Self, CircleError> {
        match Self::load(dir) {
            Err(CircleError::InUse) => Self::snapshot(dir),
            other => other,
        }
    }

    fn read_key(
        dir: &Path,
        access: &Access,
        sealing: &KeySealing,
    ) -> Result<SigningKey, CircleError> {
        let path = dir.join(KEY_FILE);
        let stored = Zeroizing::new(fs::read(&path)?);
        if is_sealed(&stored) {
            let secret = sealing.secret().ok_or(CircleError::KeyLocked)?;
            return Ok(SigningKey::from_bytes(&*unseal(secret, &stored)?));
        }
        let seed: &[u8; 32] = stored
            .as_slice()
            .try_into()
            .map_err(|_| CircleError::Corrupt)?;
        if sealing.secret().is_some() && matches!(access, Access::Owner(_)) {
            // Replaced in one step: a crash leaves the old file, never neither.
            write_private(&path, &sealing.encode(seed)?)?;
        }
        Ok(SigningKey::from_bytes(seed))
    }

    fn read(dir: &Path, access: Access, sealing: &KeySealing) -> Result<Self, CircleError> {
        let key = Self::read_key(dir, &access, sealing)?;
        let file: CircleFile = postcard::from_bytes(&fs::read(dir.join(CIRCLE_FILE))?)
            .map_err(|_| CircleError::Corrupt)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            access,
            key,
            ledger: file.ledger,
            next_seq: file.next_seq,
            pending_add: file.pending_add,
            state_lost: file.state_lost,
            discovery: file.discovery,
        })
    }

    /// Whether this directory holds a circle.
    #[must_use]
    pub fn exists(dir: &Path) -> bool {
        dir.join(KEY_FILE).exists() && dir.join(CIRCLE_FILE).exists()
    }

    fn save(&self) -> Result<(), CircleError> {
        if matches!(self.access, Access::Snapshot) {
            return Ok(());
        }
        let file = CircleFile {
            ledger: self.ledger.clone(),
            next_seq: self.next_seq,
            pending_add: self.pending_add,
            state_lost: self.state_lost,
            discovery: self.discovery.clone(),
        };
        let bytes = postcard::to_stdvec(&file).map_err(|_| CircleError::Corrupt)?;
        write_private(&self.dir.join(CIRCLE_FILE), &bytes)?;
        Ok(())
    }

    /// The device's signing key.
    #[must_use]
    pub const fn key(&self) -> &SigningKey {
        &self.key
    }

    /// The device's ID.
    #[must_use]
    pub fn device_id(&self) -> DeviceId {
        self.id()
    }

    /// The circle's ledger.
    #[must_use]
    pub const fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    fn sign(&mut self, kind: EventKind) -> Result<SignedEvent, CircleError> {
        if matches!(self.access, Access::Snapshot) {
            return Err(CircleError::ReadOnly);
        }
        if self.state_lost {
            return Err(CircleError::StateLost);
        }
        let seq = self.next_seq.max(self.ledger.next_seq(&self.id()));
        let event = sign_event(&self.key, self.ledger.circle_id(), seq, kind)?;
        let mut ledger = self.ledger.clone();
        if !ledger.insert(event.clone())? {
            // Past the last usable number: signing more would achieve nothing.
            return Err(LedgerError::Full.into());
        }
        // The event and its number reach the disk in one write, before the
        // event can leave this device. A crash or a refused event therefore
        // neither reuses a number anyone has seen nor leaves a gap, which
        // would make every later event from this device unusable.
        let old_ledger = std::mem::replace(&mut self.ledger, ledger);
        let old_seq = std::mem::replace(&mut self.next_seq, seq + 1);
        if let Err(e) = self.save() {
            self.ledger = old_ledger;
            self.next_seq = old_seq;
            return Err(e);
        }
        Ok(event)
    }

    /// Adds a device, keeping it out of sync until pairing is acknowledged.
    ///
    /// # Errors
    /// Returns an error if the event cannot be signed or saved.
    pub fn add_pending(
        &mut self,
        device: DeviceId,
        name: &str,
        platform: Platform,
    ) -> Result<(), CircleError> {
        let previous = self.pending_add.replace(device);
        let signed = self.sign(EventKind::Add {
            device,
            name: name.to_string(),
            platform,
        });
        if signed.is_err() {
            self.pending_add = previous;
        }
        signed.map(|_| ())
    }

    /// Marks a pending device as fully joined.
    ///
    /// # Errors
    /// Returns an error if the state cannot be saved.
    pub fn confirm_pending(&mut self) -> Result<(), CircleError> {
        self.pending_add = None;
        self.save()
    }

    /// Removes a device whose pairing never finished.
    ///
    /// # Errors
    /// Returns an error if the event cannot be signed or saved.
    pub fn abandon_pending(&mut self, device: DeviceId) -> Result<(), CircleError> {
        self.sign(EventKind::Revoke {
            device,
            last_seen_seq: 0,
        })?;
        self.pending_add = None;
        self.save()
    }

    /// Whether a device is currently a member.
    #[must_use]
    pub fn is_member(&self, device: &DeviceId) -> bool {
        self.ledger.membership().members.contains(device)
    }

    /// Removes another device, or this one to leave the circle.
    ///
    /// # Errors
    /// Returns an error if the device is unknown or the event cannot be signed.
    pub fn revoke(&mut self, device: DeviceId) -> Result<(), CircleError> {
        let view = self.ledger.membership();
        if !view.ever.contains(&device) {
            return Err(CircleError::NoSuchDevice);
        }
        let last_seen_seq = self
            .ledger
            .heads()
            .into_iter()
            .find(|h| h.author == device)
            .map_or(0, |h| h.seq);
        self.sign(EventKind::Revoke {
            device,
            last_seen_seq,
        })?;
        self.rotate_discovery_key()?;
        self.discovery.peers.remove(&device);
        self.save()
    }

    /// The key this device announces itself with.
    #[must_use]
    pub const fn own_discovery_key(&self) -> &DiscoveryKey {
        &self.discovery.own
    }

    /// The keys of the current members this device has learned.
    #[must_use]
    pub fn peer_discovery_keys(&self) -> BTreeMap<DeviceId, [u8; 32]> {
        let members = self.ledger.membership().members;
        self.discovery
            .peers
            .iter()
            .filter(|(id, _)| members.contains(*id))
            .map(|(id, k)| (*id, k.key))
            .collect()
    }

    /// Stores a key its owner handed over directly. Returns whether it was new.
    ///
    /// # Errors
    /// Returns an error if the state cannot be saved.
    pub fn learn_discovery_key(
        &mut self,
        owner: DeviceId,
        offered: DiscoveryKey,
    ) -> Result<bool, CircleError> {
        let newer = self
            .discovery
            .peers
            .get(&owner)
            .is_none_or(|known| offered.generation > known.generation);
        if !newer || owner == self.id() || !self.is_member(&owner) {
            return Ok(false);
        }
        self.discovery.peers.insert(owner, offered);
        self.save()?;
        Ok(true)
    }

    /// Makes this device unrecognizable to anyone who only has the old key.
    fn rotate_discovery_key(&mut self) -> Result<(), CircleError> {
        self.discovery.own = DiscoveryKey {
            generation: self.discovery.own.generation.saturating_add(1),
            key: random_bytes()?,
        };
        Ok(())
    }

    /// Applies events from a peer. Returns whether anything changed.
    ///
    /// A pending `Add` is not sent to peers, so nothing here can publish it.
    ///
    /// # Errors
    /// Returns an error if the state cannot be saved or the event is invalid.
    pub fn apply(&mut self, events: Vec<SignedEvent>) -> Result<bool, CircleError> {
        let me = self.id();
        let members_before = self.ledger.membership().members;
        let mut changed = false;
        let mut failure = None;
        let mut view = None;
        for event in events {
            // Only a validly signed event can prove this device lost state,
            // so an unsigned claim from a peer cannot stop it from signing.
            let own_future = event.body.author == me && event.body.seq >= self.next_seq;
            match self.ledger.insert_tracking(event, &mut view) {
                Ok(true) => {
                    changed = true;
                    if own_future {
                        self.state_lost = true;
                    }
                }
                Ok(false) => {}
                // Not stored, and offered again by the next sync. It must not
                // stop a removal further along in the same batch.
                Err(LedgerError::Full) => {}
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        if changed {
            let members = self.ledger.membership().members;
            if members_before.iter().any(|d| !members.contains(d)) {
                self.rotate_discovery_key()?;
                self.discovery.peers.retain(|id, _| members.contains(id));
            }
            self.save()?;
        }
        failure.map_or(Ok(changed), |e| Err(e.into()))
    }

    /// The events a peer is missing, excluding an unconfirmed pairing.
    #[must_use]
    pub fn events_for_peer(&self, heads: &[crate::ledger::AuthorHead]) -> Vec<SignedEvent> {
        self.ledger
            .missing_for(heads)
            .into_iter()
            .filter(|e| !self.is_pending_event(e))
            .collect()
    }

    fn is_pending_event(&self, e: &SignedEvent) -> bool {
        matches!(&e.body.kind, EventKind::Add { device, .. } if Some(*device) == self.pending_add)
    }

    /// Deletes this device's key after it has been removed from the circle.
    ///
    /// # Errors
    /// Returns an error if the files cannot be removed.
    pub fn forget(self) -> Result<(), CircleError> {
        fs::remove_file(self.dir.join(KEY_FILE))?;
        fs::remove_file(self.dir.join(CIRCLE_FILE))?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn secret(byte: u8) -> KeySealing {
        KeySealing::with_secret(Zeroizing::new([byte; 32]))
    }

    #[test]
    fn a_sealed_key_is_not_readable_on_disk_and_comes_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let sealing = secret(7);
        let circle = Circle::create_sealed(dir.path(), "mac", &sealing).unwrap();
        let id = circle.device_id();
        let seed = circle.key().to_bytes();
        drop(circle);
        let stored = fs::read(dir.path().join(KEY_FILE)).unwrap();
        assert_eq!(stored.len(), SEALED_LEN);
        assert!(!stored.windows(32).any(|w| w == seed));
        let reopened = Circle::load_sealed(dir.path(), &sealing).unwrap();
        assert_eq!(reopened.device_id(), id);
    }

    #[test]
    fn the_wrong_secret_opens_nothing() {
        let dir = tempfile::tempdir().unwrap();
        drop(Circle::create_sealed(dir.path(), "mac", &secret(7)).unwrap());
        let wrong = Circle::load_sealed(dir.path(), &secret(8));
        assert!(matches!(wrong, Err(CircleError::KeyLocked)));
    }

    #[test]
    fn a_sealed_key_needs_its_secret() {
        let dir = tempfile::tempdir().unwrap();
        drop(Circle::create_sealed(dir.path(), "mac", &secret(7)).unwrap());
        assert!(matches!(
            Circle::load(dir.path()),
            Err(CircleError::KeyLocked)
        ));
        assert!(matches!(
            Circle::snapshot(dir.path()),
            Err(CircleError::KeyLocked)
        ));
    }

    #[test]
    fn a_tampered_sealed_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let sealing = secret(7);
        drop(Circle::create_sealed(dir.path(), "mac", &sealing).unwrap());
        let path = dir.path().join(KEY_FILE);
        let original = fs::read(&path).unwrap();
        for at in [
            SEALED_MAGIC.len(),
            SEALED_MAGIC.len() + NONCE_LEN,
            SEALED_LEN - 1,
        ] {
            let mut changed = original.clone();
            *changed.get_mut(at).unwrap() ^= 1;
            fs::write(&path, &changed).unwrap();
            assert!(matches!(
                Circle::load_sealed(dir.path(), &sealing),
                Err(CircleError::KeyLocked)
            ));
        }
    }

    #[test]
    fn a_readable_key_is_sealed_when_a_secret_appears_and_stays_the_same_device() {
        let dir = tempfile::tempdir().unwrap();
        let circle = Circle::create(dir.path(), "mac").unwrap();
        let id = circle.device_id();
        let seed = circle.key().to_bytes();
        drop(circle);
        let path = dir.path().join(KEY_FILE);
        assert_eq!(fs::read(&path).unwrap(), seed);
        let sealing = secret(7);
        let migrated = Circle::load_sealed(dir.path(), &sealing).unwrap();
        assert_eq!(migrated.device_id(), id);
        drop(migrated);
        let stored = fs::read(&path).unwrap();
        assert!(is_sealed(&stored));
        assert!(!stored.windows(32).any(|w| w == seed));
        assert_eq!(
            Circle::load_sealed(dir.path(), &sealing)
                .unwrap()
                .device_id(),
            id
        );
    }

    #[test]
    fn a_snapshot_never_rewrites_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let circle = Circle::create(dir.path(), "mac").unwrap();
        let seed = circle.key().to_bytes();
        drop(circle);
        let path = dir.path().join(KEY_FILE);
        let lock = acquire(dir.path()).unwrap();
        let view = Circle::read(dir.path(), Access::Snapshot, &secret(7)).unwrap();
        drop(view);
        drop(lock);
        assert_eq!(fs::read(&path).unwrap(), seed);
    }

    #[test]
    fn a_key_of_the_wrong_shape_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        drop(Circle::create(dir.path(), "mac").unwrap());
        fs::write(dir.path().join(KEY_FILE), [1u8; 31]).unwrap();
        assert!(matches!(
            Circle::load_sealed(dir.path(), &secret(7)),
            Err(CircleError::Corrupt)
        ));
    }

    #[test]
    fn the_secret_never_reaches_debug_output() {
        let shown = format!("{:?}", secret(0x41));
        assert_eq!(shown, "KeySealing(sealed)");
    }

    #[test]
    fn a_sealed_key_is_private_to_its_owner() {
        let dir = tempfile::tempdir().unwrap();
        drop(Circle::create_sealed(dir.path(), "mac", &secret(7)).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
    }

    #[test]
    fn saves_and_reloads_a_circle() {
        let dir = tempfile::tempdir().unwrap();
        let created = Circle::create(dir.path(), "laptop").unwrap();
        let (id, ledger) = (created.device_id(), created.ledger().clone());
        drop(created);
        let loaded = Circle::load(dir.path()).unwrap();
        assert_eq!(id, loaded.device_id());
        assert_eq!(&ledger, loaded.ledger());
    }

    fn test_key(i: u32) -> SigningKey {
        let mut seed = [0x11u8; 32];
        seed[..4].copy_from_slice(&i.to_le_bytes());
        SigningKey::from_bytes(&seed)
    }

    fn add_member(circle: &mut Circle, id: DeviceId) {
        circle.add_pending(id, "device", Platform::Other).unwrap();
        circle.confirm_pending().unwrap();
    }

    fn discovery_key(generation: u32, byte: u8) -> DiscoveryKey {
        DiscoveryKey {
            generation,
            key: [byte; 32],
        }
    }

    #[test]
    fn a_discovery_key_is_kept_only_from_a_member_and_only_if_newer() {
        let dir = tempfile::tempdir().unwrap();
        let mut circle = Circle::create(dir.path(), "laptop").unwrap();
        let member = test_key(1).verifying_key().to_bytes();
        let stranger = test_key(2).verifying_key().to_bytes();
        add_member(&mut circle, member);

        assert!(
            !circle
                .learn_discovery_key(stranger, discovery_key(9, 1))
                .unwrap()
        );
        assert!(
            circle
                .learn_discovery_key(member, discovery_key(2, 2))
                .unwrap()
        );
        assert!(
            !circle
                .learn_discovery_key(member, discovery_key(2, 3))
                .unwrap()
        );
        assert!(
            !circle
                .learn_discovery_key(member, discovery_key(1, 4))
                .unwrap()
        );
        assert_eq!(circle.peer_discovery_keys().get(&member), Some(&[2u8; 32]));
        assert_eq!(circle.peer_discovery_keys().len(), 1);
    }

    #[test]
    fn removing_a_device_changes_the_key_it_knew() {
        let dir = tempfile::tempdir().unwrap();
        let mut circle = Circle::create(dir.path(), "laptop").unwrap();
        let member = test_key(1).verifying_key().to_bytes();
        add_member(&mut circle, member);
        circle
            .learn_discovery_key(member, discovery_key(1, 2))
            .unwrap();
        let before = circle.own_discovery_key().clone();

        circle.revoke(member).unwrap();
        let after = circle.own_discovery_key().clone();

        assert!(after.generation > before.generation);
        assert_ne!(after.key, before.key);
        assert!(circle.peer_discovery_keys().is_empty());
    }

    #[test]
    fn discovery_keys_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut circle = Circle::create(dir.path(), "laptop").unwrap();
        let member = test_key(1).verifying_key().to_bytes();
        add_member(&mut circle, member);
        circle
            .learn_discovery_key(member, discovery_key(3, 5))
            .unwrap();
        let own = circle.own_discovery_key().clone();
        drop(circle);

        let loaded = Circle::load(dir.path()).unwrap();
        assert_eq!(loaded.own_discovery_key(), &own);
        assert_eq!(loaded.peer_discovery_keys().get(&member), Some(&[5u8; 32]));
    }

    #[test]
    fn a_refused_event_uses_up_no_sequence_number() {
        let dir = tempfile::tempdir().unwrap();
        let mut circle = Circle::create(dir.path(), "laptop").unwrap();
        let target = test_key(1).verifying_key().to_bytes();
        add_member(&mut circle, target);

        let too_long = "x".repeat(64);
        let newcomer = test_key(2).verifying_key().to_bytes();
        assert!(
            circle
                .add_pending(newcomer, &too_long, Platform::Other)
                .is_err()
        );
        assert_eq!(circle.pending_add, None);

        circle.revoke(target).unwrap();
        assert!(!circle.is_member(&target));
    }

    #[test]
    fn signing_saves_the_event_and_its_number_together() {
        let dir = tempfile::tempdir().unwrap();
        let mut circle = Circle::create(dir.path(), "laptop").unwrap();
        let target = test_key(1).verifying_key().to_bytes();
        add_member(&mut circle, target);
        circle.revoke(target).unwrap();
        let expected = circle.ledger().clone();
        drop(circle);

        let mut reopened = Circle::load(dir.path()).unwrap();
        assert_eq!(reopened.ledger(), &expected);
        let other = test_key(2).verifying_key().to_bytes();
        add_member(&mut reopened, other);
        assert!(reopened.is_member(&other));
    }

    fn unknown_author_event(i: u32, circle: &Circle) -> SignedEvent {
        sign_event(
            &test_key(1000 + i),
            circle.ledger().circle_id(),
            1,
            EventKind::Add {
                device: [1; 32],
                name: String::new(),
                platform: Platform::Other,
            },
        )
        .unwrap()
    }

    #[test]
    fn a_flood_from_unknown_authors_cannot_block_a_removal() {
        let dir = tempfile::tempdir().unwrap();
        let mut circle = Circle::create(dir.path(), "laptop").unwrap();
        let (bad, good) = (test_key(1), test_key(2));
        add_member(&mut circle, bad.verifying_key().to_bytes());
        add_member(&mut circle, good.verifying_key().to_bytes());
        let held = u32::try_from(circle.ledger().all_events().count()).unwrap();
        let cap = u32::try_from(crate::ledger::MAX_EVENTS).unwrap();
        let flood: Vec<_> = (0..cap - held)
            .map(|i| unknown_author_event(i, &circle))
            .collect();
        circle.apply(flood).unwrap();

        let revoke = sign_event(
            &good,
            circle.ledger().circle_id(),
            1,
            EventKind::Revoke {
                device: bad.verifying_key().to_bytes(),
                last_seen_seq: 0,
            },
        )
        .unwrap();
        let batch = vec![unknown_author_event(100_000, &circle), revoke];
        circle.apply(batch).unwrap();
        assert!(!circle.is_member(&bad.verifying_key().to_bytes()));
    }

    #[test]
    fn events_from_unknown_authors_make_way_for_a_member() {
        let dir = tempfile::tempdir().unwrap();
        let mut circle = Circle::create(dir.path(), "laptop").unwrap();
        let held = u32::try_from(circle.ledger().all_events().count()).unwrap();
        let cap = u32::try_from(crate::ledger::MAX_EVENTS).unwrap();
        let flood: Vec<_> = (0..cap - held)
            .map(|i| unknown_author_event(i, &circle))
            .collect();
        circle.apply(flood).unwrap();

        let newcomer = test_key(1).verifying_key().to_bytes();
        add_member(&mut circle, newcomer);
        assert!(circle.is_member(&newcomer));
    }

    fn circle_with_a_member(dir: &Path) -> (Circle, SigningKey, SigningKey) {
        let (owner, member) = (test_key(1), test_key(3));
        let mut ledger = Ledger::create(&owner, [7; 32], "owner".to_string()).unwrap();
        let id = *ledger.circle_id();
        let add = |device, name: &str| EventKind::Add {
            device,
            name: name.to_string(),
            platform: Platform::Other,
        };
        let member_id = member.verifying_key().to_bytes();
        ledger
            .insert(sign_event(&owner, &id, 2, add(member_id, "m")).unwrap())
            .unwrap();
        let mine = test_key(77);
        let mine_id = mine.verifying_key().to_bytes();
        ledger
            .insert(sign_event(&owner, &id, 3, add(mine_id, "v")).unwrap())
            .unwrap();
        (Circle::adopt(dir, mine, ledger, 1).unwrap(), owner, member)
    }

    #[test]
    fn a_member_cannot_block_its_own_removal_by_flooding_revokes() {
        let dir = tempfile::tempdir().unwrap();
        let (mut circle, owner, member) = circle_with_a_member(dir.path());
        let id = *circle.ledger().circle_id();
        let flood: Vec<_> = (1..=u32::try_from(crate::ledger::MAX_REVOKES).unwrap())
            .map(|s| {
                let kind = EventKind::Revoke {
                    device: [(s % 251) as u8 | 1; 32],
                    last_seen_seq: 0,
                };
                sign_event(&member, &id, s, kind).unwrap()
            })
            .collect();
        circle.apply(flood).unwrap();

        let member_id = member.verifying_key().to_bytes();
        let kind = EventKind::Revoke {
            device: member_id,
            last_seen_seq: 0,
        };
        circle
            .apply(vec![sign_event(&owner, &id, 4, kind).unwrap()])
            .unwrap();
        assert!(!circle.is_member(&member_id));
    }

    #[test]
    fn a_member_cannot_strand_a_removal_behind_a_refused_event() {
        let dir = tempfile::tempdir().unwrap();
        let (mut circle, owner, member) = circle_with_a_member(dir.path());
        let id = *circle.ledger().circle_id();
        let flood: Vec<_> = (1..=u32::try_from(crate::ledger::MAX_EVENTS).unwrap())
            .map(|s| {
                let kind = EventKind::Add {
                    device: [(s % 251) as u8 + 1; 32],
                    name: String::new(),
                    platform: Platform::Other,
                };
                sign_event(&member, &id, s, kind).unwrap()
            })
            .collect();
        circle.apply(flood).unwrap();

        let member_id = member.verifying_key().to_bytes();
        let newcomer = test_key(50).verifying_key().to_bytes();
        let add = EventKind::Add {
            device: newcomer,
            name: String::new(),
            platform: Platform::Other,
        };
        let revoke = EventKind::Revoke {
            device: member_id,
            last_seen_seq: 0,
        };
        let batch = vec![
            sign_event(&owner, &id, 4, add).unwrap(),
            sign_event(&owner, &id, 5, revoke).unwrap(),
        ];
        circle.apply(batch).unwrap();
        assert!(!circle.is_member(&member_id));
        assert!(circle.is_member(&newcomer));
    }

    #[test]
    fn a_second_writer_is_refused_while_the_first_is_running() {
        let dir = tempfile::tempdir().unwrap();
        let _owner = Circle::create(dir.path(), "laptop").unwrap();
        assert!(matches!(Circle::load(dir.path()), Err(CircleError::InUse)));
    }

    #[test]
    fn a_snapshot_cannot_sign_or_save() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Circle::create(dir.path(), "laptop").unwrap();
        let mut view = Circle::snapshot(dir.path()).unwrap();
        assert!(matches!(
            view.revoke(owner.device_id()),
            Err(CircleError::ReadOnly)
        ));
        let on_disk = Circle::snapshot(dir.path()).unwrap();
        assert_eq!(on_disk.ledger(), owner.ledger());
    }

    #[test]
    fn open_falls_back_to_a_snapshot_when_the_state_is_owned() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Circle::create(dir.path(), "laptop").unwrap();
        let view = Circle::open(dir.path()).unwrap();
        assert!(matches!(view.access, Access::Snapshot));
        drop((owner, view));
        assert!(matches!(
            Circle::open(dir.path()).unwrap().access,
            Access::Owner(_)
        ));
    }

    #[test]
    fn reading_a_snapshot_does_not_abandon_a_pairing_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let mut owner = Circle::create(dir.path(), "laptop").unwrap();
        owner
            .add_pending([7; 32], "phone", Platform::Android)
            .unwrap();
        let view = Circle::snapshot(dir.path()).unwrap();
        assert!(view.is_member(&[7; 32]));
        assert!(owner.is_member(&[7; 32]));
    }

    #[cfg(unix)]
    #[test]
    fn keeps_the_key_file_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        Circle::create(dir.path(), "laptop").unwrap();
        let mode = fs::metadata(dir.path().join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_pending_add_is_revoked_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut circle = Circle::create(dir.path(), "laptop").unwrap();
        let ghost = SigningKey::from_bytes(&[3; 32]).verifying_key().to_bytes();
        circle.add_pending(ghost, "phone", Platform::Ios).unwrap();
        drop(circle);
        let loaded = Circle::load(dir.path()).unwrap();
        assert!(!loaded.is_member(&ghost));
        assert!(loaded.ledger().membership().revoked.contains(&ghost));
    }

    #[test]
    fn a_pending_add_is_not_offered_to_peers() {
        let dir = tempfile::tempdir().unwrap();
        let mut circle = Circle::create(dir.path(), "laptop").unwrap();
        let ghost = SigningKey::from_bytes(&[3; 32]).verifying_key().to_bytes();
        circle.add_pending(ghost, "phone", Platform::Ios).unwrap();
        let sent = circle.events_for_peer(&[]);
        assert!(
            sent.iter()
                .all(|e| !matches!(e.body.kind, EventKind::Add { .. }))
        );
        circle.confirm_pending().unwrap();
        let sent = circle.events_for_peer(&[]);
        assert!(
            sent.iter()
                .any(|e| matches!(e.body.kind, EventKind::Add { .. }))
        );
    }

    #[test]
    fn an_own_event_from_the_future_stops_signing() {
        let dir = tempfile::tempdir().unwrap();
        let mut circle = Circle::create(dir.path(), "laptop").unwrap();
        let future = sign_event(
            circle.key(),
            circle.ledger().circle_id(),
            5,
            EventKind::Add {
                device: [1; 32],
                name: String::new(),
                platform: Platform::Other,
            },
        )
        .unwrap();
        assert!(circle.apply(vec![future]).unwrap());
        assert!(matches!(
            circle.revoke([9; 32]),
            Err(CircleError::NoSuchDevice)
        ));
        let ghost = SigningKey::from_bytes(&[3; 32]).verifying_key().to_bytes();
        assert!(matches!(
            circle.add_pending(ghost, "x", Platform::Other),
            Err(CircleError::StateLost)
        ));
    }

    #[test]
    fn never_reuses_a_sequence_number() {
        let dir = tempfile::tempdir().unwrap();
        let mut circle = Circle::create(dir.path(), "laptop").unwrap();
        let a = SigningKey::from_bytes(&[3; 32]).verifying_key().to_bytes();
        circle.add_pending(a, "a", Platform::Other).unwrap();
        circle.confirm_pending().unwrap();
        circle.revoke(a).unwrap();
        let view = circle.ledger().membership();
        assert!(view.revoked.contains(&a));
        assert!(
            view.revoked.len() == 1,
            "the device must not have revoked itself"
        );
    }
}
