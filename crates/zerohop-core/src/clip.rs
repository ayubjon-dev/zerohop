//! The shared clipboard (protocol section 12).
//!
//! A clip lives in memory only. Nothing here writes to disk or to a log, and
//! every buffer holding clipboard contents is wiped when it is dropped.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use zeroize::{Zeroize, Zeroizing};

use crate::circle::random_bytes;
use crate::ledger::DeviceId;
use crate::session::{Session, SessionError, WRITE_TIMEOUT};
use crate::transfer::{ReceiveConfig, TransferError};
use crate::wire::{
    ClipDataRef, ClipEnd, ClipFetch, ClipFormat, ClipItem, ClipOffer, FrameType, WireError,
    write_raw,
};

/// Longest a clip may stay on a device.
pub const MAX_CLIP_TTL: Duration = Duration::from_millis(120_000);
/// Most representations in one clip.
pub const MAX_CLIP_ITEMS: usize = 8;
/// Largest clip, and the most clipboard data held in memory at once.
pub const MAX_CLIP_BYTES: u64 = MEMORY_PERMITS as u64;
/// Most bytes carried inside an offer.
pub const MAX_CLIP_INLINE: usize = 128 * 1024;

const MEMORY_PERMITS: usize = 64 * 1024 * 1024;
const MAX_TTL_MS: u32 = 120_000;
const CHUNK: usize = 256 * 1024;

/// One representation of a copied clip. Wiped from memory when dropped.
pub struct ClipPart {
    /// What it holds.
    pub format: ClipFormat,
    /// The content.
    pub bytes: Zeroizing<Vec<u8>>,
}

/// A clip that arrived and passed every check.
pub struct ReceivedClip {
    /// Time left before it should be gone from this device.
    pub ttl: Duration,
    /// The representations the owner's settings allow.
    pub parts: Vec<ClipPart>,
}

/// Receives clips from other devices.
pub trait ClipSink: Send + Sync {
    /// Whether the owner allows this representation from this device.
    fn accepts(&self, peer: &DeviceId, format: ClipFormat, size: u64) -> bool;
    /// Takes a clip. The sink becomes responsible for it.
    fn deliver(&self, peer: &DeviceId, clip: ReceivedClip);
}

/// A clip ready to be offered.
pub struct OutgoingClip {
    clip_id: [u8; 16],
    ttl_ms: u32,
    parts: Vec<ClipPart>,
}

impl OutgoingClip {
    /// Prepares a clip.
    ///
    /// # Errors
    /// Returns [`TransferError::BadOffer`] if it breaks a protocol limit or
    /// no secure randomness is available.
    pub fn new(parts: Vec<ClipPart>, ttl: Duration) -> Result<Self, TransferError> {
        let ttl_ms = u32::try_from(ttl.as_millis()).map_err(|_| TransferError::BadOffer)?;
        let mut total = 0u64;
        for part in &parts {
            let size = part.bytes.len() as u64;
            if size == 0 {
                return Err(TransferError::BadOffer);
            }
            total = total.saturating_add(size);
        }
        if ttl_ms == 0
            || ttl_ms > MAX_TTL_MS
            || parts.is_empty()
            || parts.len() > MAX_CLIP_ITEMS
            || total > MAX_CLIP_BYTES
        {
            return Err(TransferError::BadOffer);
        }
        Ok(Self {
            clip_id: random_bytes().map_err(|_| TransferError::BadOffer)?,
            ttl_ms,
            parts,
        })
    }

    const fn ttl(&self) -> Duration {
        Duration::from_millis(self.ttl_ms as u64)
    }

    fn offer(&self) -> ClipOffer {
        let mut room = MAX_CLIP_INLINE;
        let items = self
            .parts
            .iter()
            .map(|part| {
                let fits = part.bytes.len() <= room;
                if fits {
                    room -= part.bytes.len();
                }
                ClipItem {
                    format: part.format,
                    size: part.bytes.len() as u64,
                    inline: fits.then(|| part.bytes.to_vec()),
                }
            })
            .collect();
        ClipOffer {
            clip_id: self.clip_id,
            ttl_ms: self.ttl_ms,
            items,
        }
    }
}

/// How much clipboard data the receiving side holds across all devices.
pub struct ClipMemory(Arc<Semaphore>);

