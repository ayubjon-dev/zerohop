//! Sending and receiving transfers over a session (protocol section 10).

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::future::Future;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, watch};
use zeroize::Zeroize;

use crate::circle::random_bytes;
use crate::clip::{self, ClipMemory, ClipSink};
use crate::ledger::DeviceId;
use crate::names;
use crate::receive::{
    FinishCounts, MAX_ITEMS, MAX_MANIFEST_BYTES, MAX_MANIFEST_MEMORY, ReceiveFolder, ResumePoint,
    SEGMENT_SIZE, StoreError, TransferWriter, ValidManifest, file_root, prefix_digest,
    segment_count, validate_manifest,
};
use crate::session::{Incoming, Session, SessionError, WRITE_TIMEOUT};
use crate::wire::{
    Cancel, ClipOffer, ClipPull, Complete, Decision, Done, FILE_DATA_HEADER, FileEnd, Frame,
    FrameType, Item, ItemKind, MAX_CONTROL_PAYLOAD, MAX_DATA_PAYLOAD, Offer, OfferItems, Progress,
    ResumeRequest, ResumeState, SegmentEnd, WireError, code, file_data_header, parse_file_data,
    write_msg, write_raw,
};

const CHUNK_SIZE: usize = 512 * 1024;
const READ_AHEAD_CHUNKS: usize = 16;
const WRITE_QUEUE_CHUNKS: usize = 12;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
const ITEMS_PER_FRAME_BYTES: usize = MAX_CONTROL_PAYLOAD - 4096;
const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const MINUTE: Duration = Duration::from_secs(60);
const LINGER: Duration = Duration::from_secs(2);

/// Largest offer accepted without asking, by default.
pub const DEFAULT_AUTO_LIMIT: u64 = 10 * 1024 * 1024 * 1024;
const MAX_AUTO_BYTES_PER_DAY: u64 = 100 * 1024 * 1024 * 1024;
const MAX_AUTO_FILES_PER_DAY: u64 = 500_000;
const MAX_OFFERS_PER_MINUTE: usize = 10;
const MAX_CLIPS_PER_MINUTE: usize = 30;
const MAX_NAMES_SHOWN: usize = 5;

/// Errors during a transfer.
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    /// The session failed.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// Storing the data failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Reading the source files failed.
    #[error("could not read source file: {0}")]
    Source(#[from] io::Error),
    /// A source file changed during sending.
    #[error("a source file changed while it was being sent")]
    SourceChanged,
    /// The receiver declined.
    #[error("the other device declined (code {0})")]
    Declined(u16),
    /// The other side cancelled.
    #[error("the transfer was cancelled (code {0})")]
    Cancelled(u16),
    /// The offer had nothing usable in it, or too much.
    #[error("nothing could be offered")]
    BadOffer,
}

impl From<WireError> for TransferError {
    fn from(e: WireError) -> Self {
        Self::Session(SessionError::Wire(e))
    }
}

/// Follows one transfer and can stop it.
#[derive(Clone, Debug)]
pub struct Control {
    done: Arc<AtomicU64>,
    stop: Arc<watch::Sender<bool>>,
}

impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}

impl Control {
    /// A control for a transfer that has not started.
    #[must_use]
    pub fn new() -> Self {
        Self {
            done: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(watch::channel(false).0),
        }
    }

    /// Bytes of file data moved so far, including any carried over by a resume.
    #[must_use]
    pub fn bytes_done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    /// Asks the transfer to stop. The other device is told.
    pub fn cancel(&self) {
        self.stop.send_replace(true);
    }

    /// Whether stopping has been asked for.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.stop.borrow()
    }

    fn set_done(&self, bytes: u64) {
        self.done.store(bytes, Ordering::Relaxed);
    }

    fn add_done(&self, bytes: u64) {
        self.done.fetch_add(bytes, Ordering::Relaxed);
    }

    async fn cancelled(&self) {
        let mut stop = self.stop.subscribe();
        // The sender lives as long as `self`, so this can only end by being set.
        let _ = stop.wait_for(|set| *set).await;
    }
}

/// One item of an outgoing transfer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutgoingItem {
    /// What the receiver sees.
    pub item: Item,
    /// Where the data comes from. Absent for directories.
    pub source: Option<PathBuf>,
}

/// A prepared outgoing transfer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outgoing {
    /// The transfer's identifier.
    pub transfer_id: [u8; 16],
    /// Everything being sent, in order.
    pub items: Vec<OutgoingItem>,
    /// Sum of all file sizes.
    pub total_bytes: u64,
    /// Hash of the encoded manifest, used to bind a resume to it.
    pub manifest: [u8; 32],
}

fn millis(t: io::Result<SystemTime>) -> Option<i64> {
    let t = t.ok()?;
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).ok(),
        Err(e) => i64::try_from(e.duration().as_millis()).ok().map(|m| -m),
    }
}

fn clean_sender_name(name: &std::ffi::OsStr) -> Option<String> {
    Some(name.to_str()?.replace(['/', '\\'], "_"))
}

fn add_path(
    source: &Path,
    prefix: &[String],
    out: &mut Vec<OutgoingItem>,
    total: &mut u64,
) -> io::Result<()> {
    let Some(name) = source.file_name().and_then(clean_sender_name) else {
        return Ok(());
    };
    // A selection this large cannot go anyway, so the walk stops here.
    if out.len() > MAX_ITEMS as usize {
        return Err(io::Error::other("too many files"));
    }
    // Links are never sent, so a link cannot pull in files outside the selection.
    let meta = std::fs::symlink_metadata(source)?;
    let mut path = prefix.to_vec();
    path.push(name);
    let index = u32::try_from(out.len()).map_err(|_| io::Error::other("too many files"))?;
    let base = Item {
        index,
        path: path.clone(),
        kind: ItemKind::Directory,
        size: 0,
        mtime_ms: millis(meta.modified()),
        created_ms: millis(meta.created()),
        group: None,
    };
    if meta.is_dir() {
        out.push(OutgoingItem {
            item: base,
            source: None,
        });
        let mut entries = std::fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            add_path(&entry.path(), &path, out, total)?;
        }
    } else if meta.is_file() {
        *total = total
            .checked_add(meta.len())
            .ok_or_else(|| io::Error::other("selection is too large"))?;
        out.push(OutgoingItem {
            item: Item {
                kind: ItemKind::File,
                size: meta.len(),
                ..base
            },
            source: Some(source.to_path_buf()),
        });
    }
    Ok(())
}

fn manifest_hash(items: &[OutgoingItem]) -> Result<[u8; 32], TransferError> {
    manifest_of(items.iter().map(|i| &i.item))
}

/// The hash a transfer of exactly these items carries, as a receiver
/// computes it while reading the offer.
///
/// # Errors
/// Returns [`TransferError::BadOffer`] if an item cannot be encoded.
pub fn manifest_of<'a>(
    items: impl IntoIterator<Item = &'a Item>,
) -> Result<[u8; 32], TransferError> {
    let mut h = blake3::Hasher::new();
    for item in items {
        h.update(&postcard::to_stdvec(item).map_err(|_| TransferError::BadOffer)?);
    }
    Ok(*h.finalize().as_bytes())
}

