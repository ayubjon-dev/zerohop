//! Safely writing received files to disk (protocol sections 10 and 11).
//!
//! Everything is written inside a private tree opened through a capability
//! handle, so a path cannot leave it. A top-level item appears in the receive
//! folder only after every file in it has been verified, and nothing that
//! already exists is ever overwritten or merged into.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use serde::{Deserialize, Serialize};

use crate::ledger::DeviceId;
use crate::names::{self, NameError, TMP_DIR_NAME, TargetOs};
use crate::wire::{Item, ItemKind};

/// Size of one file segment.
#[cfg(not(fuzzing))]
pub const SEGMENT_SIZE: u64 = 16 * 1024 * 1024;
/// Tiny under the fuzzer, so that short inputs can span several segments.
#[cfg(fuzzing)]
pub const SEGMENT_SIZE: u64 = 32;
/// Most items in one offer.
pub const MAX_ITEMS: u32 = 200_000;
/// Most encoded manifest bytes in one offer.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024 * 1024;
/// Most decoded manifest bytes held in memory.
pub const MAX_MANIFEST_MEMORY: usize = 32 * 1024 * 1024;
/// Space that must stay free on the receiving disk.
pub const RESERVE_BYTES: u64 = 1 << 30;

const FILE_ROOT_PREFIX: &[u8] = b"zerohop-v1/file\0";
const FILES_DIR: &str = "files";
const TRANSFER_LOCK_FILE: &str = "lock";
/// Long enough for a replaced connection's queue to drain.
const TRANSFER_LOCK_WAIT: Duration = if cfg!(any(test, fuzzing)) {
    Duration::from_millis(60)
} else {
    Duration::from_secs(5)
};
const HEADER_FILE: &str = "header.bin";
const LOG_FILE: &str = "journal.log";
const SPACE_CHECK_INTERVAL: u64 = 64 * 1024 * 1024;

/// Errors from storing a transfer.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// A disk operation failed.
    #[error("disk error: {0}")]
    Io(#[from] io::Error),
    /// The sender broke the protocol.
    #[error("invalid transfer data")]
    Protocol,
    /// A hash did not match.
    #[error("a file failed its integrity check")]
    Integrity,
    /// The disk is too full.
    #[error("not enough disk space")]
    NoSpace,
    /// A path in the offer was refused.
    #[error("invalid path in the offer: {0}")]
    InvalidPath(#[from] NameError),
    /// The manifest did not add up.
    #[error("inconsistent manifest")]
    BadManifest,
    /// No state exists for this transfer.
    #[error("unknown transfer")]
    Unknown,
    /// An earlier connection is still finishing its writes for this transfer.
    #[error("this transfer is still being written")]
    Busy,
}

/// An item after its path has passed name handling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeItem {
    /// The item's index in the offer.
    pub index: u32,
    /// The sanitized path components.
    pub path: Vec<String>,
    /// File or directory.
    pub kind: ItemKind,
    /// Size in bytes.
    pub size: u64,
    /// Modification time, if the sender supplied one.
    pub mtime_ms: Option<i64>,
    /// Creation time, if the sender supplied one.
    pub created_ms: Option<i64>,
}

/// A validated offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidManifest {
    /// Every item, in index order.
    pub items: Vec<SafeItem>,
    /// Total bytes, computed from the items.
    pub total_bytes: u64,
    /// Number of files.
    pub file_count: u32,
    /// Whether any path looks like a program or script.
    pub has_program: bool,
}

/// Validates a whole manifest. Totals are computed here, never taken on trust.
///
/// # Errors
/// Returns an error if any path is refused or the manifest is inconsistent.
pub fn validate_manifest(
    items: &[Item],
    declared_total: u64,
    os: TargetOs,
) -> Result<ValidManifest, StoreError> {
    let mut safe = Vec::with_capacity(items.len());
    let mut total: u64 = 0;
    let mut file_count: u32 = 0;
    let mut has_program = false;
    let mut seen = HashSet::with_capacity(items.len());
    let mut files = HashSet::new();
    for (position, item) in items.iter().enumerate() {
        if usize::try_from(item.index).ok() != Some(position) {
            return Err(StoreError::BadManifest);
        }
        let path = names::sanitize_path(&item.path, os)?;
        let key = names::fold_key(&path.join("/"));
        if !seen.insert(key.clone()) {
            return Err(StoreError::BadManifest);
        }
        match item.kind {
            ItemKind::Directory if item.size != 0 => return Err(StoreError::BadManifest),
            ItemKind::Directory => {}
            ItemKind::File => {
                total = total
                    .checked_add(item.size)
                    .ok_or(StoreError::BadManifest)?;
                file_count = file_count.checked_add(1).ok_or(StoreError::BadManifest)?;
                files.insert(key);
            }
        }
        has_program |= names::is_program_path(&path);
        safe.push(SafeItem {
            index: item.index,
            path,
            kind: item.kind,
            size: item.size,
            mtime_ms: item.mtime_ms,
            created_ms: item.created_ms,
        });
    }
    if total != declared_total {
        return Err(StoreError::BadManifest);
    }
    // Two spellings of one folder are the same folder on many disks, so one of
    // them could merge into, and publish, a folder that is being held back.
    let mut folders: HashMap<String, String> = HashMap::new();
    for item in &safe {
        let mut prefix = String::new();
        let last = item.path.len().saturating_sub(1);
        for (i, part) in item.path.iter().enumerate() {
            if i == last && item.kind == ItemKind::File {
                break;
            }
            if i > 0 {
                prefix.push('/');
            }
            prefix.push_str(part);
            let key = names::fold_key(&prefix);
            if i != last && files.contains(&key) {
                return Err(StoreError::BadManifest);
            }
            match folders.entry(key) {
                Entry::Occupied(known) if *known.get() != prefix => {
                    return Err(StoreError::BadManifest);
                }
                Entry::Occupied(_) => {}
                Entry::Vacant(slot) => {
                    slot.insert(prefix.clone());
                }
            }
        }
    }
    Ok(ValidManifest {
        items: safe,
        total_bytes: total,
        file_count,
        has_program,
    })
}

/// How many segments a file of this size has.
#[must_use]
pub const fn segment_count(size: u64) -> u64 {
    if size == 0 {
        1
    } else {
        size.div_ceil(SEGMENT_SIZE)
    }
}

/// The root hash of a file from its segment hashes.
#[must_use]
pub fn file_root(size: u64, hashes: &[[u8; 32]]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(FILE_ROOT_PREFIX);
    h.update(&size.to_le_bytes());
    for s in hashes {
        h.update(s);
    }
    *h.finalize().as_bytes()
}

/// The digest of a file's completed segment hashes, used when resuming.
#[must_use]
pub fn prefix_digest(hashes: &[[u8; 32]]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    for s in hashes {
        h.update(s);
    }
    *h.finalize().as_bytes()
}

#[cfg(target_os = "macos")]
fn mark_downloaded(file: &std::fs::File, _path: &Path) -> io::Result<()> {
    use xattr::FileExt;
    // The system marks a sandboxed app's files as they are created, and does
    // not let the app change that mark.
    if file.get_xattr("com.apple.quarantine")?.is_some() {
        return Ok(());
    }
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    file.set_xattr(
        "com.apple.quarantine",
        format!("0083;{now:x};Zerohop;").as_bytes(),
    )
}

#[cfg(windows)]
fn mark_downloaded(_file: &std::fs::File, path: &Path) -> io::Result<()> {
    let mut stream = path.as_os_str().to_os_string();
    stream.push(":Zone.Identifier");
    std::fs::write(stream, b"[ZoneTransfer]\r\nZoneId=3\r\n")
}

// iOS has no download mark: nothing there runs a file because of where it came from.
#[cfg(not(any(target_os = "macos", windows)))]
#[allow(clippy::unnecessary_wraps)]
const fn mark_downloaded(_file: &std::fs::File, _path: &Path) -> io::Result<()> {
    Ok(())
}

/// The folder received files are published into.
#[derive(Clone, Debug)]
pub struct ReceiveFolder {
    path: PathBuf,
    os: TargetOs,
}

impl ReceiveFolder {
    /// Uses a folder, creating it if needed.
    ///
    /// # Errors
    /// Returns an error if the folder cannot be created.
    pub fn open(path: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            os: TargetOs::current(),
        })
    }

    /// Uses a folder without creating it. Offers fail later if it is missing.
    #[must_use]
    pub fn unchecked(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            os: TargetOs::current(),
        }
    }

    /// Where this folder is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The name conventions this folder follows.
    #[must_use]
    pub const fn os(&self) -> TargetOs {
        self.os
    }

    fn root(&self) -> io::Result<Dir> {
        Dir::open_ambient_dir(&self.path, ambient_authority())
    }

    /// Whether the free space is enough to accept this many bytes.
    ///
    /// # Errors
    /// Returns an error if free space cannot be read.
    pub fn has_room_for(&self, bytes: u64) -> io::Result<bool> {
        let free = fs4::available_space(&self.path)?;
        Ok(free >= bytes.saturating_add(RESERVE_BYTES))
    }

    /// Whether download marks can be stored on this volume.
    #[must_use]
    pub fn marks_supported(&self) -> bool {
        if !cfg!(any(target_os = "macos", windows)) {
            return false;
        }
        let probe = self.path.join(".zerohop-mark-probe");
        let result = std::fs::File::create(&probe)
            .and_then(|file| mark_downloaded(&file, &probe))
            .is_ok();
        let _ = std::fs::remove_file(&probe);
        result
    }
}

#[derive(Serialize, Deserialize)]
struct Header {
    peer: DeviceId,
    manifest: [u8; 32],
    hold_executables: bool,
    items: Vec<SafeItem>,
}