impl Default for ClipMemory {
    fn default() -> Self {
        Self(Arc::new(Semaphore::new(MEMORY_PERMITS)))
    }
}

impl ClipMemory {
    fn reserve(&self, bytes: u64) -> Option<OwnedSemaphorePermit> {
        let bytes = u32::try_from(bytes).ok()?;
        self.0.clone().try_acquire_many_owned(bytes).ok()
    }
}

fn valid_offer(offer: &ClipOffer) -> bool {
    if offer.ttl_ms == 0
        || offer.ttl_ms > MAX_TTL_MS
        || offer.items.is_empty()
        || offer.items.len() > MAX_CLIP_ITEMS
    {
        return false;
    }
    let mut total = 0u64;
    let mut inline = 0usize;
    for item in &offer.items {
        if item.size == 0 || item.size > MAX_CLIP_BYTES {
            return false;
        }
        total = total.saturating_add(item.size);
        if let Some(bytes) = &item.inline {
            if bytes.len() as u64 != item.size {
                return false;
            }
            inline = inline.saturating_add(bytes.len());
        }
    }
    total <= MAX_CLIP_BYTES && inline <= MAX_CLIP_INLINE
}

/// Serializes a message holding clipboard contents and wipes the encoding.
async fn write_secret<S, T>(
    session: &mut Session<S>,
    kind: FrameType,
    msg: &T,
) -> Result<(), TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
    T: Serialize,
{
    let payload = Zeroizing::new(postcard::to_stdvec(msg).map_err(|_| WireError::Malformed)?);
    tokio::time::timeout(
        WRITE_TIMEOUT,
        write_raw(&mut session.writer, kind, &payload),
    )
    .await
    .map_err(|_| SessionError::Timeout)??;
    Ok(())
}

/// Grows without ever leaving a copy of the contents behind in freed memory.
fn extend_wiping(buf: &mut Zeroizing<Vec<u8>>, bytes: &[u8], limit: usize) {
    let needed = buf.len() + bytes.len();
    if needed > buf.capacity() {
        let capacity = needed.max(buf.capacity() * 2).min(limit.max(needed));
        let mut grown = Zeroizing::new(Vec::with_capacity(capacity));
        grown.extend_from_slice(buf);
        *buf = grown;
    }
    buf.extend_from_slice(bytes);
}

async fn fetch_item<S>(
    session: &mut Session<S>,
    clip_id: [u8; 16],
    index: u32,
    size: u64,
) -> Result<Zeroizing<Vec<u8>>, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let request = ClipFetch {
        clip_id,
        item: index,
    };
    write_secret(session, FrameType::ClipFetch, &request).await?;
    session.flush().await?;
    let limit = usize::try_from(size).map_err(|_| TransferError::BadOffer)?;
    // Sized by what actually arrives, never by what the sender claimed.
    let mut out = Zeroizing::new(Vec::new());
    let mut hasher = blake3::Hasher::new();
    loop {
        let mut frame = session.next_frame().await?;
        match frame.kind {
            FrameType::ClipData => {
                let decoded = frame.decode::<crate::wire::ClipData>();
                frame.payload.zeroize();
                let data = decoded?;
                let arrived = out.len() as u64;
                let fits = arrived
                    .checked_add(data.bytes.len() as u64)
                    .is_some_and(|end| end <= size);
                if data.clip_id != clip_id
                    || data.item != index
                    || data.offset != arrived
                    || data.bytes.is_empty()
                    || !fits
                {
                    return session.violation().await.map_err(TransferError::from);
                }
                hasher.update(&data.bytes);
                extend_wiping(&mut out, &data.bytes, limit);
            }
            FrameType::ClipEnd => {
                let end: ClipEnd = frame.decode()?;
                if end.clip_id != clip_id
                    || end.item != index
                    || out.len() as u64 != size
                    || hasher.finalize() != blake3::Hash::from_bytes(end.hash)
                {
                    return session.violation().await.map_err(TransferError::from);
                }
                return Ok(out);
            }
            _ => return session.violation().await.map_err(TransferError::from),
        }
    }
}