/// Builds a transfer from files and folders on disk.
///
/// # Errors
/// Returns an error if a path cannot be read or nothing can be sent.
pub fn plan(paths: &[PathBuf]) -> Result<Outgoing, TransferError> {
    let mut items = Vec::new();
    let mut total_bytes = 0;
    for path in paths {
        if let Err(e) = add_path(path, &[], &mut items, &mut total_bytes) {
            return Err(if items.len() > MAX_ITEMS as usize {
                TransferError::BadOffer
            } else {
                e.into()
            });
        }
    }
    if items.is_empty() || items.len() > MAX_ITEMS as usize {
        return Err(TransferError::BadOffer);
    }
    let manifest = manifest_hash(&items)?;
    let transfer_id = random_bytes().map_err(|_| TransferError::BadOffer)?;
    Ok(Outgoing {
        transfer_id,
        items,
        total_bytes,
        manifest,
    })
}

#[derive(Serialize, Deserialize)]
struct SegmentRecord {
    file: u32,
    segment: u32,
    hash: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct SenderHeader {
    peer: DeviceId,
    outgoing: Outgoing,
}

/// The sender's record of an unfinished transfer.
struct SenderJournal {
    dir: PathBuf,
    log: BufWriter<File>,
}

fn journal_dir(state: &Path, id: &[u8; 16]) -> PathBuf {
    state
        .join("outgoing")
        .join(data_encoding::HEXLOWER.encode(id))
}

impl SenderJournal {
    fn create(state: &Path, peer: DeviceId, outgoing: &Outgoing) -> Result<Self, TransferError> {
        let dir = journal_dir(state, &outgoing.transfer_id);
        std::fs::create_dir_all(&dir)?;
        let header = SenderHeader {
            peer,
            outgoing: outgoing.clone(),
        };
        let bytes = postcard::to_stdvec(&header).map_err(|_| TransferError::BadOffer)?;
        std::fs::write(dir.join("header.bin"), bytes)?;
        let log = File::create(dir.join("segments.log"))?;
        Ok(Self {
            dir,
            log: BufWriter::with_capacity(64 * 1024, log),
        })
    }

    fn append(state: &Path, id: &[u8; 16], intact: u64) -> Result<Self, TransferError> {
        let dir = journal_dir(state, id);
        let mut log = std::fs::OpenOptions::new()
            .write(true)
            .open(dir.join("segments.log"))?;
        // A record torn by a crash must not stay in front of later ones.
        log.set_len(intact)?;
        log.seek(SeekFrom::Start(intact))?;
        Ok(Self {
            dir,
            log: BufWriter::with_capacity(64 * 1024, log),
        })
    }

    fn record(&mut self, file: u32, segment: u32, hash: [u8; 32]) -> io::Result<()> {
        let bytes = postcard::to_stdvec(&SegmentRecord {
            file,
            segment,
            hash,
        })
        .map_err(|_| io::Error::other("journal encoding failed"))?;
        self.log.write_all(&bytes)?;
        // One small write per segment, so a crash loses at most the last one.
        self.log.flush()
    }

    fn remove(self) {
        let dir = self.dir.clone();
        drop(self);
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Segment hashes the sender has written down, by file index.
type RecordedSegments = HashMap<u32, Vec<[u8; 32]>>;

struct SenderJournalState {
    peer: DeviceId,
    outgoing: Outgoing,
    recorded: RecordedSegments,
    /// Length of the part of the log that could be read.
    intact: u64,
}

fn load_sender_journal(state: &Path, id: &[u8; 16]) -> Result<SenderJournalState, TransferError> {
    let dir = journal_dir(state, id);
    let header: SenderHeader = postcard::from_bytes(&std::fs::read(dir.join("header.bin"))?)
        .map_err(|_| TransferError::BadOffer)?;
    let bytes = std::fs::read(dir.join("segments.log"))?;
    let mut map = RecordedSegments::new();
    let mut rest = bytes.as_slice();
    while let Ok((record, tail)) = postcard::take_from_bytes::<SegmentRecord>(rest) {
        let list = map.entry(record.file).or_default();
        if record.segment as usize == list.len() {
            list.push(record.hash);
        } else if (record.segment as usize) < list.len() {
            list.truncate(record.segment as usize);
            list.push(record.hash);
        }
        rest = tail;
    }
    Ok(SenderJournalState {
        peer: header.peer,
        outgoing: header.outgoing,
        recorded: map,
        intact: (bytes.len() - rest.len()) as u64,
    })
}

/// An outgoing transfer that can still be continued.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unfinished {
    /// The transfer's identifier.
    pub transfer_id: [u8; 16],
    /// The device it was going to.
    pub peer: DeviceId,
    /// Sum of all file sizes.
    pub total_bytes: u64,
    /// Number of files.
    pub files: u32,
    /// Name of the first item.
    pub first_name: String,
}

/// Lists outgoing transfers that were interrupted.
///
/// # Errors
/// Returns an error if the state directory cannot be read.
pub fn unfinished(state: &Path) -> io::Result<Vec<Unfinished>> {
    let dir = state.join("outgoing");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name();
        let id = name
            .to_str()
            .and_then(|n| data_encoding::HEXLOWER.decode(n.as_bytes()).ok())
            .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok());
        let Some(id) = id else { continue };
        if let Ok(journal) = load_sender_journal(state, &id) {
            let files = journal
                .outgoing
                .items
                .iter()
                .filter(|i| i.item.kind == ItemKind::File)
                .count();
            found.push(Unfinished {
                transfer_id: id,
                peer: journal.peer,
                total_bytes: journal.outgoing.total_bytes,
                files: u32::try_from(files).unwrap_or(u32::MAX),
                first_name: journal
                    .outgoing
                    .items
                    .first()
                    .and_then(|i| i.item.path.first())
                    .cloned()
                    .unwrap_or_default(),
            });
        }
    }
    Ok(found)
}