#[derive(Serialize, Deserialize)]
enum Record {
    Segment {
        file: u32,
        segment: u32,
        hash: [u8; 32],
    },
    FileDone {
        file: u32,
    },
    Truncate {
        file: u32,
        segments: u32,
    },
    Published {
        top: String,
    },
    Held {
        top: String,
    },
    /// A copy into the folder has started under this name.
    Copying {
        top: String,
        target: String,
    },
    /// The owner threw a held item away.
    Discarded {
        top: String,
    },
}

/// What a transfer's journal says has happened.
#[derive(Default)]
struct ParsedLog {
    hashes: BTreeMap<u32, Vec<[u8; 32]>>,
    done: BTreeSet<u32>,
    published: BTreeSet<String>,
    held: BTreeSet<String>,
    copying: Option<(String, String)>,
    /// Length of the part that could be read.
    intact: usize,
}

fn parse_log(log_bytes: &[u8]) -> Result<ParsedLog, StoreError> {
    let mut log = ParsedLog::default();
    let mut rest = log_bytes;
    while let Some((len_bytes, tail)) = rest.split_at_checked(4) {
        let len = u32::from_le_bytes(len_bytes.try_into().map_err(|_| StoreError::Unknown)?);
        // A torn final record just means slightly more is sent again.
        let Some((body, tail)) = tail.split_at_checked(len as usize) else {
            break;
        };
        let Ok(record) = postcard::from_bytes::<Record>(body) else {
            break;
        };
        match record {
            Record::Segment {
                file,
                segment,
                hash,
            } => {
                let list = log.hashes.entry(file).or_default();
                if segment as usize == list.len() {
                    list.push(hash);
                }
            }
            Record::FileDone { file } => {
                log.done.insert(file);
            }
            Record::Truncate { file, segments } => {
                log.hashes
                    .entry(file)
                    .or_default()
                    .truncate(segments as usize);
                log.done.remove(&file);
            }
            Record::Published { top } => {
                if log.copying.as_ref().is_some_and(|(t, _)| *t == top) {
                    log.copying = None;
                }
                log.published.insert(top);
            }
            Record::Copying { top, target } => {
                log.copying = Some((top, target));
            }
            Record::Held { top } => {
                log.held.insert(top);
            }
            Record::Discarded { top } => {
                log.held.remove(&top);
                log.published.insert(top);
            }
        }
        rest = tail;
    }
    log.intact = log_bytes.len() - rest.len();
    Ok(log)
}

fn read_header(dir: &Dir) -> Result<Header, StoreError> {
    postcard::from_bytes(&dir.read(HEADER_FILE).map_err(|_| StoreError::Unknown)?)
        .map_err(|_| StoreError::Unknown)
}

/// Something received that is waiting for the owner's decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldItem {
    /// The transfer it arrived in.
    pub transfer_id: [u8; 16],
    /// The device that sent it.
    pub peer: DeviceId,
    /// Its top-level name.
    pub name: String,
    /// Whether it is a folder.
    pub is_dir: bool,
    /// Sum of its file sizes.
    pub bytes: u64,
    /// When it arrived, in milliseconds since 1970.
    pub arrived_ms: i64,
}

fn transfer_id_from(name: &str) -> Option<[u8; 16]> {
    data_encoding::HEXLOWER
        .decode(name.as_bytes())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
}

/// Lists what is waiting for the owner. Reads journals only.
///
/// # Errors
/// Returns an error if the folder cannot be read.
pub fn list_held(folder: &ReceiveFolder) -> Result<Vec<HeldItem>, StoreError> {
    let root = folder.root()?;
    let Ok(tmp) = root.open_dir(TMP_DIR_NAME) else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for entry in tmp.entries()? {
        let Ok(dir_name) = entry?.file_name().into_string() else {
            continue;
        };
        let Some(transfer_id) = transfer_id_from(&dir_name) else {
            continue;
        };
        let Ok(dir) = tmp.open_dir(&dir_name) else {
            continue;
        };
        let (Ok(header), Ok(bytes)) = (read_header(&dir), dir.read(LOG_FILE)) else {
            continue;
        };
        let log = parse_log(&bytes)?;
        let arrived_ms = dir
            .metadata(LOG_FILE)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| {
                t.into_std()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .ok()
                    .and_then(|d| i64::try_from(d.as_millis()).ok())
            })
            .unwrap_or(0);
        for name in log.held.difference(&log.published) {
            let mut members = header
                .items
                .iter()
                .filter(|i| i.path.first() == Some(name))
                .peekable();
            if members.peek().is_none() {
                continue;
            }
            let mut complete = true;
            let mut size = 0u64;
            let mut is_dir = false;
            for item in members {
                is_dir |= item.path.len() > 1 || item.kind == ItemKind::Directory;
                if item.kind == ItemKind::File {
                    size = size.saturating_add(item.size);
                    complete &= log.done.contains(&item.index);
                }
            }
            if complete {
                found.push(HeldItem {
                    transfer_id,
                    peer: header.peer,
                    name: name.clone(),
                    is_dir,
                    bytes: size,
                    arrived_ms,
                });
            }
        }
    }
    Ok(found)
}

fn open_held(folder: &ReceiveFolder, transfer_id: &[u8; 16]) -> Result<TransferWriter, StoreError> {
    let root = folder.root()?;
    let dir = root
        .open_dir(format!("{TMP_DIR_NAME}/{}", hex(transfer_id)))
        .map_err(|_| StoreError::Unknown)?;
    let header = read_header(&dir)?;
    drop(dir);
    Ok(TransferWriter::resume(folder, transfer_id, header.peer, header.manifest)?.0)
}

/// Lets a held item into the folder. What is on disk is verified again first,
/// and the item keeps its download mark.
///
/// # Errors
/// Returns [`StoreError::Unknown`] if nothing complete is held under this name.
pub fn release_held(
    folder: &ReceiveFolder,
    transfer_id: &[u8; 16],
    name: &str,
) -> Result<(), StoreError> {
    let mut writer = open_held(folder, transfer_id)?;
    let top = writer.tops.get_mut(name).ok_or(StoreError::Unknown)?;
    if !top.held || top.published || top.remaining != 0 {
        return Err(StoreError::Unknown);
    }
    top.held = false;
    writer.settle_top(name)?;
    writer.close_if_settled()
}

/// Deletes a held item.
///
/// # Errors
/// Returns [`StoreError::Unknown`] if nothing is held under this name.
pub fn discard_held(
    folder: &ReceiveFolder,
    transfer_id: &[u8; 16],
    name: &str,
) -> Result<(), StoreError> {
    let mut writer = open_held(folder, transfer_id)?;
    let top = writer.tops.get_mut(name).ok_or(StoreError::Unknown)?;
    if !top.held || top.published {
        return Err(StoreError::Unknown);
    }
    top.held = false;
    top.published = true;
    let slot = top.slot.to_string();
    match writer.tree.remove_dir_all(&slot) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    writer.record(&Record::Discarded {
        top: name.to_string(),
    })?;
    writer.close_if_settled()
}

/// Deletes transfers untouched for longer than `max_age`, held items included,
/// leaving alone anything in use. Returns how many were removed.
///
/// # Errors
/// Returns an error if the folder cannot be read.
pub fn sweep_expired(folder: &ReceiveFolder, max_age: Duration) -> Result<u32, StoreError> {
    let root = folder.root()?;
    let Ok(tmp) = root.open_dir(TMP_DIR_NAME) else {
        return Ok(0);
    };
    let mut removed = 0;
    for entry in tmp.entries()? {
        let Ok(name) = entry?.file_name().into_string() else {
            continue;
        };
        let Ok(dir) = tmp.open_dir(&name) else {
            continue;
        };
        let touched = dir
            .metadata(LOG_FILE)
            .or_else(|_| tmp.metadata(&name))
            .and_then(|m| m.modified());
        let old = touched.is_ok_and(|t| t.into_std().elapsed().is_ok_and(|age| age > max_age));
        if !old {
            continue;
        }
        let mut options = OpenOptions::new();
        options.write(true).create(true);
        let Ok(lock) = dir.open_with(TRANSFER_LOCK_FILE, &options) else {
            continue;
        };
        let lock = lock.into_std();
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::Error(e))
                if e.kind() == io::ErrorKind::Unsupported
                    && !is_writing_here(&folder.path.join(TMP_DIR_NAME).join(&name)) => {}
            Err(_) => continue,
        }
        drop((lock, dir));
        if tmp.remove_dir_all(&name).is_ok() {
            removed += 1;
        }
    }
    let _ = root.remove_dir(TMP_DIR_NAME);
    Ok(removed)
}

/// What a writer can say about its transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterSummary {
    /// Number of files.
    pub files: u32,
    /// Total size.
    pub total_bytes: u64,
    /// Bytes already here and verified.
    pub already: u64,
    /// Names of the first few top-level items, safe to display.
    pub names: Vec<String>,
}

struct OpenFile {
    index: u32,
    file: Option<std::fs::File>,
    path: Vec<String>,
    size: u64,
    offset: u64,
    segment_bytes: u64,
    hasher: blake3::Hasher,
    hashes: Vec<[u8; 32]>,
    mtime_ms: Option<i64>,
    #[cfg_attr(
        not(any(target_vendor = "apple", windows)),
        expect(
            dead_code,
            reason = "only Apple systems and Windows can set a creation time"
        )
    )]
    created_ms: Option<i64>,
}

#[derive(Default)]
struct Top {
    /// Its numbered folder in the private tree.
    slot: u32,
    remaining: u32,
    is_dir: bool,
    held: bool,
    published: bool,
}