/// Takes one offered clip. Anything the owner does not allow is left behind
/// without telling the sender why; the session always ends here.
pub(crate) async fn receive<S>(
    session: &mut Session<S>,
    cfg: &ReceiveConfig,
    mut offer: ClipOffer,
) -> Result<(), TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    if !valid_offer(&offer) {
        return session.violation().await.map_err(TransferError::from);
    }
    let started = Instant::now();
    let peer = *session.peer();
    let wanted: Vec<usize> = match cfg.clip.as_ref() {
        Some(sink) if cfg.budgets.allow_clip(&peer) => offer
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| sink.accepts(&peer, item.format, item.size))
            .map(|(index, _)| index)
            .collect(),
        _ => Vec::new(),
    };
    let needed: u64 = wanted
        .iter()
        .filter_map(|index| offer.items.get(*index))
        .map(|item| item.size)
        .sum();
    let memory = cfg.budgets.clip_memory().reserve(needed);
    let (Some(sink), Some(_memory), false) = (cfg.clip.as_ref(), memory, wanted.is_empty()) else {
        session.goodbye().await;
        return Ok(());
    };

    let ttl = Duration::from_millis(u64::from(offer.ttl_ms));
    let clip_id = offer.clip_id;
    let fetching = async {
        let mut parts = Vec::with_capacity(wanted.len());
        for index in &wanted {
            let Some(item) = offer.items.get_mut(*index) else {
                return Err(TransferError::BadOffer);
            };
            let format = item.format;
            let bytes = match item.inline.take() {
                Some(bytes) => Zeroizing::new(bytes),
                None => {
                    let number = u32::try_from(*index).map_err(|_| TransferError::BadOffer)?;
                    fetch_item(session, clip_id, number, item.size).await?
                }
            };
            parts.push(ClipPart { format, bytes });
        }
        Ok(parts)
    };
    let parts = tokio::time::timeout(ttl, fetching)
        .await
        .map_err(|_| SessionError::Timeout)??;
    session.goodbye().await;
    sink.deliver(
        &peer,
        ReceivedClip {
            ttl: ttl.saturating_sub(started.elapsed()),
            parts,
        },
    );
    Ok(())
}

async fn send_part<S>(
    session: &mut Session<S>,
    clip_id: [u8; 16],
    index: u32,
    bytes: &[u8],
) -> Result<(), TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let mut offset = 0u64;
    for chunk in bytes.chunks(CHUNK) {
        let data = ClipDataRef {
            clip_id,
            item: index,
            offset,
            bytes: chunk,
        };
        write_secret(session, FrameType::ClipData, &data).await?;
        offset += chunk.len() as u64;
    }
    let end = ClipEnd {
        clip_id,
        item: index,
        hash: *blake3::hash(bytes).as_bytes(),
    };
    write_secret(session, FrameType::ClipEnd, &end).await?;
    session.flush().await?;
    Ok(())
}