/// Gives up on an outgoing transfer so it can no longer be continued.
///
/// # Errors
/// Returns an error if the record exists but cannot be removed.
pub fn forget_unfinished(state: &Path, id: &[u8; 16]) -> io::Result<()> {
    match std::fs::remove_dir_all(journal_dir(state, id)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Removes records of outgoing transfers last touched longer ago than `max_age`.
///
/// # Errors
/// Returns an error if the state directory cannot be read.
pub fn sweep_outgoing(state: &Path, max_age: Duration) -> io::Result<u32> {
    let dir = state.join("outgoing");
    if !dir.exists() {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let touched = std::fs::metadata(path.join("segments.log"))
            .or_else(|_| std::fs::metadata(&path))
            .and_then(|m| m.modified());
        let old = touched.is_ok_and(|t| t.elapsed().is_ok_and(|age| age > max_age));
        if old && std::fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

enum Chunk {
    Data(Vec<u8>),
    SegmentEnd(SegmentEnd),
    FileEnd(FileEnd),
    Failed(TransferError),
}

struct StartAt {
    file: u32,
    segment: u32,
    prior: Vec<[u8; 32]>,
}

fn check_unchanged(file: &File, item: &Item) -> Result<(), TransferError> {
    let meta = file.metadata()?;
    if meta.len() != item.size || millis(meta.modified()) != item.mtime_ms {
        return Err(TransferError::SourceChanged);
    }
    Ok(())
}

fn send_file(
    item: &Item,
    source: &Path,
    start: Option<(u32, Vec<[u8; 32]>)>,
    tx: &mpsc::Sender<Chunk>,
    journal: &mut SenderJournal,
) -> Result<bool, TransferError> {
    let mut file = File::open(source)?;
    check_unchanged(&file, item)?;
    let (first_segment, mut hashes) = start.unwrap_or((0, Vec::new()));
    let mut offset = u64::from(first_segment) * SEGMENT_SIZE;
    file.seek(SeekFrom::Start(offset))?;
    let mut segment = first_segment;
    let mut hasher = blake3::Hasher::new();
    let mut in_segment: u64 = 0;

    loop {
        let want = usize::try_from(
            (SEGMENT_SIZE - in_segment)
                .min(CHUNK_SIZE as u64)
                .min(item.size - offset),
        )
        .map_err(|_| TransferError::BadOffer)?;
        if want > 0 {
            let mut buf = vec![0u8; FILE_DATA_HEADER + want];
            let (head, body) = buf.split_at_mut(FILE_DATA_HEADER);
            file.read_exact(body).map_err(|e| {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    TransferError::SourceChanged
                } else {
                    TransferError::Source(e)
                }
            })?;
            head.copy_from_slice(&file_data_header(item.index, offset));
            hasher.update(body);
            offset += want as u64;
            in_segment += want as u64;
            if tx.blocking_send(Chunk::Data(buf)).is_err() {
                return Ok(false);
            }
        }
        let segment_full = in_segment == SEGMENT_SIZE;
        let at_end = offset == item.size;
        if segment_full || at_end {
            let hash = *hasher.finalize().as_bytes();
            hasher.reset();
            in_segment = 0;
            hashes.push(hash);
            journal.record(item.index, segment, hash)?;
            let msg = SegmentEnd {
                file_index: item.index,
                segment,
                hash,
            };
            if tx.blocking_send(Chunk::SegmentEnd(msg)).is_err() {
                return Ok(false);
            }
            segment += 1;
        }
        if at_end {
            break;
        }
    }
    check_unchanged(&file, item)?;
    journal.log.flush()?;
    let msg = FileEnd {
        file_index: item.index,
        root: file_root(item.size, &hashes),
    };
    Ok(tx.blocking_send(Chunk::FileEnd(msg)).is_ok())
}

fn read_worker(
    outgoing: &Outgoing,
    mut start: Option<StartAt>,
    tx: &mpsc::Sender<Chunk>,
    journal: &mut SenderJournal,
) -> Result<(), TransferError> {
    let first = start.as_ref().map_or(0, |s| s.file);
    for entry in outgoing.items.iter().filter(|i| i.item.index >= first) {
        let Some(source) = entry.source.as_deref() else {
            continue;
        };
        let begin = start
            .take_if(|s| s.file == entry.item.index)
            .map(|s| (s.segment, s.prior));
        if !send_file(&entry.item, source, begin, tx, journal)? {
            return Ok(());
        }
    }
    Ok(())
}

/// The result of a completed send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendReport {
    /// Bytes of file data put on the wire in this run.
    pub bytes_sent: u64,
    /// Files the receiver published.
    pub published: u32,
    /// Top-level items the receiver is holding for approval.
    pub held: u32,
}

async fn write_bounded<S, T>(
    session: &mut Session<S>,
    kind: FrameType,
    msg: &T,
) -> Result<(), TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
    T: Serialize,
{
    tokio::time::timeout(WRITE_TIMEOUT, write_msg(&mut session.writer, kind, msg))
        .await
        .map_err(|_| SessionError::Timeout)??;
    Ok(())
}

/// Hashes the source prefix again. Timestamps alone cannot prove it is unchanged.
fn rehash_prefix(
    source: &Path,
    item: &Item,
    segments: u32,
) -> Result<Vec<[u8; 32]>, TransferError> {
    let mut file = File::open(source)?;
    check_unchanged(&file, item)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut hashes = Vec::with_capacity(segments as usize);
    for _ in 0..segments {
        let mut left = SEGMENT_SIZE;
        let mut hasher = blake3::Hasher::new();
        while left > 0 {
            let take = usize::try_from(left.min(1 << 20)).map_err(|_| TransferError::BadOffer)?;
            let slice = buf.get_mut(..take).ok_or(TransferError::BadOffer)?;
            file.read_exact(slice)?;
            hasher.update(slice);
            left -= take as u64;
        }
        hashes.push(*hasher.finalize().as_bytes());
    }
    Ok(hashes)
}

/// Waits for slow disk work while still answering the peer's pings, so that
/// re-hashing a large file does not look like a dead connection.
async fn keeping_alive<S, T>(
    session: &mut Session<S>,
    mut work: tokio::task::JoinHandle<T>,
) -> Result<T, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    loop {
        tokio::select! {
            done = &mut work => {
                return done.map_err(|_| TransferError::Session(SessionError::Poisoned));
            }
            event = session.wait() => {
                if session.process(event).await?.is_some() {
                    return session.violation().await.map_err(TransferError::from);
                }
            }
        }
    }
}

/// Tells the receiver the transfer cannot go on, so it can discard its data.
async fn announce_failure<S>(session: &mut Session<S>, transfer_id: [u8; 16], error: &TransferError)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let reason = if matches!(error, TransferError::SourceChanged) {
        code::SOURCE_CHANGED
    } else {
        code::CANCELLED
    };
    let _ = write_bounded(
        session,
        FrameType::Cancel,
        &Cancel {
            transfer_id,
            reason,
        },
    )
    .await;
    let _ = session.flush().await;
}

/// Waits briefly for the other side to hang up after being told to stop.
/// Closing while its data is still arriving would reset the connection, and
/// the reset can destroy the very message that explains why.
async fn linger<S>(session: &mut Session<S>)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let _ = tokio::time::timeout(LINGER, async {
        while !matches!(
            session.wait().await,
            Incoming::Closed | Incoming::Frame(Err(_)) | Incoming::PeerRemoved
        ) {}
    })
    .await;
}