/// The state a sender needs to continue an interrupted transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumePoint {
    /// Index of the next file to send.
    pub next_file: u32,
    /// Next segment within that file.
    pub next_segment: u32,
    /// Digest of the completed segment hashes of that file.
    pub prefix: [u8; 32],
}

/// Final counts for a transfer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinishCounts {
    /// Top-level items now in the receive folder.
    pub published: u32,
    /// Top-level items waiting for the owner's approval.
    pub held: u32,
}

/// Writes one transfer. Runs on a blocking thread.
pub struct TransferWriter {
    /// Held for as long as this writer exists.
    lock: TransferLock,
    root: Dir,
    transfer_dir_name: String,
    tree: Dir,
    tree_path: PathBuf,
    log: BufWriter<std::fs::File>,
    items: Vec<SafeItem>,
    file_hashes: HashMap<u32, Vec<[u8; 32]>>,
    done: BTreeSet<u32>,
    tops: HashMap<String, Top>,
    existing: HashSet<String>,
    made_dirs: HashSet<Vec<String>>,
    made_slots: HashSet<u32>,
    current: Option<OpenFile>,
    last_index: Option<u32>,
    resumed_file: Option<u32>,
    hold_executables: bool,
    folder: ReceiveFolder,
    since_space_check: u64,
}

/// Whether this data is also reachable under another name.
#[cfg(unix)]
fn is_shared(file: &std::fs::File) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(file.metadata()?.nlink() > 1)
}

/// Link counts are not available here, so publishing relies on its journal order.
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
const fn is_shared(_file: &std::fs::File) -> io::Result<bool> {
    Ok(false)
}

/// Transfers this process is writing, for file systems without file locks.
static WRITING_HERE: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// Held by the only writer of a transfer, for as long as it exists.
struct TransferLock {
    _file: std::fs::File,
    claimed_here: Option<PathBuf>,
}

impl Drop for TransferLock {
    fn drop(&mut self) {
        if let Some(path) = &self.claimed_here {
            WRITING_HERE
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(path);
        }
    }
}

fn claim_here(path: &Path) -> bool {
    WRITING_HERE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(path.to_path_buf())
}

fn is_writing_here(path: &Path) -> bool {
    WRITING_HERE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(path)
}