/// Offers a clip and answers fetches until the other device is finished.
///
/// # Errors
/// Returns an error if the session fails, the other device breaks the
/// protocol, or it takes longer than the clip's lifetime.
pub async fn offer<S>(session: &mut Session<S>, clip: &OutgoingClip) -> Result<(), TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let offer = clip.offer();
    write_secret(session, FrameType::ClipOffer, &offer).await?;
    session.flush().await?;
    let mut sent = vec![false; clip.parts.len()];
    let serving = async {
        loop {
            let frame = match session.next_frame().await {
                Ok(frame) => frame,
                Err(SessionError::Closed) => return Ok(()),
                Err(e) => return Err(TransferError::from(e)),
            };
            match frame.kind {
                FrameType::Goodbye => return Ok(()),
                FrameType::ClipFetch => {
                    let fetch: ClipFetch = frame.decode()?;
                    let index = usize::try_from(fetch.item).unwrap_or(usize::MAX);
                    let inline = offer.items.get(index).is_none_or(|i| i.inline.is_some());
                    let (Some(part), Some(done)) = (clip.parts.get(index), sent.get_mut(index))
                    else {
                        return session.violation().await.map_err(TransferError::from);
                    };
                    if fetch.clip_id != clip.clip_id || inline || *done {
                        return session.violation().await.map_err(TransferError::from);
                    }
                    *done = true;
                    send_part(session, clip.clip_id, fetch.item, &part.bytes).await?;
                }
                _ => return session.violation().await.map_err(TransferError::from),
            }
        }
    };
    tokio::time::timeout(clip.ttl(), serving)
        .await
        .map_err(|_| SessionError::Timeout)?
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn part(size: usize) -> ClipPart {
        ClipPart {
            format: ClipFormat::Text,
            bytes: Zeroizing::new(vec![b'a'; size]),
        }
    }

    fn item(size: u64, inline: Option<Vec<u8>>) -> ClipItem {
        ClipItem {
            format: ClipFormat::Text,
            size,
            inline,
        }
    }

    fn offer_of(ttl_ms: u32, items: Vec<ClipItem>) -> ClipOffer {
        ClipOffer {
            clip_id: [1; 16],
            ttl_ms,
            items,
        }
    }

    #[test]
    fn accepts_an_offer_inside_every_limit() {
        let most = MAX_CLIP_INLINE as u64;
        let offer = offer_of(
            MAX_TTL_MS,
            vec![
                item(most, Some(vec![0; MAX_CLIP_INLINE])),
                item(MAX_CLIP_BYTES - most, None),
            ],
        );
        assert!(valid_offer(&offer));
    }

    #[test]
    fn refuses_offers_outside_the_limits() {
        let bad = [
            offer_of(0, vec![item(1, None)]),
            offer_of(MAX_TTL_MS + 1, vec![item(1, None)]),
            offer_of(1000, Vec::new()),
            offer_of(1000, vec![item(0, None)]),
            offer_of(1000, vec![item(MAX_CLIP_BYTES + 1, None)]),
            offer_of(1000, (0..=MAX_CLIP_ITEMS).map(|_| item(1, None)).collect()),
            offer_of(
                1000,
                vec![item(MAX_CLIP_BYTES, None), item(1, Some(vec![0]))],
            ),
            offer_of(1000, vec![item(5, Some(vec![0; 4]))]),
            offer_of(
                1000,
                vec![
                    item(MAX_CLIP_INLINE as u64, Some(vec![0; MAX_CLIP_INLINE])),
                    item(1, Some(vec![0])),
                ],
            ),
        ];
        for offer in &bad {
            assert!(!valid_offer(offer), "{offer:?}");
        }
    }

    #[test]
    fn sizes_that_overflow_are_refused() {
        let offer = offer_of(
            1000,
            vec![item(u64::MAX, None), item(u64::MAX, None), item(2, None)],
        );
        assert!(!valid_offer(&offer));
    }

    #[test]
    fn outgoing_clips_respect_the_same_limits() {
        let ttl = Duration::from_secs(1);
        assert!(OutgoingClip::new(vec![part(1)], ttl).is_ok());
        assert!(OutgoingClip::new(Vec::new(), ttl).is_err());
        assert!(OutgoingClip::new(vec![part(0)], ttl).is_err());
        assert!(OutgoingClip::new(vec![part(1)], Duration::ZERO).is_err());
        assert!(OutgoingClip::new(vec![part(1)], MAX_CLIP_TTL * 2).is_err());
        let many = (0..=MAX_CLIP_ITEMS).map(|_| part(1)).collect();
        assert!(OutgoingClip::new(many, ttl).is_err());
    }

    #[test]
    fn only_what_fits_travels_inside_the_offer() {
        let ttl = Duration::from_secs(1);
        let clip = OutgoingClip::new(
            vec![part(MAX_CLIP_INLINE), part(1), part(MAX_CLIP_INLINE + 1)],
            ttl,
        )
        .unwrap();
        let offer = clip.offer();
        assert!(offer.items[0].inline.is_some());
        assert!(offer.items[1].inline.is_none());
        assert!(offer.items[2].inline.is_none());
        assert!(valid_offer(&offer));
    }

    #[test]
    fn memory_is_shared_and_returned() {
        let memory = ClipMemory::default();
        let first = memory.reserve(MAX_CLIP_BYTES).unwrap();
        assert!(memory.reserve(1).is_none());
        drop(first);
        assert!(memory.reserve(MAX_CLIP_BYTES).is_some());
    }

    #[test]
    fn growing_keeps_every_byte_in_order() {
        let mut buf = Zeroizing::new(Vec::new());
        let mut expected = Vec::new();
        for n in 0u8..100 {
            let piece = vec![n; 1000];
            extend_wiping(&mut buf, &piece, 100_000);
            expected.extend_from_slice(&piece);
        }
        assert_eq!(*buf, expected);
        assert!(buf.capacity() <= 100_000);
    }
}