async fn stream_data<S>(
    session: &mut Session<S>,
    outgoing: Arc<Outgoing>,
    start: Option<StartAt>,
    mut journal: SenderJournal,
    control: &Control,
) -> Result<SendReport, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (tx, mut rx) = mpsc::channel::<Chunk>(READ_AHEAD_CHUNKS);
    let worker_data = outgoing.clone();
    let worker = tokio::task::spawn_blocking(move || {
        if let Err(e) = read_worker(&worker_data, start, &tx, &mut journal) {
            let _ = tx.blocking_send(Chunk::Failed(e));
        }
        journal
    });

    let mut bytes_sent = 0u64;
    let mut reading = true;
    while reading {
        enum Step {
            In(Incoming),
            Chunk(Option<Chunk>),
            Stop,
        }
        let step = tokio::select! {
            event = session.wait() => Step::In(event),
            chunk = rx.recv() => Step::Chunk(chunk),
            () = control.cancelled() => Step::Stop,
        };
        match step {
            Step::Stop => {
                let stop = TransferError::Cancelled(code::CANCELLED);
                announce_failure(session, outgoing.transfer_id, &stop).await;
                drop(rx);
                if let Ok(journal) = worker.await {
                    journal.remove();
                }
                linger(session).await;
                return Err(stop);
            }
            Step::In(event) => {
                if let Some(frame) = session.process(event).await?
                    && let Err(e) = handle_sender_frame(&frame)
                {
                    // Once the other side has discarded it there is nothing to continue.
                    drop(rx);
                    if matches!(e, TransferError::Cancelled(_))
                        && let Ok(journal) = worker.await
                    {
                        journal.remove();
                    }
                    return Err(e);
                }
            }
            Step::Chunk(None) => reading = false,
            Step::Chunk(Some(Chunk::Data(buf))) => {
                let moved = (buf.len() - FILE_DATA_HEADER) as u64;
                bytes_sent += moved;
                control.add_done(moved);
                tokio::time::timeout(
                    WRITE_TIMEOUT,
                    write_raw(&mut session.writer, FrameType::FileData, &buf),
                )
                .await
                .map_err(|_| SessionError::Timeout)??;
            }
            Step::Chunk(Some(Chunk::SegmentEnd(msg))) => {
                write_bounded(session, FrameType::SegmentEnd, &msg).await?;
            }
            Step::Chunk(Some(Chunk::FileEnd(msg))) => {
                write_bounded(session, FrameType::FileEnd, &msg).await?;
            }
            Step::Chunk(Some(Chunk::Failed(e))) => {
                announce_failure(session, outgoing.transfer_id, &e).await;
                if let Ok(journal) = worker.await {
                    journal.remove();
                }
                return Err(e);
            }
        }
    }
    let journal = worker
        .await
        .map_err(|_| TransferError::Session(SessionError::Poisoned))?;

    let files = outgoing
        .items
        .iter()
        .filter(|i| i.item.kind == ItemKind::File)
        .count();
    let complete = Complete {
        transfer_id: outgoing.transfer_id,
        files: u32::try_from(files).map_err(|_| TransferError::BadOffer)?,
        bytes: outgoing.total_bytes,
    };
    write_bounded(session, FrameType::Complete, &complete).await?;
    session.flush().await?;

    loop {
        let frame = tokio::select! {
            frame = session.next_frame() => frame?,
            () = control.cancelled() => {
                let stop = TransferError::Cancelled(code::CANCELLED);
                announce_failure(session, outgoing.transfer_id, &stop).await;
                journal.remove();
                return Err(stop);
            }
        };
        if frame.kind == FrameType::Done {
            let done: Done = frame.decode()?;
            if done.status != 0 {
                return Err(TransferError::Cancelled(done.status));
            }
            journal.remove();
            return Ok(SendReport {
                bytes_sent,
                published: done.published,
                held: done.held,
            });
        }
        if let Err(e) = handle_sender_frame(&frame) {
            if matches!(e, TransferError::Cancelled(_)) {
                journal.remove();
            }
            return Err(e);
        }
    }
}

fn handle_sender_frame(frame: &Frame) -> Result<(), TransferError> {
    match frame.kind {
        FrameType::Progress => {
            frame.decode::<Progress>()?;
            Ok(())
        }
        FrameType::Cancel => Err(TransferError::Cancelled(frame.decode::<Cancel>()?.reason)),
        _ => Err(TransferError::Session(SessionError::Protocol)),
    }
}

async fn write_manifest<S>(
    session: &mut Session<S>,
    outgoing: &Outgoing,
) -> Result<(), TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let offer = Offer {
        transfer_id: outgoing.transfer_id,
        item_count: u32::try_from(outgoing.items.len()).map_err(|_| TransferError::BadOffer)?,
        total_bytes: outgoing.total_bytes,
    };
    write_bounded(session, FrameType::Offer, &offer).await?;
    let mut batch = Vec::new();
    let mut batch_bytes = 0usize;
    for entry in &outgoing.items {
        let size = postcard::to_stdvec(&entry.item)
            .map_err(|_| TransferError::BadOffer)?
            .len();
        if batch_bytes + size > ITEMS_PER_FRAME_BYTES && !batch.is_empty() {
            let items = std::mem::take(&mut batch);
            write_bounded(session, FrameType::OfferItems, &OfferItems { items }).await?;
            batch_bytes = 0;
        }
        batch.push(entry.item.clone());
        batch_bytes += size;
    }
    if !batch.is_empty() {
        write_bounded(session, FrameType::OfferItems, &OfferItems { items: batch }).await?;
    }
    session.flush().await?;
    Ok(())
}

/// Offers files to the peer and sends them if accepted.
///
/// # Errors
/// Returns an error if the offer is declined or the transfer fails.
pub async fn send<S>(
    session: &mut Session<S>,
    outgoing: Outgoing,
    state: &Path,
) -> Result<SendReport, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    send_with(session, outgoing, state, &Control::new()).await
}

/// Like [`send`], reporting progress to and stopping on `control`.
///
/// # Errors
/// Returns an error if the offer is declined or the transfer fails or is stopped.
pub async fn send_with<S>(
    session: &mut Session<S>,
    outgoing: Outgoing,
    state: &Path,
    control: &Control,
) -> Result<SendReport, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let journal = SenderJournal::create(state, *session.peer(), &outgoing)?;
    write_manifest(session, &outgoing).await?;
    // The owner of the other device may take a while to answer.
    let frame = tokio::select! {
        frame = session.next_frame() => frame?,
        () = control.cancelled() => {
            let stop = TransferError::Cancelled(code::CANCELLED);
            announce_failure(session, outgoing.transfer_id, &stop).await;
            journal.remove();
            return Err(stop);
        }
    };
    if frame.kind != FrameType::Decision {
        return Err(TransferError::Session(SessionError::Protocol));
    }
    let decision: Decision = frame.decode()?;
    if !decision.accept {
        journal.remove();
        return Err(TransferError::Declined(
            decision.reason.unwrap_or(code::DECLINED),
        ));
    }
    stream_data(session, Arc::new(outgoing), None, journal, control).await
}

/// Continues an interrupted transfer.
///
/// # Errors
/// Returns an error if the transfer cannot be continued.
pub async fn resume<S>(
    session: &mut Session<S>,
    transfer_id: [u8; 16],
    state: &Path,
) -> Result<SendReport, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    resume_with(session, transfer_id, state, &Control::new()).await
}

fn bytes_before(outgoing: &Outgoing, start: &StartAt) -> u64 {
    let whole: u64 = outgoing
        .items
        .iter()
        .filter(|i| i.item.index < start.file)
        .map(|i| i.item.size)
        .sum();
    whole + u64::from(start.segment) * SEGMENT_SIZE
}

/// Like [`resume`], reporting progress to and stopping on `control`.
///
/// # Errors
/// Returns an error if the transfer cannot be continued or is stopped.
pub async fn resume_with<S>(
    session: &mut Session<S>,
    transfer_id: [u8; 16],
    state: &Path,
    control: &Control,
) -> Result<SendReport, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let SenderJournalState {
        peer,
        outgoing,
        recorded: journal_hashes,
        intact,
    } = load_sender_journal(state, &transfer_id)?;
    if peer != *session.peer() {
        return Err(TransferError::BadOffer);
    }
    let request = ResumeRequest {
        transfer_id,
        manifest: outgoing.manifest,
    };
    write_bounded(session, FrameType::ResumeRequest, &request).await?;
    session.flush().await?;
    let frame = session.next_frame().await?;
    if frame.kind != FrameType::ResumeState {
        return Err(TransferError::Session(SessionError::Protocol));
    }
    let point: ResumeState = frame.decode()?;

    let start = match outgoing
        .items
        .iter()
        .find(|i| i.item.index == point.next_file && i.item.kind == ItemKind::File)
    {
        Some(entry) => {
            let source = entry.source.clone().ok_or(TransferError::BadOffer)?;
            let item = entry.item.clone();
            let segments = point.next_segment;
            // Only whole segments can be skipped, and the last one never is.
            if u64::from(segments) >= segment_count(item.size) {
                return Err(TransferError::Session(SessionError::Protocol));
            }
            let recorded = journal_hashes.get(&item.index).cloned().unwrap_or_default();
            let prefix = point.prefix;
            let work = tokio::task::spawn_blocking(move || rehash_prefix(&source, &item, segments));
            let hashes = match keeping_alive(session, work).await? {
                Ok(hashes) => hashes,
                Err(e) => {
                    announce_failure(session, transfer_id, &e).await;
                    if matches!(e, TransferError::SourceChanged) {
                        let _ = std::fs::remove_dir_all(journal_dir(state, &transfer_id));
                    }
                    return Err(e);
                }
            };
            let matches = prefix_digest(&hashes) == prefix
                && recorded.get(..hashes.len()) == Some(hashes.as_slice());
            // Any disagreement restarts this file from its beginning.
            let (segment, prior) = if matches {
                (segments, hashes)
            } else {
                (0, Vec::new())
            };
            StartAt {
                file: point.next_file,
                segment,
                prior,
            }
        }
        None => StartAt {
            file: point.next_file,
            segment: 0,
            prior: Vec::new(),
        },
    };
    control.set_done(bytes_before(&outgoing, &start));
    let journal = SenderJournal::append(state, &transfer_id, intact)?;
    stream_data(session, Arc::new(outgoing), Some(start), journal, control).await
}