/// Makes this writer the only one for a transfer.
///
/// A replaced connection's writer can still be draining its queue when the
/// same transfer resumes, and two writers could publish and write at once.
/// Android's shared storage has no file locks. Only one process writes there,
/// so a claim inside this process does the same job.
fn lock_transfer(dir: &Dir, path: &Path) -> Result<TransferLock, StoreError> {
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    let file = dir.open_with(TRANSFER_LOCK_FILE, &options)?.into_std();
    let deadline = Instant::now() + TRANSFER_LOCK_WAIT;
    let mut use_claim = false;
    loop {
        if use_claim && claim_here(path) {
            return Ok(TransferLock {
                _file: file,
                claimed_here: Some(path.to_path_buf()),
            });
        }
        if !use_claim {
            match file.try_lock() {
                Ok(()) => {
                    return Ok(TransferLock {
                        _file: file,
                        claimed_here: None,
                    });
                }
                Err(std::fs::TryLockError::WouldBlock) => {}
                Err(std::fs::TryLockError::Error(e)) if e.kind() == io::ErrorKind::Unsupported => {
                    use_claim = true;
                    continue;
                }
                Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
            }
        }
        if Instant::now() >= deadline {
            return Err(StoreError::Busy);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn starts_like_a_program(file: &mut std::fs::File) -> io::Result<bool> {
    file.seek(SeekFrom::Start(0))?;
    let mut head = Vec::with_capacity(4);
    Read::take(&mut *file, 4).read_to_end(&mut head)?;
    Ok(names::looks_executable(&head))
}

fn hex(id: &[u8; 16]) -> String {
    data_encoding::HEXLOWER.encode(id)
}

fn file_options_new() -> OpenOptions {
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    o
}

fn to_time(ms: i64) -> Option<SystemTime> {
    let d = Duration::from_millis(ms.unsigned_abs());
    if ms >= 0 {
        SystemTime::UNIX_EPOCH.checked_add(d)
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(d)
    }
}

fn build_tops(items: &[SafeItem]) -> HashMap<String, Top> {
    let mut tops: HashMap<String, Top> = HashMap::new();
    for item in items {
        let Some(first) = item.path.first() else {
            continue;
        };
        let slot = u32::try_from(tops.len()).unwrap_or(u32::MAX);
        let top = tops.entry(first.clone()).or_insert_with(|| Top {
            slot,
            ..Top::default()
        });
        top.is_dir |= item.path.len() > 1 || item.kind == ItemKind::Directory;
        if item.kind == ItemKind::File {
            top.remaining += 1;
        }
    }
    tops
}

fn read_existing(root: &Dir) -> io::Result<HashSet<String>> {
    let mut set = HashSet::new();
    for entry in root.entries()? {
        if let Ok(name) = entry?.file_name().into_string() {
            set.insert(names::fold_key(&name));
        }
    }
    Ok(set)
}

/// Whether a name cannot be used because something else already has it. In
/// Android's shared storage another app's file looks like a refusal instead.
fn is_taken(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::AlreadyExists
        || (cfg!(target_os = "android") && error.kind() == io::ErrorKind::PermissionDenied)
}

fn candidate(name: &str, n: u32) -> String {
    if n == 0 {
        return name.to_string();
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({n}).{ext}"),
        _ => format!("{name} ({n})"),
    }
}

impl TransferWriter {
    /// Starts a new transfer. Refuses if one with this ID already exists.
    ///
    /// # Errors
    /// Returns an error if the private tree cannot be created.
    pub fn begin(
        folder: &ReceiveFolder,
        transfer_id: &[u8; 16],
        peer: DeviceId,
        manifest: [u8; 32],
        valid: &ValidManifest,
        hold_executables: bool,
    ) -> Result<Self, StoreError> {
        let root = folder.root()?;
        root.create_dir_all(TMP_DIR_NAME)?;
        let name = hex(transfer_id);
        let transfer_rel = format!("{TMP_DIR_NAME}/{name}");
        root.create_dir(&transfer_rel)?;
        let dir = root.open_dir(&transfer_rel)?;
        let lock = lock_transfer(&dir, &folder.path.join(&transfer_rel))?;
        dir.create_dir(FILES_DIR)?;
        let header = Header {
            peer,
            manifest,
            hold_executables,
            items: valid.items.clone(),
        };
        let bytes = postcard::to_stdvec(&header).map_err(|_| StoreError::Protocol)?;
        dir.write(HEADER_FILE, bytes)?;
        let log = dir.open_with(LOG_FILE, &file_options_new())?.into_std();
        let tree = dir.open_dir(FILES_DIR)?;
        let tree_path = folder.path.join(&transfer_rel).join(FILES_DIR);
        let mut writer = Self {
            lock,
            existing: read_existing(&root)?,
            root,
            transfer_dir_name: name,
            tree,
            tree_path,
            log: BufWriter::with_capacity(64 * 1024, log),
            tops: build_tops(&valid.items),
            items: valid.items.clone(),
            file_hashes: HashMap::new(),
            done: BTreeSet::new(),
            made_dirs: HashSet::new(),
            made_slots: HashSet::new(),
            current: None,
            last_index: None,
            resumed_file: None,
            hold_executables,
            folder: folder.clone(),
            since_space_check: 0,
        };
        writer.make_directories()?;
        Ok(writer)
    }

    fn make_directories(&mut self) -> Result<(), StoreError> {
        let dirs: Vec<Vec<String>> = self
            .items
            .iter()
            .filter(|i| i.kind == ItemKind::Directory)
            .map(|i| i.path.clone())
            .collect();
        for path in dirs {
            self.ensure_dir(&path)?;
        }
        Ok(())
    }

    /// Where an item lives in the private tree. Every top-level item has its
    /// own numbered folder, so that two of them can never share one on disk,
    /// whatever the file system considers to be the same name.
    fn private_rel(&self, path: &[String]) -> Result<String, StoreError> {
        let top = path
            .first()
            .and_then(|t| self.tops.get(t))
            .ok_or(StoreError::Protocol)?;
        Ok(format!("{}/{}", top.slot, path.join("/")))
    }

    fn ensure_slot(&mut self, path: &[String]) -> Result<(), StoreError> {
        let slot = path
            .first()
            .and_then(|t| self.tops.get(t))
            .ok_or(StoreError::Protocol)?
            .slot;
        if self.made_slots.insert(slot) {
            match self.tree.create_dir(slot.to_string()) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// Creates the folders above an item in the private tree.
    fn ensure_parents(&mut self, path: &[String]) -> Result<(), StoreError> {
        self.ensure_slot(path)?;
        let (parents, _) = path.split_at(path.len().saturating_sub(1));
        self.ensure_dir(parents)
    }

    fn ensure_dir(&mut self, path: &[String]) -> Result<(), StoreError> {
        if !path.is_empty() {
            self.ensure_slot(path)?;
        }
        for depth in 1..=path.len() {
            let prefix = path.get(..depth).ok_or(StoreError::Protocol)?;
            if self.made_dirs.contains(prefix) {
                continue;
            }
            let rel = self.private_rel(prefix)?;
            match self.tree.create_dir(&rel) {
                Ok(()) => {
                    let handle = self.tree.open_dir(&rel)?.into_std_file();
                    mark_downloaded(&handle, &self.tree_path.join(&rel))?;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
            self.made_dirs.insert(prefix.to_vec());
        }
        Ok(())
    }

    fn record(&mut self, record: &Record) -> Result<(), StoreError> {
        let bytes = postcard::to_stdvec(record).map_err(|_| StoreError::Protocol)?;
        let len = u32::try_from(bytes.len()).map_err(|_| StoreError::Protocol)?;
        self.log.write_all(&len.to_le_bytes())?;
        self.log.write_all(&bytes)?;
        Ok(())
    }

    fn top_is_published(&self, path: &[String]) -> bool {
        path.first()
            .and_then(|t| self.tops.get(t))
            .is_some_and(|t| t.published)
    }

    fn open_current(&mut self, index: u32) -> Result<(), StoreError> {
        if self.current.as_ref().is_some_and(|c| c.index == index) {
            return Ok(());
        }
        if self.current.is_some() || self.last_index.is_some_and(|last| index <= last) {
            return Err(StoreError::Protocol);
        }
        let item = self
            .items
            .get(usize::try_from(index).map_err(|_| StoreError::Protocol)?)
            .filter(|i| i.kind == ItemKind::File)
            .cloned()
            .ok_or(StoreError::Protocol)?;
        let file = if self.top_is_published(&item.path) {
            None
        } else {
            self.ensure_parents(&item.path)?;
            let rel = self.private_rel(&item.path)?;
            // Never truncate in place: after a crash this name can still be
            // linked to a file that is already visible in the folder.
            let _ = self.tree.remove_file(&rel);
            let mut options = file_options_new();
            options.read(true);
            let file = self.tree.open_with(&rel, &options)?.into_std();
            mark_downloaded(&file, &self.tree_path.join(&rel))?;
            Some(file)
        };
        self.file_hashes.remove(&index);
        self.current = Some(OpenFile {
            index,
            file,
            path: item.path,
            size: item.size,
            offset: 0,
            segment_bytes: 0,
            hasher: blake3::Hasher::new(),
            hashes: Vec::new(),
            mtime_ms: item.mtime_ms,
            created_ms: item.created_ms,
        });
        Ok(())
    }

    /// Writes one chunk of a file.
    ///
    /// # Errors
    /// Returns an error for out-of-order data, data past the declared size,
    /// or a disk failure.
    pub fn write(&mut self, index: u32, offset: u64, data: &[u8]) -> Result<(), StoreError> {
        self.open_current(index)?;
        self.since_space_check += data.len() as u64;
        if self.since_space_check >= SPACE_CHECK_INTERVAL {
            self.since_space_check = 0;
            if !self.folder.has_room_for(0)? {
                return Err(StoreError::NoSpace);
            }
        }
        // A sender whose source no longer matches restarts that one file.
        if offset == 0 && self.resumed_file == Some(index) {
            self.restart_current()?;
        }
        self.resumed_file = None;
        let current = self.current.as_mut().ok_or(StoreError::Protocol)?;
        let end = offset
            .checked_add(data.len() as u64)
            .ok_or(StoreError::Protocol)?;
        if offset != current.offset || end > current.size {
            return Err(StoreError::Protocol);
        }
        if let Some(file) = current.file.as_mut() {
            file.write_all(data)?;
        }
        current.hasher.update(data);
        current.offset = end;
        current.segment_bytes += data.len() as u64;
        Ok(())
    }

    fn restart_current(&mut self) -> Result<(), StoreError> {
        let current = self.current.as_mut().ok_or(StoreError::Protocol)?;
        let index = current.index;
        if let Some(file) = current.file.as_mut() {
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
        }
        current.offset = 0;
        current.segment_bytes = 0;
        current.hasher.reset();
        current.hashes.clear();
        self.record(&Record::Truncate {
            file: index,
            segments: 0,
        })
    }

    /// Checks a finished segment against the sender's hash.
    ///
    /// # Errors
    /// Returns an error if the hash or the segment's length is wrong.
    pub fn segment_end(
        &mut self,
        index: u32,
        segment: u32,
        hash: [u8; 32],
    ) -> Result<(), StoreError> {
        self.open_current(index)?;
        let current = self.current.as_mut().ok_or(StoreError::Protocol)?;
        if u64::from(segment) != current.hashes.len() as u64
            || u64::from(segment) >= segment_count(current.size)
        {
            return Err(StoreError::Protocol);
        }
        let done_before = u64::from(segment) * SEGMENT_SIZE;
        let expected = SEGMENT_SIZE.min(current.size.saturating_sub(done_before));
        if current.segment_bytes != expected {
            return Err(StoreError::Protocol);
        }
        if *current.hasher.finalize().as_bytes() != hash {
            return Err(StoreError::Integrity);
        }
        current.hashes.push(hash);
        current.hasher.reset();
        current.segment_bytes = 0;
        self.record(&Record::Segment {
            file: index,
            segment,
            hash,
        })
    }

    /// Finishes a file, checks its root hash, and publishes its item if complete.
    ///
    /// # Errors
    /// Returns an error if the file is incomplete or the root hash is wrong.
    pub fn file_end(&mut self, index: u32, root: [u8; 32]) -> Result<(), StoreError> {
        self.open_current(index)?;
        let current = self.current.take().ok_or(StoreError::Protocol)?;
        if current.offset != current.size
            || current.hashes.len() as u64 != segment_count(current.size)
            || current.segment_bytes != 0
        {
            return Err(StoreError::Protocol);
        }
        if file_root(current.size, &current.hashes) != root {
            return Err(StoreError::Integrity);
        }
        let top_name = current.path.first().cloned().ok_or(StoreError::Protocol)?;
        if let Some(mut file) = current.file {
            // Checked here rather than as data arrives: a sender controls how
            // the first bytes are split across frames and can resume past them.
            // A file named as a program was shown to the owner as one. Any other
            // file in the same offer was approved as what its name claims.
            if self.hold_executables
                && !names::declared_as_program(&current.path)
                && starts_like_a_program(&mut file)?
            {
                if let Some(top) = self.tops.get_mut(&top_name) {
                    top.held = true;
                }
                self.record(&Record::Held {
                    top: top_name.clone(),
                })?;
            }
            let mut times = std::fs::FileTimes::new();
            if let Some(t) = current.mtime_ms.and_then(to_time) {
                times = times.set_modified(t);
            }
            #[cfg(target_vendor = "apple")]
            if let Some(t) = current.created_ms.and_then(to_time) {
                #[cfg(target_os = "ios")]
                use std::os::ios::fs::FileTimesExt;
                #[cfg(target_os = "macos")]
                use std::os::macos::fs::FileTimesExt;
                times = times.set_created(t);
            }
            #[cfg(windows)]
            if let Some(t) = current.created_ms.and_then(to_time) {
                use std::os::windows::fs::FileTimesExt;
                times = times.set_created(t);
            }
            file.set_times(times)?;
        }
        self.last_index = Some(index);
        self.done.insert(index);
        self.file_hashes.insert(index, current.hashes);
        self.record(&Record::FileDone { file: index })?;

        let ready = match self.tops.get_mut(&top_name) {
            Some(top) => {
                top.remaining = top.remaining.saturating_sub(1);
                top.remaining == 0 && !top.published
            }
            None => false,
        };
        if ready {
            self.settle_top(&top_name)?;
        }
        Ok(())
    }

    fn settle_top(&mut self, name: &str) -> Result<(), StoreError> {
        let Some(top) = self.tops.get(name) else {
            return Ok(());
        };
        if top.held {
            return Ok(());
        }
        let is_dir = top.is_dir;
        // On disk first, so that a crash once the data is visible in the folder
        // cannot make a later resume treat that data as unfinished.
        self.log.flush()?;
        self.publish(name, is_dir)?;
        if let Some(t) = self.tops.get_mut(name) {
            t.published = true;
        }
        self.record(&Record::Published {
            top: name.to_string(),
        })
    }

    fn publish(&mut self, name: &str, is_dir: bool) -> Result<(), StoreError> {
        for n in 0..10_000 {
            let target = candidate(name, n);
            let key = names::fold_key(&target);
            if self.existing.contains(&key) {
                continue;
            }
            let source = self.private_rel(std::slice::from_ref(&name.to_string()))?;
            let result = if is_dir {
                self.publish_dir(&source, &target)
            } else {
                self.publish_file(name, &source, &target)
            };
            match result {
                Ok(()) => {
                    self.existing.insert(key);
                    return Ok(());
                }
                Err(e) if is_taken(&e) => {
                    self.existing.insert(key);
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(StoreError::Io(io::Error::other("too many name collisions")))
    }

    fn publish_file(&mut self, top: &str, source: &str, target: &str) -> io::Result<()> {
        // A crash between the link and the removal leaves the work half done.
        if is_shared(&self.tree.open(source)?.into_std())? {
            return self.tree.remove_file(source);
        }
        // A hard link fails if the target exists, which a plain rename would not.
        match self.tree.hard_link(source, &self.root, target) {
            Ok(()) => self.tree.remove_file(source),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
            Err(_) => self.publish_by_copy(top, source, target),
        }
    }

    fn publish_by_copy(&mut self, top: &str, source: &str, target: &str) -> io::Result<()> {
        let mut out = self.root.open_with(target, &file_options_new())?.into_std();
        // The name is ours, because creating it was exclusive. Recorded before
        // any data goes in, so a crash cannot leave a partial file looking done.
        self.record(&Record::Copying {
            top: top.to_string(),
            target: target.to_string(),
        })
        .map_err(io::Error::other)?;
        self.log.flush()?;
        // Android's shared storage has no hard links. Moving the file over the
        // one just claimed is a single step and needs no second copy of the data.
        if self.tree.rename(source, &self.root, target).is_ok() {
            return Ok(());
        }
        let mut input = self.tree.open(source)?.into_std();
        let copied = mark_downloaded(&out, &self.folder.path.join(target))
            .and_then(|()| io::copy(&mut input, &mut out));
        if let Err(e) = copied {
            let _ = self.root.remove_file(target);
            return Err(e);
        }
        self.tree.remove_file(source)
    }

    /// Settles a copy into the folder that a crash interrupted.
    fn settle_interrupted_copy(&mut self, top: &str, target: &str) -> Result<(), StoreError> {
        let source = self.private_rel(std::slice::from_ref(&top.to_string()))?;
        if self.tree.metadata(&source).is_ok() {
            // The copy may be partial, and the original is still here to redo it.
            match self.root.remove_file(target) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            self.existing.remove(&names::fold_key(target));
            return Ok(());
        }
        // The original is removed only once the copy is complete.
        if let Some(t) = self.tops.get_mut(top) {
            t.published = true;
        }
        self.record(&Record::Published {
            top: top.to_string(),
        })
    }

    #[cfg(not(windows))]
    fn publish_dir(&self, source: &str, target: &str) -> io::Result<()> {
        // Claim the name first: rename then replaces only the empty directory
        // made here, and fails if anything else has appeared inside it.
        self.root.create_dir(target)?;
        let moved = self.tree.rename(source, &self.root, target);
        // Android's shared storage may refuse to replace even an empty folder.
        // Only this app can write there, and a rename still cannot merge.
        #[cfg(target_os = "android")]
        let moved = moved.or_else(|_| {
            self.root.remove_dir(target)?;
            self.tree.rename(source, &self.root, target)
        });
        moved.inspect_err(|_| {
            let _ = self.root.remove_dir(target);
        })
    }

    #[cfg(windows)]
    fn publish_dir(&self, source: &str, target: &str) -> io::Result<()> {
        // On Windows, renaming onto an existing directory fails by itself.
        if self.root.metadata(target).is_ok() {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        self.tree.rename(source, &self.root, target)
    }

    /// Finishes the transfer once every file has arrived.
    ///
    /// # Errors
    /// Returns an error if any file is still missing.
    pub fn finish(mut self) -> Result<FinishCounts, StoreError> {
        let missing = self
            .items
            .iter()
            .any(|i| i.kind == ItemKind::File && !self.done.contains(&i.index));
        if missing || self.current.is_some() {
            return Err(StoreError::Protocol);
        }
        let empty_tops: Vec<String> = self
            .tops
            .iter()
            .filter(|(_, t)| t.remaining == 0 && !t.published && !t.held)
            .map(|(n, _)| n.clone())
            .collect();
        for name in empty_tops {
            self.settle_top(&name)?;
        }
        self.log.flush()?;
        let count = |wanted: fn(&Top) -> bool| {
            u32::try_from(self.tops.values().filter(|t| wanted(t)).count()).unwrap_or(u32::MAX)
        };
        let counts = FinishCounts {
            published: count(|t| t.published),
            held: count(|t| t.held && !t.published),
        };
        if counts.held == 0 {
            let Self {
                root,
                transfer_dir_name,
                log,
                lock,
                tree,
                ..
            } = self;
            drop((log, lock, tree));
            root.remove_dir_all(format!("{TMP_DIR_NAME}/{transfer_dir_name}"))?;
            let _ = root.remove_dir(TMP_DIR_NAME);
        }
        Ok(counts)
    }

    /// Discards all unpublished data for this transfer.
    ///
    /// # Errors
    /// Returns an error if the data cannot be removed.
    pub fn cancel(self) -> Result<(), StoreError> {
        let Self {
            root,
            transfer_dir_name,
            log,
            lock,
            tree,
            ..
        } = self;
        drop((log, lock, tree));
        root.remove_dir_all(format!("{TMP_DIR_NAME}/{transfer_dir_name}"))?;
        let _ = root.remove_dir(TMP_DIR_NAME);
        Ok(())
    }

    /// Reopens an interrupted transfer and verifies what is on disk.
    ///
    /// # Errors
    /// Returns [`StoreError::Unknown`] if the transfer does not exist, belongs
    /// to another peer, or was approved with a different manifest.
    pub fn resume(
        folder: &ReceiveFolder,
        transfer_id: &[u8; 16],
        peer: DeviceId,
        manifest: [u8; 32],
    ) -> Result<(Self, ResumePoint), StoreError> {
        let root = folder.root()?;
        let name = hex(transfer_id);
        let rel = format!("{TMP_DIR_NAME}/{name}");
        let dir = root.open_dir(&rel).map_err(|_| StoreError::Unknown)?;
        let lock = lock_transfer(&dir, &folder.path.join(&rel))?;
        let header = read_header(&dir)?;
        if header.peer != peer || header.manifest != manifest {
            return Err(StoreError::Unknown);
        }

        let log_bytes = dir.read(LOG_FILE).map_err(|_| StoreError::Unknown)?;
        let ParsedLog {
            hashes,
            done,
            published,
            held,
            copying,
            intact,
        } = parse_log(&log_bytes)?;

        let log = {
            let mut o = OpenOptions::new();
            o.write(true);
            let mut file = dir.open_with(LOG_FILE, &o)?.into_std();
            // A record torn by a crash must not stay in front of later ones.
            file.set_len(intact as u64)?;
            file.seek(SeekFrom::Start(intact as u64))?;
            file
        };
        let tree = dir.open_dir(FILES_DIR)?;
        let mut tops = build_tops(&header.items);
        for item in header.items.iter().filter(|i| i.kind == ItemKind::File) {
            if !done.contains(&item.index) {
                continue;
            }
            if let Some(t) = item.path.first().and_then(|p| tops.get_mut(p)) {
                t.remaining = t.remaining.saturating_sub(1);
            }
        }
        for name in &published {
            if let Some(t) = tops.get_mut(name) {
                t.published = true;
            }
        }
        for name in &held {
            if let Some(t) = tops.get_mut(name) {
                t.held = true;
            }
        }

        let mut writer = Self {
            lock,
            existing: read_existing(&root)?,
            root,
            transfer_dir_name: name,
            tree,
            tree_path: folder.path.join(&rel).join(FILES_DIR),
            log: BufWriter::with_capacity(64 * 1024, log),
            tops,
            items: header.items,
            file_hashes: hashes.clone().into_iter().collect(),
            done: done.clone(),
            made_dirs: HashSet::new(),
            made_slots: HashSet::new(),
            current: None,
            last_index: None,
            resumed_file: None,
            hold_executables: header.hold_executables,
            folder: folder.clone(),
            since_space_check: 0,
        };
        if let Some((top, target)) = copying {
            writer.settle_interrupted_copy(&top, &target)?;
        }
        let point = writer.verify_on_disk(&hashes, &done)?;
        Ok((writer, point))
    }

    /// Re-hashes data on disk, because a lost write must be detected here.
    fn verify_on_disk(
        &mut self,
        hashes: &BTreeMap<u32, Vec<[u8; 32]>>,
        done: &BTreeSet<u32>,
    ) -> Result<ResumePoint, StoreError> {
        let files: Vec<SafeItem> = self
            .items
            .iter()
            .filter(|i| i.kind == ItemKind::File)
            .cloned()
            .collect();
        for item in &files {
            if self.top_is_published(&item.path) {
                continue;
            }
            let recorded = hashes.get(&item.index).map_or(&[][..], Vec::as_slice);
            let good = self.count_good_segments(item, recorded);
            let complete = done.contains(&item.index) && good as u64 == segment_count(item.size);
            if complete {
                continue;
            }
            return self.rewind_to(item, recorded, good);
        }
        let next_file = self.items.last().map_or(0, |i| i.index + 1);
        Ok(ResumePoint {
            next_file,
            next_segment: 0,
            prefix: prefix_digest(&[]),
        })
    }

    fn count_good_segments(&self, item: &SafeItem, recorded: &[[u8; 32]]) -> usize {
        let Ok(rel) = self.private_rel(&item.path) else {
            return 0;
        };
        let Ok(mut file) = self.tree.open(rel).map(cap_std::fs::File::into_std) else {
            return 0;
        };
        let mut buf = vec![0u8; 1 << 20];
        let mut good = 0;
        for (i, expected) in recorded.iter().enumerate() {
            let want = SEGMENT_SIZE.min(item.size.saturating_sub(i as u64 * SEGMENT_SIZE));
            let mut left = want;
            let mut hasher = blake3::Hasher::new();
            while left > 0 {
                let take = usize::try_from(left.min(1 << 20)).unwrap_or(1 << 20);
                let Some(slice) = buf.get_mut(..take) else {
                    return good;
                };
                if file.read_exact(slice).is_err() {
                    return good;
                }
                hasher.update(slice);
                left -= take as u64;
            }
            if hasher.finalize().as_bytes() != expected {
                return good;
            }
            good += 1;
        }
        good
    }

    fn rewind_to(
        &mut self,
        item: &SafeItem,
        recorded: &[[u8; 32]],
        good: usize,
    ) -> Result<ResumePoint, StoreError> {
        // With every segment present but the file not closed, the last one is
        // sent again so that the root hash is still checked. Its end may be
        // short, so resuming past it would seek beyond the end of the file.
        let good = good.min(usize::try_from(segment_count(item.size) - 1).unwrap_or(usize::MAX));
        self.ensure_parents(&item.path)?;
        let rel = self.private_rel(&item.path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        let mut file = self.tree.open_with(&rel, &options)?.into_std();
        // Data still linked into the folder is already visible, so it is left
        // alone and this file starts over under a fresh name.
        let good = if is_shared(&file)? {
            self.tree.remove_file(&rel)?;
            file = self.tree.open_with(&rel, &options)?.into_std();
            0
        } else {
            good
        };
        mark_downloaded(&file, &self.tree_path.join(&rel))?;
        let segments = u32::try_from(good).map_err(|_| StoreError::Protocol)?;
        let kept = recorded.get(..good).ok_or(StoreError::Protocol)?.to_vec();
        self.record(&Record::Truncate {
            file: item.index,
            segments,
        })?;
        let offset = u64::from(segments) * SEGMENT_SIZE;
        file.set_len(offset)?;
        file.seek(SeekFrom::Start(offset))?;
        for index in item.index..=self.items.last().map_or(item.index, |i| i.index) {
            self.done.remove(&index);
        }
        self.tops = {
            let mut tops = build_tops(&self.items);
            for (name, top) in &mut tops {
                if let Some(old) = self.tops.get(name) {
                    top.published = old.published;
                    top.held = old.held;
                }
            }
            for done in self.items.iter().filter(|i| self.done.contains(&i.index)) {
                if let Some(t) = done.path.first().and_then(|p| tops.get_mut(p)) {
                    t.remaining = t.remaining.saturating_sub(1);
                }
            }
            tops
        };
        let prefix = prefix_digest(&kept);
        self.last_index = item.index.checked_sub(1);
        self.resumed_file = Some(item.index);
        self.current = Some(OpenFile {
            index: item.index,
            file: Some(file),
            path: item.path.clone(),
            size: item.size,
            offset,
            segment_bytes: 0,
            hasher: blake3::Hasher::new(),
            hashes: kept,
            mtime_ms: item.mtime_ms,
            created_ms: item.created_ms,
        });
        Ok(ResumePoint {
            next_file: item.index,
            next_segment: segments,
            prefix,
        })
    }

    /// Flushes the journal so an interrupted transfer can continue.
    ///
    /// # Errors
    /// Returns an error if the journal cannot be written.
    pub fn checkpoint(&mut self) -> Result<(), StoreError> {
        self.log.flush()?;
        Ok(())
    }

    /// Describes the transfer for the owner.
    #[must_use]
    pub fn summary(&self) -> WriterSummary {
        let mut names = Vec::new();
        let (mut files, mut total_bytes, mut already) = (0u32, 0u64, 0u64);
        for item in &self.items {
            if let Some(first) = item.path.first()
                && names.len() < 5
                && !names.contains(first)
            {
                names.push(first.clone());
            }
            if item.kind != ItemKind::File {
                continue;
            }
            files = files.saturating_add(1);
            total_bytes = total_bytes.saturating_add(item.size);
            if self.done.contains(&item.index) || self.top_is_published(&item.path) {
                already = already.saturating_add(item.size);
            } else if let Some(current) = self.current.as_ref()
                && current.index == item.index
            {
                already = already.saturating_add(current.offset);
            }
        }
        WriterSummary {
            files,
            total_bytes,
            already,
            names: names.iter().map(|n| names::display_name(n)).collect(),
        }
    }

    /// Removes the private tree once nothing in it is still needed.
    fn close_if_settled(mut self) -> Result<(), StoreError> {
        let settled = self.current.is_none()
            && self
                .items
                .iter()
                .all(|i| i.kind != ItemKind::File || self.done.contains(&i.index))
            && self.tops.values().all(|t| t.published);
        if settled {
            self.cancel()
        } else {
            self.checkpoint()
        }
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

    #[test]
    fn a_transfer_can_be_claimed_only_once_inside_this_process() {
        let path = Path::new("/nowhere/claim-test");
        assert!(claim_here(path));
        assert!(!claim_here(path));
        assert!(is_writing_here(path));
        let dir = tempfile::tempdir().unwrap();
        let held = TransferLock {
            _file: std::fs::File::create(dir.path().join("lock")).unwrap(),
            claimed_here: Some(path.to_path_buf()),
        };
        drop(held);
        assert!(!is_writing_here(path));
        assert!(claim_here(path));
        WRITING_HERE.lock().unwrap().remove(path);
    }

    #[test]
    fn a_file_moved_over_its_claim_is_published_whole() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"MZ moved instead of copied".to_vec();
        let items = [file_item(0, &["cat.jpg"], data.len() as u64)];
        let mut w = begin_holding(dir.path(), &items);
        write_whole(&mut w, 0, &data);
        let source = w.private_rel(&["cat.jpg".to_string()]).unwrap();

        w.publish_by_copy("cat.jpg", &source, "cat.jpg").unwrap();

        assert_eq!(std::fs::read(dir.path().join("cat.jpg")).unwrap(), data);
        assert!(w.tree.metadata(&source).is_err(), "nothing stays behind");
    }

    fn file_item(index: u32, path: &[&str], size: u64) -> Item {
        Item {
            index,
            path: path.iter().map(ToString::to_string).collect(),
            kind: ItemKind::File,
            size,
            mtime_ms: None,
            created_ms: None,
            group: None,
        }
    }

    fn dir_item(index: u32, path: &[&str]) -> Item {
        Item {
            index,
            path: path.iter().map(ToString::to_string).collect(),
            kind: ItemKind::Directory,
            size: 0,
            mtime_ms: None,
            created_ms: None,
            group: None,
        }
    }

    fn manifest(items: &[Item]) -> ValidManifest {
        let total = items.iter().map(|i| i.size).sum();
        validate_manifest(items, total, TargetOs::Other).unwrap()
    }

    fn hash(data: &[u8]) -> [u8; 32] {
        *blake3::hash(data).as_bytes()
    }

    fn write_whole(w: &mut TransferWriter, index: u32, data: &[u8]) {
        if !data.is_empty() {
            w.write(index, 0, data).unwrap();
        }
        let h = hash(data);
        w.segment_end(index, 0, h).unwrap();
        w.file_end(index, file_root(data.len() as u64, &[h]))
            .unwrap();
    }

    const ID: [u8; 16] = [1; 16];
    const PEER: DeviceId = [2; 32];
    const MANIFEST: [u8; 32] = [3; 32];

    fn begin(dir: &Path, items: &[Item]) -> TransferWriter {
        let folder = ReceiveFolder::open(dir).unwrap();
        TransferWriter::begin(&folder, &ID, PEER, MANIFEST, &manifest(items), false).unwrap()
    }

    fn begin_holding(dir: &Path, items: &[Item]) -> TransferWriter {
        let folder = ReceiveFolder::open(dir).unwrap();
        TransferWriter::begin(&folder, &ID, PEER, MANIFEST, &manifest(items), true).unwrap()
    }

    #[test]
    fn publishes_a_verified_file() {
        let dir = tempfile::tempdir().unwrap();
        let items = [file_item(0, &["a.txt"], 5)];
        let mut w = begin(dir.path(), &items);
        write_whole(&mut w, 0, b"hello");
        assert_eq!(w.finish().unwrap().published, 1);
        assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"hello");
    }

    #[test]
    fn removes_the_private_tree_when_done() {
        let dir = tempfile::tempdir().unwrap();
        let items = [file_item(0, &["a.txt"], 1)];
        let mut w = begin(dir.path(), &items);
        write_whole(&mut w, 0, b"x");
        w.finish().unwrap();
        assert!(!dir.path().join(TMP_DIR_NAME).exists());
    }

    #[test]
    fn never_overwrites_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"keep me").unwrap();
        let items = [file_item(0, &["a.txt"], 3)];
        let mut w = begin(dir.path(), &items);
        write_whole(&mut w, 0, b"new");
        w.finish().unwrap();
        assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"keep me");
        assert_eq!(std::fs::read(dir.path().join("a (1).txt")).unwrap(), b"new");
    }

    #[test]
    fn never_merges_into_an_existing_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("App")).unwrap();
        std::fs::write(dir.path().join("App/program.exe"), b"original").unwrap();
        let items = [
            dir_item(0, &["App"]),
            file_item(1, &["App", "version.dll"], 4),
        ];
        let mut w = begin(dir.path(), &items);
        write_whole(&mut w, 1, b"evil");
        w.finish().unwrap();
        assert!(!dir.path().join("App/version.dll").exists());
        assert_eq!(
            std::fs::read(dir.path().join("App/program.exe")).unwrap(),
            b"original"
        );
    }

    #[test]
    fn publishes_a_colliding_directory_under_a_new_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("App")).unwrap();
        let items = [dir_item(0, &["App"]), file_item(1, &["App", "lib.dll"], 3)];
        let mut w = begin(dir.path(), &items);
        write_whole(&mut w, 1, b"abc");
        w.finish().unwrap();
        assert!(!dir.path().join("App/lib.dll").exists());
        assert!(dir.path().join("App (1)/lib.dll").exists());
    }

    #[test]
    fn treats_a_case_variant_as_a_collision() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Report.TXT"), b"old").unwrap();
        let items = [file_item(0, &["report.txt"], 3)];
        let mut w = begin(dir.path(), &items);
        write_whole(&mut w, 0, b"new");
        w.finish().unwrap();
        assert!(dir.path().join("report (1).txt").exists());
    }

    #[test]
    fn keeps_a_file_out_of_the_folder_when_its_hash_is_wrong() {
        let dir = tempfile::tempdir().unwrap();
        let items = [file_item(0, &["a.txt"], 5)];
        let mut w = begin(dir.path(), &items);
        w.write(0, 0, b"hello").unwrap();
        assert!(matches!(
            w.segment_end(0, 0, [0; 32]),
            Err(StoreError::Integrity)
        ));
        assert!(!dir.path().join("a.txt").exists());
    }

    #[test]
    fn rejects_data_past_the_declared_size() {
        let dir = tempfile::tempdir().unwrap();
        let items = [file_item(0, &["a.txt"], 3)];
        let mut w = begin(dir.path(), &items);
        assert!(matches!(
            w.write(0, 0, b"toolong"),
            Err(StoreError::Protocol)
        ));
    }

    #[test]
    fn rejects_an_out_of_order_offset() {
        let dir = tempfile::tempdir().unwrap();
        let items = [file_item(0, &["a.txt"], 10)];
        let mut w = begin(dir.path(), &items);
        assert!(matches!(w.write(0, 5, b"x"), Err(StoreError::Protocol)));
    }

    #[test]
    fn rejects_data_for_a_directory_item() {
        let dir = tempfile::tempdir().unwrap();
        let items = [dir_item(0, &["d"])];
        let mut w = begin(dir.path(), &items);
        assert!(matches!(w.write(0, 0, b"x"), Err(StoreError::Protocol)));
    }

    #[test]
    fn finish_fails_when_a_file_never_arrived() {
        let dir = tempfile::tempdir().unwrap();
        let items = [file_item(0, &["a"], 1), file_item(1, &["b"], 1)];
        let mut w = begin(dir.path(), &items);
        write_whole(&mut w, 0, b"x");
        assert!(matches!(w.finish(), Err(StoreError::Protocol)));
    }

    #[test]
    fn holds_back_a_disguised_executable() {
        let dir = tempfile::tempdir().unwrap();
        let folder = ReceiveFolder::open(dir.path()).unwrap();
        let items = [file_item(0, &["photo.jpg"], 4)];
        let mut w =
            TransferWriter::begin(&folder, &[1; 16], [2; 32], [3; 32], &manifest(&items), true)
                .unwrap();
        write_whole(&mut w, 0, b"MZ\x90\0");
        let counts = w.finish().unwrap();
        assert_eq!(counts.held, 1);
        assert_eq!(counts.published, 0);
        assert!(!dir.path().join("photo.jpg").exists());
    }

    /// Case pairs that this library's folding tables are too old to know.
    const NEWER_CASE_PAIR: (&str, &str) = ("D\u{A7CE}r", "D\u{A7CF}r");

    #[test]
    fn two_items_never_share_a_folder_even_when_the_disk_thinks_their_names_match() {
        let dir = tempfile::tempdir().unwrap();
        let (one, two) = NEWER_CASE_PAIR;
        let items = [
            file_item(0, &[one, "evil.jpg"], 4),
            file_item(1, &[two, "ok.txt"], 4),
        ];
        let mut w = begin_holding(dir.path(), &items);
        write_whole(&mut w, 0, b"MZab");
        write_whole(&mut w, 1, b"fine");
        let counts = w.finish().unwrap();
        assert_eq!((counts.published, counts.held), (1, 1));
        let visible: Vec<_> = std::fs::read_dir(dir.path().join(two))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(visible, ["ok.txt"]);
    }

    #[test]
    fn a_partial_copy_left_by_a_crash_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"complete";
        let mut w = begin(dir.path(), &[file_item(0, &["a.bin"], 8)]);
        w.write(0, 0, data).unwrap();
        w.segment_end(0, 0, hash(data)).unwrap();
        w.record(&Record::FileDone { file: 0 }).unwrap();
        w.record(&Record::Copying {
            top: "a.bin".to_string(),
            target: "a.bin".to_string(),
        })
        .unwrap();
        w.checkpoint().unwrap();
        w.current = None;
        drop(w);
        std::fs::write(dir.path().join("a.bin"), b"par").unwrap();

        let folder = ReceiveFolder::open(dir.path()).unwrap();
        let (w, _) = TransferWriter::resume(&folder, &ID, PEER, MANIFEST).unwrap();
        w.finish().unwrap();
        assert_eq!(std::fs::read(dir.path().join("a.bin")).unwrap(), data);
        assert!(!dir.path().join("a (1).bin").exists());
    }

    #[test]
    fn a_finished_copy_found_after_a_crash_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"complete";
        let mut w = begin(dir.path(), &[file_item(0, &["a.bin"], 8)]);
        w.write(0, 0, data).unwrap();
        w.segment_end(0, 0, hash(data)).unwrap();
        w.record(&Record::FileDone { file: 0 }).unwrap();
        w.record(&Record::Copying {
            top: "a.bin".to_string(),
            target: "a.bin".to_string(),
        })
        .unwrap();
        w.checkpoint().unwrap();
        w.current = None;
        drop(w);
        std::fs::write(dir.path().join("a.bin"), data).unwrap();
        std::fs::remove_file(
            dir.path()
                .join(TMP_DIR_NAME)
                .join(hex(&ID))
                .join(FILES_DIR)
                .join("0")
                .join("a.bin"),
        )
        .unwrap();

        let folder = ReceiveFolder::open(dir.path()).unwrap();
        let (w, _) = TransferWriter::resume(&folder, &ID, PEER, MANIFEST).unwrap();
        let counts = w.finish().unwrap();
        assert_eq!(counts.published, 1);
        assert_eq!(std::fs::read(dir.path().join("a.bin")).unwrap(), data);
    }

    #[test]
    fn a_folder_named_like_a_program_does_not_excuse_what_is_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let items = [
            dir_item(0, &["trip.py"]),
            file_item(1, &["trip.py", "cat.jpg"], 4),
        ];
        let mut w = begin_holding(dir.path(), &items);
        write_whole(&mut w, 1, b"MZab");
        let counts = w.finish().unwrap();
        assert_eq!((counts.published, counts.held), (0, 1));
    }

    #[test]
    fn the_program_inside_an_application_bundle_is_not_held() {
        let dir = tempfile::tempdir().unwrap();
        let items = [
            dir_item(0, &["Tool.app"]),
            dir_item(1, &["Tool.app", "MacOS"]),
            file_item(2, &["Tool.app", "MacOS", "Tool"], 4),
        ];
        let mut w = begin_holding(dir.path(), &items);
        write_whole(&mut w, 2, b"\xCF\xFA\xED\xFE");
        let counts = w.finish().unwrap();
        assert_eq!((counts.published, counts.held), (1, 0));
    }

    #[test]
    fn manifest_rejects_folders_that_differ_only_by_case() {
        let top = [
            file_item(0, &["Dir", "evil.jpg"], 0),
            file_item(1, &["dir", "ok.txt"], 0),
        ];
        assert!(matches!(
            validate_manifest(&top, 0, TargetOs::Other),
            Err(StoreError::BadManifest)
        ));
        let nested = [
            file_item(0, &["a", "B", "x"], 0),
            file_item(1, &["a", "b", "y"], 0),
        ];
        assert!(matches!(
            validate_manifest(&nested, 0, TargetOs::Other),
            Err(StoreError::BadManifest)
        ));
        let declared = [dir_item(0, &["Dir"]), file_item(1, &["dir", "ok.txt"], 0)];
        assert!(matches!(
            validate_manifest(&declared, 0, TargetOs::Other),
            Err(StoreError::BadManifest)
        ));
    }

    #[test]
    fn manifest_accepts_one_folder_spelled_consistently() {
        let items = [
            dir_item(0, &["Dir"]),
            file_item(1, &["Dir", "a"], 0),
            file_item(2, &["Dir", "b"], 0),
        ];
        assert!(validate_manifest(&items, 0, TargetOs::Other).is_ok());
    }

    #[test]
    fn rejects_a_segment_past_the_end_of_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = begin(dir.path(), &[file_item(0, &["empty"], 0)]);
        w.segment_end(0, 0, hash(b"")).unwrap();
        assert!(matches!(
            w.segment_end(0, 1, hash(b"")),
            Err(StoreError::Protocol)
        ));
    }

    #[test]
    fn a_program_in_the_offer_does_not_excuse_a_disguised_one() {
        let dir = tempfile::tempdir().unwrap();
        let items = [
            dir_item(0, &["pack"]),
            file_item(1, &["pack", "a.py"], 4),
            file_item(2, &["pack", "cat.jpg"], 4),
        ];
        let mut w = begin_holding(dir.path(), &items);
        write_whole(&mut w, 1, b"pass");
        write_whole(&mut w, 2, b"MZab");
        let counts = w.finish().unwrap();
        assert_eq!((counts.published, counts.held), (0, 1));
    }

    #[test]
    fn a_file_named_as_a_program_is_not_held() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = begin_holding(dir.path(), &[file_item(0, &["run.py"], 4)]);
        write_whole(&mut w, 0, b"#!ab");
        let counts = w.finish().unwrap();
        assert_eq!((counts.published, counts.held), (1, 0));
    }

    #[test]
    fn resume_waits_for_an_earlier_writer_of_the_same_transfer() {
        let dir = tempfile::tempdir().unwrap();
        let live = begin(dir.path(), &[file_item(0, &["a"], 5)]);
        let folder = ReceiveFolder::open(dir.path()).unwrap();
        assert!(matches!(
            TransferWriter::resume(&folder, &ID, PEER, MANIFEST),
            Err(StoreError::Busy)
        ));
        drop(live);
        assert!(TransferWriter::resume(&folder, &ID, PEER, MANIFEST).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_crash_after_linking_cannot_rewrite_the_published_file() {
        let dir = tempfile::tempdir().unwrap();
        let data = [9u8; 64];
        let mut w = begin(dir.path(), &[file_item(0, &["a.bin"], 64)]);
        w.write(0, 0, &data).unwrap();
        w.segment_end(0, 0, hash(&data)).unwrap();
        w.checkpoint().unwrap();
        drop(w);
        let private = dir
            .path()
            .join(TMP_DIR_NAME)
            .join(hex(&ID))
            .join(FILES_DIR)
            .join("0")
            .join("a.bin");
        std::fs::hard_link(&private, dir.path().join("a.bin")).unwrap();

        let folder = ReceiveFolder::open(dir.path()).unwrap();
        let (mut w, point) = TransferWriter::resume(&folder, &ID, PEER, MANIFEST).unwrap();
        assert_eq!(point.next_segment, 0);
        w.write(0, 0, &[1u8; 8]).unwrap();
        assert_eq!(std::fs::read(dir.path().join("a.bin")).unwrap(), data);
    }

    #[test]
    fn a_torn_journal_record_does_not_hide_later_ones() {
        let dir = tempfile::tempdir().unwrap();
        let size = SEGMENT_SIZE * 2 + 5;
        let items = [file_item(0, &["a.bin"], size)];
        let seg = vec![3u8; SEGMENT_SIZE as usize];
        let mut w = begin(dir.path(), &items);
        w.write(0, 0, &seg).unwrap();
        w.segment_end(0, 0, hash(&seg)).unwrap();
        w.checkpoint().unwrap();
        drop(w);
        let journal = dir.path().join(TMP_DIR_NAME).join(hex(&ID)).join(LOG_FILE);
        let mut bytes = std::fs::read(&journal).unwrap();
        bytes.extend_from_slice(&[200, 0]);
        std::fs::write(&journal, bytes).unwrap();

        let folder = ReceiveFolder::open(dir.path()).unwrap();
        let (mut w, point) = TransferWriter::resume(&folder, &ID, PEER, MANIFEST).unwrap();
        assert_eq!(point.next_segment, 1);
        w.write(0, SEGMENT_SIZE, &seg).unwrap();
        w.segment_end(0, 1, hash(&seg)).unwrap();
        w.checkpoint().unwrap();
        drop(w);
        let (_w, point) = TransferWriter::resume(&folder, &ID, PEER, MANIFEST).unwrap();
        assert_eq!(point.next_segment, 2);
    }

    #[test]
    fn holds_a_program_whose_first_bytes_are_split_across_frames() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"MZ\x90\0";
        let mut w = begin_holding(dir.path(), &[file_item(0, &["cat.jpg"], data.len() as u64)]);
        w.write(0, 0, b"M").unwrap();
        w.write(0, 1, b"Z\x90\0").unwrap();
        let h = hash(data);
        w.segment_end(0, 0, h).unwrap();
        w.file_end(0, file_root(data.len() as u64, &[h])).unwrap();
        let counts = w.finish().unwrap();
        assert_eq!((counts.published, counts.held), (0, 1));
        assert!(!dir.path().join("cat.jpg").exists());
    }

    #[test]
    fn a_resume_does_not_clear_a_hold() {
        let dir = tempfile::tempdir().unwrap();
        let items = [
            dir_item(0, &["pics"]),
            file_item(1, &["pics", "cat.jpg"], 4),
            file_item(2, &["pics", "dog.jpg"], 4),
        ];
        let mut w = begin_holding(dir.path(), &items);
        write_whole(&mut w, 1, b"MZab");
        w.checkpoint().unwrap();
        drop(w);

        let folder = ReceiveFolder::open(dir.path()).unwrap();
        let (mut w, point) = TransferWriter::resume(&folder, &ID, PEER, MANIFEST).unwrap();
        assert_eq!((point.next_file, point.next_segment), (2, 0));
        write_whole(&mut w, 2, b"fine");
        let counts = w.finish().unwrap();
        assert_eq!((counts.published, counts.held), (0, 1));
        assert!(!dir.path().join("pics").exists());
    }

    #[test]
    fn resuming_a_file_with_every_segment_present_resends_the_last_one() {
        let dir = tempfile::tempdir().unwrap();
        let data = vec![7u8; SEGMENT_SIZE as usize + 5];
        let mut w = begin(dir.path(), &[file_item(0, &["a.bin"], data.len() as u64)]);
        let (first, second) = data.split_at(SEGMENT_SIZE as usize);
        w.write(0, 0, first).unwrap();
        w.segment_end(0, 0, hash(first)).unwrap();
        w.write(0, SEGMENT_SIZE, second).unwrap();
        w.segment_end(0, 1, hash(second)).unwrap();
        w.checkpoint().unwrap();
        drop(w);

        let folder = ReceiveFolder::open(dir.path()).unwrap();
        let (mut w, point) = TransferWriter::resume(&folder, &ID, PEER, MANIFEST).unwrap();
        assert_eq!(point.next_segment, 1);
        w.write(0, SEGMENT_SIZE, second).unwrap();
        w.segment_end(0, 1, hash(second)).unwrap();
        let root = file_root(data.len() as u64, &[hash(first), hash(second)]);
        w.file_end(0, root).unwrap();
        w.finish().unwrap();
        assert_eq!(std::fs::read(dir.path().join("a.bin")).unwrap(), data);
    }

    #[test]
    fn publishes_an_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let items = [dir_item(0, &["empty"])];
        let w = begin(dir.path(), &items);
        w.finish().unwrap();
        assert!(dir.path().join("empty").is_dir());
    }

    #[test]
    fn cancel_removes_unpublished_data() {
        let dir = tempfile::tempdir().unwrap();
        let items = [file_item(0, &["a"], 5)];
        let mut w = begin(dir.path(), &items);
        w.write(0, 0, b"he").unwrap();
        w.cancel().unwrap();
        assert!(!dir.path().join(TMP_DIR_NAME).exists());
    }

    #[test]
    fn manifest_rejects_a_wrong_total() {
        let items = [file_item(0, &["a"], 5)];
        assert!(matches!(
            validate_manifest(&items, 1, TargetOs::Other),
            Err(StoreError::BadManifest)
        ));
    }

    #[test]
    fn manifest_rejects_an_overflowing_total() {
        let items = [file_item(0, &["a"], u64::MAX), file_item(1, &["b"], 2)];
        assert!(matches!(
            validate_manifest(&items, 1, TargetOs::Other),
            Err(StoreError::BadManifest)
        ));
    }

    #[test]
    fn manifest_rejects_duplicates_after_folding() {
        let items = [file_item(0, &["File"], 0), file_item(1, &["file"], 0)];
        assert!(matches!(
            validate_manifest(&items, 0, TargetOs::Other),
            Err(StoreError::BadManifest)
        ));
    }

    #[test]
    fn manifest_rejects_a_collision_created_by_name_handling() {
        let items = [file_item(0, &["a:b"], 0), file_item(1, &["a_b"], 0)];
        assert!(matches!(
            validate_manifest(&items, 0, TargetOs::Windows),
            Err(StoreError::BadManifest)
        ));
    }

    #[test]
    fn manifest_rejects_a_file_used_as_a_parent() {
        let items = [file_item(0, &["x"], 0), file_item(1, &["X", "child"], 0)];
        assert!(matches!(
            validate_manifest(&items, 0, TargetOs::Other),
            Err(StoreError::BadManifest)
        ));
    }

    #[test]
    fn manifest_rejects_a_directory_with_a_size() {
        let mut item = dir_item(0, &["d"]);
        item.size = 1;
        assert!(matches!(
            validate_manifest(&[item], 1, TargetOs::Other),
            Err(StoreError::BadManifest)
        ));
    }

    #[test]
    fn manifest_rejects_a_gap_in_indexes() {
        let items = [file_item(1, &["a"], 0)];
        assert!(matches!(
            validate_manifest(&items, 0, TargetOs::Other),
            Err(StoreError::BadManifest)
        ));
    }

    #[test]
    fn manifest_rejects_path_traversal() {
        let items = [file_item(0, &["..", "evil"], 0)];
        assert!(matches!(
            validate_manifest(&items, 0, TargetOs::Other),
            Err(StoreError::InvalidPath(_))
        ));
    }

    #[test]
    fn resume_refuses_another_peer() {
        let dir = tempfile::tempdir().unwrap();
        let folder = ReceiveFolder::open(dir.path()).unwrap();
        let items = [file_item(0, &["a"], 5)];
        let w = begin(dir.path(), &items);
        drop(w);
        assert!(matches!(
            TransferWriter::resume(&folder, &[1; 16], [9; 32], [3; 32]),
            Err(StoreError::Unknown)
        ));
        assert!(matches!(
            TransferWriter::resume(&folder, &[1; 16], [2; 32], [9; 32]),
            Err(StoreError::Unknown)
        ));
    }

    #[test]
    fn resume_detects_a_corrupted_segment_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let folder = ReceiveFolder::open(dir.path()).unwrap();
        let size = SEGMENT_SIZE + 10;
        let items = [file_item(0, &["big.bin"], size)];
        let mut w = begin(dir.path(), &items);
        let segment = vec![7u8; SEGMENT_SIZE as usize];
        w.write(0, 0, &segment).unwrap();
        let h = hash(&segment);
        w.segment_end(0, 0, h).unwrap();
        w.checkpoint().unwrap();
        drop(w);

        let tree = dir
            .path()
            .join(TMP_DIR_NAME)
            .join(data_encoding::HEXLOWER.encode(&[1; 16]))
            .join(FILES_DIR)
            .join("0")
            .join("big.bin");
        let (w, point) = TransferWriter::resume(&folder, &[1; 16], [2; 32], [3; 32]).unwrap();
        assert_eq!(point.next_segment, 1);
        assert_eq!(point.prefix, prefix_digest(&[h]));
        drop(w);

        let mut data = std::fs::read(&tree).unwrap();
        data[100] ^= 1;
        std::fs::write(&tree, data).unwrap();
        let (_, point) = TransferWriter::resume(&folder, &[1; 16], [2; 32], [3; 32]).unwrap();
        assert_eq!(point.next_segment, 0);
    }

    #[test]
    fn candidates_keep_the_extension() {
        assert_eq!(candidate("a.txt", 0), "a.txt");
        assert_eq!(candidate("a.tar.gz", 2), "a.tar (2).gz");
        assert_eq!(candidate("folder", 1), "folder (1)");
        assert_eq!(candidate(".hidden", 1), ".hidden (1)");
    }

    #[test]
    fn segment_count_handles_boundaries() {
        assert_eq!(segment_count(0), 1);
        assert_eq!(segment_count(1), 1);
        assert_eq!(segment_count(SEGMENT_SIZE), 1);
        assert_eq!(segment_count(SEGMENT_SIZE + 1), 2);
    }
}