/// How a device treats incoming offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiveMode {
    /// The owner confirms every offer.
    AlwaysAsk,
    /// Offers are accepted up to the limits.
    Automatic,
    /// As automatic, but an offer containing a program asks first.
    AutomaticExceptPrograms,
    /// Offers are declined without asking.
    Refuse,
}

/// What the owner is asked about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferSummary {
    /// The sending device.
    pub peer: DeviceId,
    /// Number of files.
    pub files: u32,
    /// Total size.
    pub total_bytes: u64,
    /// Whether a program or script is included.
    pub has_program: bool,
    /// Names of the first few top-level items, safe to display.
    pub names: Vec<String>,
    /// Path of the first program in the offer, safe to display.
    pub first_program: Option<String>,
    /// The transfer being offered.
    pub transfer_id: [u8; 16],
}

/// What is about to arrive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiveStart {
    /// The transfer.
    pub transfer_id: [u8; 16],
    /// The sending device.
    pub peer: DeviceId,
    /// Number of files.
    pub files: u32,
    /// Total size.
    pub total_bytes: u64,
    /// Bytes already here, when an interrupted transfer continues.
    pub already: u64,
    /// Names of the first few top-level items, safe to display.
    pub names: Vec<String>,
}

/// Asks the owner to approve an offer, and hears how transfers go.
pub trait Approver: Send + Sync {
    /// Resolves to whether the owner accepted. Dropped if the offer goes away.
    fn approve(&self, offer: OfferSummary) -> Pin<Box<dyn Future<Output = bool> + Send>>;

    /// Called when data is about to arrive. The result follows and can stop it.
    fn receiving(&self, _start: &ReceiveStart) -> Control {
        Control::new()
    }

    /// Called once an offer has been dealt with.
    fn finished(&self, _peer: &DeviceId, _transfer: &[u8; 16], _outcome: &OfferOutcome) {}

    /// Called when a transfer that had started stopped before finishing. The
    /// reason is missing when a replaced connection simply dropped the work.
    fn interrupted(&self, _peer: &DeviceId, _transfer: &[u8; 16], _why: Option<&TransferError>) {}
}

/// Reports a transfer that ended early, however it ended, including when a
/// replaced connection simply drops the work in progress.
struct InterruptGuard<'a> {
    approver: &'a dyn Approver,
    peer: DeviceId,
    transfer: [u8; 16],
    reported: bool,
}

impl Drop for InterruptGuard<'_> {
    fn drop(&mut self) {
        if !self.reported {
            self.approver.interrupted(&self.peer, &self.transfer, None);
        }
    }
}

#[derive(Default)]
struct PeerUsage {
    offers: VecDeque<Instant>,
    clips: VecDeque<Instant>,
    accepted: VecDeque<(Instant, u64, u64)>,
}

/// Limits on what each peer can do without asking.
#[derive(Default)]
pub struct Budgets {
    peers: Mutex<HashMap<DeviceId, PeerUsage>>,
    clip_memory: ClipMemory,
}

impl Budgets {
    pub(crate) const fn clip_memory(&self) -> &ClipMemory {
        &self.clip_memory
    }

    pub(crate) fn allow_clip(&self, peer: &DeviceId) -> bool {
        let Ok(mut peers) = self.peers.lock() else {
            return false;
        };
        let usage = peers.entry(*peer).or_default();
        let now = Instant::now();
        while usage.clips.front().is_some_and(|t| now - *t > MINUTE) {
            usage.clips.pop_front();
        }
        if usage.clips.len() >= MAX_CLIPS_PER_MINUTE {
            return false;
        }
        usage.clips.push_back(now);
        true
    }

    pub(crate) fn allow_offer(&self, peer: &DeviceId) -> bool {
        let Ok(mut peers) = self.peers.lock() else {
            return false;
        };
        let usage = peers.entry(*peer).or_default();
        let now = Instant::now();
        while usage.offers.front().is_some_and(|t| now - *t > MINUTE) {
            usage.offers.pop_front();
        }
        if usage.offers.len() >= MAX_OFFERS_PER_MINUTE {
            return false;
        }
        usage.offers.push_back(now);
        true
    }

    fn within_auto_budget(&self, peer: &DeviceId, bytes: u64, files: u64) -> bool {
        let Ok(mut peers) = self.peers.lock() else {
            return false;
        };
        let usage = peers.entry(*peer).or_default();
        let now = Instant::now();
        while usage
            .accepted
            .front()
            .is_some_and(|(t, _, _)| now - *t > DAY)
        {
            usage.accepted.pop_front();
        }
        let used_bytes: u64 = usage.accepted.iter().map(|(_, b, _)| *b).sum();
        let used_files: u64 = usage.accepted.iter().map(|(_, _, f)| *f).sum();
        used_bytes.saturating_add(bytes) <= MAX_AUTO_BYTES_PER_DAY
            && used_files.saturating_add(files) <= MAX_AUTO_FILES_PER_DAY
    }

    fn charge(&self, peer: &DeviceId, bytes: u64, files: u64) {
        if let Ok(mut peers) = self.peers.lock() {
            peers
                .entry(*peer)
                .or_default()
                .accepted
                .push_back((Instant::now(), bytes, files));
        }
    }
}

struct SettingsState {
    folder: ReceiveFolder,
    default_mode: ReceiveMode,
    per_device: HashMap<DeviceId, ReceiveMode>,
}

/// Where files go and how offers are treated. A change applies to the next offer.
#[derive(Clone)]
pub struct ReceiveSettings(Arc<RwLock<SettingsState>>);

impl ReceiveSettings {
    /// Settings with one mode for every device.
    #[must_use]
    pub fn new(folder: ReceiveFolder, default_mode: ReceiveMode) -> Self {
        Self(Arc::new(RwLock::new(SettingsState {
            folder,
            default_mode,
            per_device: HashMap::new(),
        })))
    }

    /// The folder files are published into.
    #[must_use]
    pub fn folder(&self) -> ReceiveFolder {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .folder
            .clone()
    }

    /// How offers from this device are treated.
    #[must_use]
    pub fn mode_for(&self, peer: &DeviceId) -> ReceiveMode {
        let state = self.0.read().unwrap_or_else(PoisonError::into_inner);
        state
            .per_device
            .get(peer)
            .copied()
            .unwrap_or(state.default_mode)
    }

    /// Changes where files are published.
    pub fn set_folder(&self, folder: ReceiveFolder) {
        self.0
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .folder = folder;
    }

    /// Changes the mode for devices without their own.
    pub fn set_default_mode(&self, mode: ReceiveMode) {
        self.0
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .default_mode = mode;
    }

    /// Sets this device's own mode, or removes it with `None`.
    pub fn set_device_mode(&self, peer: DeviceId, mode: Option<ReceiveMode>) {
        let mut state = self.0.write().unwrap_or_else(PoisonError::into_inner);
        match mode {
            Some(mode) => {
                state.per_device.insert(peer, mode);
            }
            None => {
                state.per_device.remove(&peer);
            }
        }
    }
}

/// Receiving configuration for a device.
#[derive(Clone)]
pub struct ReceiveConfig {
    /// Where files go and how offers are treated.
    pub settings: ReceiveSettings,
    /// Asks the owner when needed.
    pub approver: Arc<dyn Approver>,
    /// Largest offer accepted without asking.
    pub auto_limit: u64,
    /// Per-peer limits.
    pub budgets: Arc<Budgets>,
    /// Where copied clips go. Without one, every clip is left behind.
    pub clip: Option<Arc<dyn ClipSink>>,
}

enum WriteCmd {
    Data(Vec<u8>),
    SegmentEnd(SegmentEnd),
    FileEnd(FileEnd),
    Finish,
    Cancel,
}

enum WriteEvent {
    Progress(u32, u32),
}

fn writer_loop(
    mut writer: TransferWriter,
    mut rx: mpsc::Receiver<WriteCmd>,
    events: &mpsc::Sender<WriteEvent>,
    control: &Control,
) -> Result<Option<FinishCounts>, StoreError> {
    let mut last = Instant::now();
    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            WriteCmd::Data(payload) => {
                let (index, offset, data) =
                    parse_file_data(&payload).map_err(|_| StoreError::Protocol)?;
                writer.write(index, offset, data)?;
                control.add_done(data.len() as u64);
            }
            WriteCmd::SegmentEnd(m) => {
                writer.segment_end(m.file_index, m.segment, m.hash)?;
                if last.elapsed() >= PROGRESS_INTERVAL {
                    last = Instant::now();
                    writer.checkpoint()?;
                    let _ = events.try_send(WriteEvent::Progress(m.file_index, m.segment));
                }
            }
            WriteCmd::FileEnd(m) => writer.file_end(m.file_index, m.root)?,
            WriteCmd::Finish => return writer.finish().map(Some),
            WriteCmd::Cancel => {
                writer.cancel()?;
                return Ok(None);
            }
        }
    }
    // The connection ended early. Keep what arrived so the transfer can continue.
    writer.checkpoint()?;
    Ok(None)
}

const fn store_error_code(e: &StoreError) -> u16 {
    match e {
        StoreError::Integrity => code::INTEGRITY_FAILED,
        StoreError::NoSpace => code::NO_SPACE,
        StoreError::InvalidPath(_) | StoreError::BadManifest => code::INVALID_PATH,
        StoreError::Unknown => code::UNKNOWN_TRANSFER,
        StoreError::Busy => code::RATE_LIMITED,
        StoreError::Io(_) | StoreError::Protocol => code::CANCELLED,
    }
}

async fn receive_stream<S>(
    session: &mut Session<S>,
    transfer_id: [u8; 16],
    writer: TransferWriter,
    control: Control,
) -> Result<FinishCounts, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (cmd_tx, cmd_rx) = mpsc::channel::<WriteCmd>(WRITE_QUEUE_CHUNKS);
    let (event_tx, mut event_rx) = mpsc::channel::<WriteEvent>(4);
    let meter = control.clone();
    let mut task =
        tokio::task::spawn_blocking(move || writer_loop(writer, cmd_rx, &event_tx, &meter));

    enum Step {
        In(Incoming),
        Progress(u32, u32),
        Finished(Result<Result<Option<FinishCounts>, StoreError>, tokio::task::JoinError>),
        Stop,
    }

    loop {
        let step = tokio::select! {
            event = session.wait() => Step::In(event),
            Some(WriteEvent::Progress(file, segment)) = event_rx.recv() => Step::Progress(file, segment),
            result = &mut task => Step::Finished(result),
            () = control.cancelled() => Step::Stop,
        };
        match step {
            Step::Stop => {
                let stop = TransferError::Cancelled(code::CANCELLED);
                let _ = cmd_tx.send(WriteCmd::Cancel).await;
                let _ = task.await;
                announce_failure(session, transfer_id, &stop).await;
                linger(session).await;
                return Err(stop);
            }
            Step::Progress(file_index, segment) => {
                let msg = Progress {
                    transfer_id,
                    file_index,
                    segment,
                };
                write_bounded(session, FrameType::Progress, &msg).await?;
                session.flush().await?;
            }
            Step::Finished(result) => {
                let outcome = result.map_err(|_| TransferError::Session(SessionError::Poisoned))?;
                return match outcome {
                    Ok(Some(counts)) => {
                        let done = Done {
                            transfer_id,
                            status: 0,
                            published: counts.published,
                            held: counts.held,
                        };
                        write_bounded(session, FrameType::Done, &done).await?;
                        session.flush().await?;
                        Ok(counts)
                    }
                    Ok(None) => Err(TransferError::Cancelled(code::CANCELLED)),
                    Err(e) => {
                        let reason = store_error_code(&e);
                        let msg = Cancel {
                            transfer_id,
                            reason,
                        };
                        let _ = write_bounded(session, FrameType::Cancel, &msg).await;
                        let _ = session.flush().await;
                        linger(session).await;
                        Err(e.into())
                    }
                };
            }
            Step::In(event) => {
                let Some(frame) = session.process(event).await? else {
                    continue;
                };
                let cmd = match frame.kind {
                    FrameType::FileData => WriteCmd::Data(frame.payload),
                    FrameType::SegmentEnd => WriteCmd::SegmentEnd(frame.decode()?),
                    FrameType::FileEnd => WriteCmd::FileEnd(frame.decode()?),
                    FrameType::Complete => {
                        frame.decode::<Complete>()?;
                        WriteCmd::Finish
                    }
                    FrameType::Cancel => {
                        let msg: Cancel = frame.decode()?;
                        // A source that changed can be sent again, so keep nothing either way.
                        let _ = cmd_tx.send(WriteCmd::Cancel).await;
                        let _ = task.await;
                        return Err(TransferError::Cancelled(msg.reason));
                    }
                    _ => return session.violation().await.map_err(TransferError::from),
                };
                // A full queue means the disk is behind, which is exactly when
                // this must stop reading the socket.
                tokio::time::timeout(WRITE_TIMEOUT, cmd_tx.send(cmd))
                    .await
                    .map_err(|_| SessionError::Timeout)?
                    .map_err(|_| SessionError::Closed)
                    .or_else(|e| {
                        // The writer has stopped; its result carries the reason.
                        if task.is_finished() { Ok(()) } else { Err(e) }
                    })?;
            }
        }
    }
}

struct ManifestRead {
    items: Vec<Item>,
    hash: [u8; 32],
    /// Bytes the manifest occupied on the wire, and roughly on disk.
    encoded: usize,
}

async fn read_manifest<S>(
    session: &mut Session<S>,
    count: u32,
) -> Result<ManifestRead, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    if count > MAX_ITEMS {
        return Err(StoreError::BadManifest.into());
    }
    let mut items = Vec::new();
    let mut encoded = 0usize;
    let mut memory = 0usize;
    let mut hasher = blake3::Hasher::new();
    while items.len() < count as usize {
        let frame = session.next_frame().await?;
        if frame.kind != FrameType::OfferItems {
            return session.violation().await.map_err(TransferError::from);
        }
        encoded += frame.payload.len();
        if encoded > MAX_MANIFEST_BYTES {
            return Err(StoreError::BadManifest.into());
        }
        let msg: OfferItems = frame.decode()?;
        for item in msg.items {
            memory += 128 + item.path.iter().map(|p| p.len() + 32).sum::<usize>();
            if memory > MAX_MANIFEST_MEMORY || items.len() >= count as usize {
                return Err(StoreError::BadManifest.into());
            }
            hasher.update(&postcard::to_stdvec(&item).map_err(|_| StoreError::BadManifest)?);
            items.push(item);
        }
    }
    Ok(ManifestRead {
        items,
        hash: *hasher.finalize().as_bytes(),
        encoded,
    })
}

async fn decline<S>(
    session: &mut Session<S>,
    transfer_id: [u8; 16],
    reason: u16,
) -> Result<(), TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let msg = Decision {
        transfer_id,
        accept: false,
        reason: Some(reason),
    };
    write_bounded(session, FrameType::Decision, &msg).await?;
    session.flush().await?;
    Ok(())
}

async fn ask_owner<S>(
    session: &mut Session<S>,
    cfg: &ReceiveConfig,
    summary: OfferSummary,
) -> Result<Answer, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let mut decision = cfg.approver.approve(summary);
    loop {
        // Waiting for a person can take a while, so keep answering pings.
        tokio::select! {
            accepted = &mut decision => {
                return Ok(if accepted { Answer::Accepted } else { Answer::Declined });
            }
            event = session.wait() => {
                if let Some(frame) = session.process(event).await? {
                    if frame.kind == FrameType::Cancel {
                        frame.decode::<Cancel>()?;
                        return Ok(Answer::Withdrawn);
                    }
                    return session.violation().await.map_err(TransferError::from);
                }
            }
        }
    }
}

enum Answer {
    Accepted,
    Declined,
    /// The sender changed its mind while the owner was being asked.
    Withdrawn,
}

/// The first few top-level names, and the first program, safe to display.
fn offer_names(valid: &ValidManifest) -> (Vec<String>, Option<String>) {
    let mut top = Vec::new();
    for item in &valid.items {
        if let Some(first) = item.path.first()
            && !top.contains(first)
            && top.len() < MAX_NAMES_SHOWN
        {
            top.push(first.clone());
        }
    }
    let first_program = valid
        .items
        .iter()
        .find(|i| names::is_program_path(&i.path))
        .map(|i| names::display_name(&i.path.join("/")));
    (
        top.iter().map(|n| names::display_name(n)).collect(),
        first_program,
    )
}

/// What happened to one offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferOutcome {
    /// The transfer finished.
    Received(FinishCounts),
    /// The offer was declined with this reason.
    Declined(u16),
}

/// What an offer costs against a peer's limits: bytes and entries. Folders and
/// the journal cost something even when every file is empty.
fn offer_cost(valid: &ValidManifest, encoded: usize) -> (u64, u64) {
    (
        valid.total_bytes.saturating_add(encoded as u64),
        valid.items.len() as u64,
    )
}

async fn handle_offer<S>(
    session: &mut Session<S>,
    cfg: &ReceiveConfig,
    offer: Offer,
) -> Result<OfferOutcome, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    receive_offer(session, cfg, offer, None).await
}

/// Takes an offer. With `expect`, only a transfer of exactly that manifest is
/// accepted: what a pull asked for, and nothing else in its place.
async fn receive_offer<S>(
    session: &mut Session<S>,
    cfg: &ReceiveConfig,
    offer: Offer,
    expect: Option<[u8; 32]>,
) -> Result<OfferOutcome, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let manifest = read_manifest(session, offer.item_count).await?;
    let peer = *session.peer();
    let id = offer.transfer_id;
    let reject = |reason| OfferOutcome::Declined(reason);
    if expect.is_some_and(|hash| hash != manifest.hash) {
        decline(session, id, code::DECLINED).await?;
        return Ok(reject(code::DECLINED));
    }
    let folder = cfg.settings.folder();
    let configured = cfg.settings.mode_for(&peer);

    if configured == ReceiveMode::Refuse {
        decline(session, id, code::DECLINED).await?;
        return Ok(reject(code::DECLINED));
    }
    if !cfg.budgets.allow_offer(&peer) {
        decline(session, id, code::RATE_LIMITED).await?;
        return Ok(reject(code::RATE_LIMITED));
    }
    let valid: ValidManifest =
        match validate_manifest(&manifest.items, offer.total_bytes, folder.os()) {
            Ok(v) => v,
            Err(_) => {
                decline(session, id, code::INVALID_PATH).await?;
                return Ok(reject(code::INVALID_PATH));
            }
        };
    let (cost, entries) = offer_cost(&valid, manifest.encoded);
    if !folder.has_room_for(cost)? {
        decline(session, id, code::NO_SPACE).await?;
        return Ok(reject(code::NO_SPACE));
    }

    let mode = if configured == ReceiveMode::Automatic && !folder.marks_supported() {
        ReceiveMode::AutomaticExceptPrograms
    } else {
        configured
    };
    let needs_owner = mode == ReceiveMode::AlwaysAsk
        || (mode == ReceiveMode::AutomaticExceptPrograms && valid.has_program)
        || valid.total_bytes > cfg.auto_limit
        || !cfg.budgets.within_auto_budget(&peer, cost, entries);
    if needs_owner {
        let (names, first_program) = offer_names(&valid);
        let summary = OfferSummary {
            peer,
            files: valid.file_count,
            total_bytes: valid.total_bytes,
            has_program: valid.has_program,
            names,
            first_program,
            transfer_id: id,
        };
        match ask_owner(session, cfg, summary).await? {
            Answer::Accepted => {}
            Answer::Declined => {
                decline(session, id, code::DECLINED).await?;
                return Ok(reject(code::DECLINED));
            }
            Answer::Withdrawn => return Ok(reject(code::CANCELLED)),
        }
    } else {
        cfg.budgets.charge(&peer, cost, entries);
    }

    // Approving an offer approves files as what their names claim, so a file
    // whose contents disagree with its name is still held back.
    let hold = mode != ReceiveMode::Automatic;
    let manifest_hash = manifest.hash;
    let (names, _) = offer_names(&valid);
    let (files, total_bytes) = (valid.file_count, valid.total_bytes);
    let begun = tokio::task::spawn_blocking(move || {
        TransferWriter::begin(&folder, &id, peer, manifest_hash, &valid, hold)
    })
    .await
    .map_err(|_| TransferError::Session(SessionError::Poisoned))?;
    let writer = match begun {
        Ok(writer) => writer,
        Err(e) => {
            // The sender deserves a reason, not a dropped connection.
            let reason = store_error_code(&e);
            decline(session, id, reason).await?;
            return Ok(reject(reason));
        }
    };
    let msg = Decision {
        transfer_id: id,
        accept: true,
        reason: None,
    };
    write_bounded(session, FrameType::Decision, &msg).await?;
    session.flush().await?;
    let start = ReceiveStart {
        transfer_id: id,
        peer,
        files,
        total_bytes,
        already: 0,
        names,
    };
    receive_watched(session, cfg, start, writer).await
}

/// Runs the receiving half with the owner kept informed, whatever happens.
async fn receive_watched<S>(
    session: &mut Session<S>,
    cfg: &ReceiveConfig,
    start: ReceiveStart,
    writer: TransferWriter,
) -> Result<OfferOutcome, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let control = cfg.approver.receiving(&start);
    control.set_done(start.already);
    let mut guard = InterruptGuard {
        approver: cfg.approver.as_ref(),
        peer: start.peer,
        transfer: start.transfer_id,
        reported: false,
    };
    let result = receive_stream(session, start.transfer_id, writer, control).await;
    guard.reported = true;
    match result {
        Ok(counts) => Ok(OfferOutcome::Received(counts)),
        Err(error) => {
            cfg.approver
                .interrupted(&start.peer, &start.transfer_id, Some(&error));
            Err(error)
        }
    }
}

async fn handle_resume<S>(
    session: &mut Session<S>,
    cfg: &ReceiveConfig,
    request: ResumeRequest,
) -> Result<OfferOutcome, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let folder = cfg.settings.folder();
    let peer = *session.peer();
    let id = request.transfer_id;
    let manifest = request.manifest;
    if cfg.settings.mode_for(&peer) == ReceiveMode::Refuse {
        session.send_error(code::DECLINED).await;
        return Ok(OfferOutcome::Declined(code::DECLINED));
    }
    // Reopening re-hashes everything received so far, so it counts as an offer.
    if !cfg.budgets.allow_offer(&peer) {
        session.send_error(code::RATE_LIMITED).await;
        return Ok(OfferOutcome::Declined(code::RATE_LIMITED));
    }
    let work =
        tokio::task::spawn_blocking(move || TransferWriter::resume(&folder, &id, peer, manifest));
    let reopened = keeping_alive(session, work).await?;
    let (writer, point): (TransferWriter, ResumePoint) = match reopened {
        Ok(v) => v,
        Err(StoreError::Unknown) => {
            session.send_error(code::UNKNOWN_TRANSFER).await;
            return Ok(OfferOutcome::Declined(code::UNKNOWN_TRANSFER));
        }
        Err(StoreError::Busy) => {
            session.send_error(code::RATE_LIMITED).await;
            return Ok(OfferOutcome::Declined(code::RATE_LIMITED));
        }
        Err(e) => {
            let reason = store_error_code(&e);
            session.send_error(reason).await;
            return Ok(OfferOutcome::Declined(reason));
        }
    };
    let state = ResumeState {
        transfer_id: id,
        next_file: point.next_file,
        next_segment: point.next_segment,
        prefix: point.prefix,
    };
    write_bounded(session, FrameType::ResumeState, &state).await?;
    session.flush().await?;
    let summary = writer.summary();
    let start = ReceiveStart {
        transfer_id: id,
        peer,
        files: summary.files,
        total_bytes: summary.total_bytes,
        already: summary.already,
        names: summary.names,
    };
    receive_watched(session, cfg, start, writer).await
}

/// Handles offers on an established session until the peer finishes.
///
/// # Errors
/// Returns an error if the session or a transfer fails.
pub async fn serve_session<S>(
    session: &mut Session<S>,
    cfg: &ReceiveConfig,
) -> Result<Vec<OfferOutcome>, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let mut outcomes = Vec::new();
    loop {
        let mut frame = match session.next_frame().await {
            Ok(f) => f,
            Err(SessionError::Closed) => return Ok(outcomes),
            Err(e) => return Err(e.into()),
        };
        match frame.kind {
            FrameType::Goodbye => return Ok(outcomes),
            FrameType::ClipOffer => {
                let offer = frame.decode::<ClipOffer>();
                frame.payload.zeroize();
                clip::receive(session, cfg, offer?).await?;
                return Ok(outcomes);
            }
            FrameType::Offer => {
                let offer: Offer = frame.decode()?;
                let id = offer.transfer_id;
                let outcome = handle_offer(session, cfg, offer).await?;
                cfg.approver.finished(session.peer(), &id, &outcome);
                outcomes.push(outcome);
            }
            FrameType::ResumeRequest => {
                let request: ResumeRequest = frame.decode()?;
                let id = request.transfer_id;
                let outcome = handle_resume(session, cfg, request).await?;
                cfg.approver.finished(session.peer(), &id, &outcome);
                outcomes.push(outcome);
            }
            FrameType::ClipPull => {
                let pull: ClipPull = frame.decode()?;
                serve_pull(session, cfg, &pull).await?;
                return Ok(outcomes);
            }
            _ => return session.violation().await.map_err(TransferError::from),
        }
    }
}

/// Sends the files of a `Files` item this device offered, as an ordinary
/// transfer in the same session. A clip that has gone, or was never offered
/// to this peer, gets a goodbye and nothing else.
async fn serve_pull<S>(
    session: &mut Session<S>,
    cfg: &ReceiveConfig,
    pull: &ClipPull,
) -> Result<(), TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let peer = *session.peer();
    let found = cfg
        .clip
        .as_ref()
        .filter(|_| cfg.budgets.allow_offer(&peer))
        .and_then(|sink| sink.files_for(&peer, &pull.clip_id, pull.item));
    let Some((mut outgoing, state)) = found else {
        session.goodbye().await;
        return Ok(());
    };
    // Each paste is a transfer of its own, so two never share a journal.
    outgoing.transfer_id = random_bytes().map_err(|_| TransferError::BadOffer)?;
    match send_with(session, outgoing, &state, &Control::new()).await {
        // The receiver turning it down is an answer, not a failure here.
        Ok(_) | Err(TransferError::Declined(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Asks a member for the files of a `Files` item it offered, and receives
/// them under `cfg` as an ordinary transfer. Only a transfer whose manifest
/// hashes to `manifest`, the one the clip listed, is accepted.
///
/// # Errors
/// Returns an error if the session fails or the member breaks the protocol.
pub async fn pull<S>(
    session: &mut Session<S>,
    request: &ClipPull,
    manifest: [u8; 32],
    cfg: &ReceiveConfig,
) -> Result<OfferOutcome, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    write_bounded(session, FrameType::ClipPull, request).await?;
    session.flush().await?;
    let frame = match session.next_frame().await {
        Ok(frame) => frame,
        Err(SessionError::Closed) => return Ok(OfferOutcome::Declined(code::CANCELLED)),
        Err(e) => return Err(e.into()),
    };
    match frame.kind {
        FrameType::Goodbye => Ok(OfferOutcome::Declined(code::CANCELLED)),
        FrameType::Offer => {
            let offer: Offer = frame.decode()?;
            let id = offer.transfer_id;
            let outcome = receive_offer(session, cfg, offer, Some(manifest)).await?;
            cfg.approver.finished(session.peer(), &id, &outcome);
            Ok(outcome)
        }
        _ => session.violation().await.map_err(TransferError::from),
    }
}

/// Exposes the frame size used for streaming, for benchmarks.
pub const CHUNK_PAYLOAD: usize = if CHUNK_SIZE + FILE_DATA_HEADER <= MAX_DATA_PAYLOAD {
    CHUNK_SIZE
} else {
    MAX_DATA_PAYLOAD - FILE_DATA_HEADER
};

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::names::TargetOs;

    fn folder(index: u32) -> Item {
        Item {
            index,
            path: vec![format!("d{index}")],
            kind: ItemKind::Directory,
            size: 0,
            mtime_ms: None,
            created_ms: None,
            group: None,
        }
    }

    #[test]
    fn empty_folders_count_against_a_peers_limits() {
        let items: Vec<Item> = (0..3).map(folder).collect();
        let valid = validate_manifest(&items, 0, TargetOs::Other).unwrap();
        assert_eq!(offer_cost(&valid, 120), (120, 3));
    }
}
