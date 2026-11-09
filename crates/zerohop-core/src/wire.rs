//! Frames and messages on the wire (protocol section 9).

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroize;

use crate::ledger::{AuthorHead, CircleId, Platform, SignedEvent};

/// Largest `FileData` or `ClipData` payload.
pub const MAX_DATA_PAYLOAD: usize = 1 << 20;
/// Largest payload of any other frame.
pub const MAX_CONTROL_PAYLOAD: usize = 256 * 1024;
/// Largest payload accepted before pairing key confirmation succeeds.
pub const MAX_PRE_AUTH_PAYLOAD: usize = 128;
/// Bytes of `file_index` and `offset` that start a `FileData` payload.
pub const FILE_DATA_HEADER: usize = 12;

/// Errors from reading or writing frames.
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    /// The connection failed.
    #[error("connection error: {0}")]
    Io(#[from] std::io::Error),
    /// A frame was larger than its limit.
    #[error("frame too large")]
    FrameTooLarge,
    /// A mandatory frame type was not recognized.
    #[error("unknown frame type {0:#x}")]
    UnknownFrame(u8),
    /// A payload could not be decoded.
    #[error("malformed frame payload")]
    Malformed,
}

/// Every frame type in protocol version 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum FrameType {
    Hello = 0x01,
    Goodbye = 0x02,
    Error = 0x03,
    Ping = 0x04,
    Pong = 0x05,
    LedgerHave = 0x10,
    LedgerEvents = 0x11,
    DiscoveryKey = 0x12,
    Offer = 0x20,
    OfferItems = 0x21,
    Decision = 0x22,
    FileData = 0x23,
    SegmentEnd = 0x24,
    FileEnd = 0x25,
    Progress = 0x26,
    Cancel = 0x27,
    Complete = 0x28,
    Done = 0x29,
    ResumeRequest = 0x2A,
    ResumeState = 0x2B,
    ClipOffer = 0x30,
    ClipFetch = 0x31,
    ClipData = 0x32,
    ClipEnd = 0x33,
    PakeA = 0x40,
    PakeB = 0x41,
    Confirm = 0x42,
    SasCommit = 0x43,
    SasNonce = 0x44,
    SasReveal = 0x45,
    PeerInfo = 0x46,
    Approve = 0x47,
    WelcomeEnd = 0x48,
    WelcomeAck = 0x49,
}

impl FrameType {
    /// Maps a byte to a mandatory frame type.
    #[must_use]
    pub const fn from_byte(b: u8) -> Option<Self> {
        use FrameType as F;
        Some(match b {
            0x01 => F::Hello,
            0x02 => F::Goodbye,
            0x03 => F::Error,
            0x04 => F::Ping,
            0x05 => F::Pong,
            0x10 => F::LedgerHave,
            0x11 => F::LedgerEvents,
            0x12 => F::DiscoveryKey,
            0x20 => F::Offer,
            0x21 => F::OfferItems,
            0x22 => F::Decision,
            0x23 => F::FileData,
            0x24 => F::SegmentEnd,
            0x25 => F::FileEnd,
            0x26 => F::Progress,
            0x27 => F::Cancel,
            0x28 => F::Complete,
            0x29 => F::Done,
            0x2A => F::ResumeRequest,
            0x2B => F::ResumeState,
            0x30 => F::ClipOffer,
            0x31 => F::ClipFetch,
            0x32 => F::ClipData,
            0x33 => F::ClipEnd,
            0x40 => F::PakeA,
            0x41 => F::PakeB,
            0x42 => F::Confirm,
            0x43 => F::SasCommit,
            0x44 => F::SasNonce,
            0x45 => F::SasReveal,
            0x46 => F::PeerInfo,
            0x47 => F::Approve,
            0x48 => F::WelcomeEnd,
            0x49 => F::WelcomeAck,
            _ => return None,
        })
    }

    const fn payload_limit(self) -> usize {
        match self {
            Self::FileData | Self::ClipData => MAX_DATA_PAYLOAD,
            _ => MAX_CONTROL_PAYLOAD,
        }
    }
}

/// A received frame.
#[derive(Debug)]
pub struct Frame {
    /// The frame's type.
    pub kind: FrameType,
    /// The undecoded payload.
    pub payload: Vec<u8>,
}

impl Frame {
    /// Decodes the payload, refusing trailing bytes.
    ///
    /// # Errors
    /// Returns [`WireError::Malformed`] if the payload does not decode exactly.
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T, WireError> {
        decode(&self.payload)
    }
}

/// Decodes a control payload, refusing trailing bytes.
///
/// # Errors
/// Returns [`WireError::Malformed`] if the payload does not decode exactly.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, WireError> {
    let (value, rest) = postcard::take_from_bytes(bytes).map_err(|_| WireError::Malformed)?;
    if rest.is_empty() {
        Ok(value)
    } else {
        Err(WireError::Malformed)
    }
}

/// Reads one frame, ignoring optional extension frames.
///
/// Every length is checked against its limit before anything is allocated.
///
/// # Errors
/// Returns an error for a failed connection, an oversized frame, or an
/// unknown mandatory frame type.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Frame, WireError> {
    read_frame_limited(r, usize::MAX).await
}

/// Reads one frame with an additional limit on the payload size.
///
/// # Errors
/// Returns an error for a failed connection, an oversized frame, or an
/// unknown mandatory frame type.
pub async fn read_frame_limited<R: AsyncRead + Unpin>(
    r: &mut R,
    extra_limit: usize,
) -> Result<Frame, WireError> {
    loop {
        let mut header = [0u8; 4];
        r.read_exact(&mut header).await?;
        let [ty, a, b, c] = header;
        let len = u32::from_be_bytes([0, a, b, c]) as usize;
        let kind = FrameType::from_byte(ty);
        if kind.is_none() && ty < 0x80 {
            return Err(WireError::UnknownFrame(ty));
        }
        let limit = kind.map_or(MAX_CONTROL_PAYLOAD, FrameType::payload_limit);
        if len > limit.min(extra_limit) {
            return Err(WireError::FrameTooLarge);
        }
        let mut payload = vec![0u8; len];
        r.read_exact(&mut payload).await?;
        if let Some(kind) = kind {
            return Ok(Frame { kind, payload });
        }
    }
}

/// Writes one frame with a raw payload. Does not flush.
///
/// # Errors
/// Returns an error for a failed connection or an oversized payload.
pub async fn write_raw<W: AsyncWrite + Unpin>(
    w: &mut W,
    kind: FrameType,
    payload: &[u8],
) -> Result<(), WireError> {
    if payload.len() > kind.payload_limit() {
        return Err(WireError::FrameTooLarge);
    }
    let len = u32::try_from(payload.len())
        .map_err(|_| WireError::FrameTooLarge)?
        .to_be_bytes();
    let [_, a, b, c] = len;
    w.write_all(&[kind as u8, a, b, c]).await?;
    w.write_all(payload).await?;
    Ok(())
}

/// Encodes and writes one control frame. Does not flush.
///
/// # Errors
/// Returns an error for a failed connection or a message that cannot be encoded.
pub async fn write_msg<W: AsyncWrite + Unpin, T: Serialize>(
    w: &mut W,
    kind: FrameType,
    msg: &T,
) -> Result<(), WireError> {
    let payload = postcard::to_stdvec(msg).map_err(|_| WireError::Malformed)?;
    write_raw(w, kind, &payload).await
}

/// Builds a `FileData` payload header.
#[must_use]
pub fn file_data_header(file_index: u32, offset: u64) -> [u8; FILE_DATA_HEADER] {
    let mut h = [0u8; FILE_DATA_HEADER];
    let (a, b) = h.split_at_mut(4);
    a.copy_from_slice(&file_index.to_le_bytes());
    b.copy_from_slice(&offset.to_le_bytes());
    h
}

/// Splits a `FileData` payload into its index, offset and data.
///
/// # Errors
/// Returns [`WireError::Malformed`] if the payload is shorter than its header.
pub fn parse_file_data(payload: &[u8]) -> Result<(u32, u64, &[u8]), WireError> {
    let (head, data) = payload
        .split_at_checked(FILE_DATA_HEADER)
        .ok_or(WireError::Malformed)?;
    let (a, b) = head.split_at(4);
    let index = u32::from_le_bytes(a.try_into().map_err(|_| WireError::Malformed)?);
    let offset = u64::from_le_bytes(b.try_into().map_err(|_| WireError::Malformed)?);
    Ok((index, offset, data))
}

/// Error codes from protocol section 14.
#[allow(missing_docs)]
pub mod code {
    pub const PROTOCOL_VIOLATION: u16 = 1;
    pub const FRAME_TOO_LARGE: u16 = 2;
    pub const DUPLICATE_SESSION: u16 = 3;
    pub const NOT_MEMBER: u16 = 4;
    pub const PAIRING_FAILED: u16 = 10;
    pub const PAIRING_DECLINED: u16 = 11;
    pub const PAIRING_UNAVAILABLE: u16 = 12;
    pub const DECLINED: u16 = 20;
    pub const INVALID_PATH: u16 = 21;
    pub const NO_SPACE: u16 = 22;
    pub const RATE_LIMITED: u16 = 23;
    pub const TOO_LARGE: u16 = 24;
    pub const INTEGRITY_FAILED: u16 = 25;
    pub const SOURCE_CHANGED: u16 = 26;
    pub const UNKNOWN_TRANSFER: u16 = 27;
    pub const CANCELLED: u16 = 28;
}

/// A device's key for recognizing its announcements (protocol section 6).
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryKey {
    pub generation: u32,
    pub key: [u8; 32],
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorMsg {
    pub code: u16,
    pub message: Option<String>,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub software: String,
    pub features: u64,
    pub name: String,
    pub platform: Platform,
    pub circle_id: CircleId,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerHave {
    pub authors: Vec<AuthorHead>,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEvents {
    pub events: Vec<SignedEvent>,
}

/// Whether an item is a file or a directory.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemKind {
    File,
    Directory,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub index: u32,
    pub path: Vec<String>,
    pub kind: ItemKind,
    pub size: u64,
    pub mtime_ms: Option<i64>,
    pub created_ms: Option<i64>,
    pub group: Option<u32>,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub transfer_id: [u8; 16],
    pub item_count: u32,
    pub total_bytes: u64,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferItems {
    pub items: Vec<Item>,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub transfer_id: [u8; 16],
    pub accept: bool,
    pub reason: Option<u16>,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentEnd {
    pub file_index: u32,
    pub segment: u32,
    pub hash: [u8; 32],
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEnd {
    pub file_index: u32,
    pub root: [u8; 32],
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub transfer_id: [u8; 16],
    pub file_index: u32,
    pub segment: u32,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Complete {
    pub transfer_id: [u8; 16],
    pub files: u32,
    pub bytes: u64,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    pub transfer_id: [u8; 16],
    pub status: u16,
    pub published: u32,
    pub held: u32,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cancel {
    pub transfer_id: [u8; 16],
    pub reason: u16,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeRequest {
    pub transfer_id: [u8; 16],
    pub manifest: [u8; 32],
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeState {
    pub transfer_id: [u8; 16],
    pub next_file: u32,
    pub next_segment: u32,
    pub prefix: [u8; 32],
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PakeA {
    #[serde(with = "BigArray")]
    pub msg_a: [u8; 33],
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PakeB {
    #[serde(with = "BigArray")]
    pub msg_b: [u8; 33],
    pub tag: [u8; 32],
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confirm {
    pub tag: [u8; 32],
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SasCommit {
    pub commit: [u8; 32],
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SasNonce {
    pub nonce: [u8; 32],
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub name: String,
    pub platform: Platform,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WelcomeEnd {
    pub event_count: u32,
}

/// What one representation of a copied clip holds.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClipFormat {
    Text,
    Html,
    Rtf,
    Png,
    Jpeg,
    Files,
}

/// One representation of a clip. Its contents never reach a log and are
/// wiped when it is dropped.
#[allow(missing_docs)]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipItem {
    pub format: ClipFormat,
    pub size: u64,
    pub inline: Option<Vec<u8>>,
}

impl fmt::Debug for ClipItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClipItem")
            .field("format", &self.format)
            .field("size", &self.size)
            .field("inline", &self.inline.as_ref().map(Vec::len))
            .finish()
    }
}

impl Drop for ClipItem {
    fn drop(&mut self) {
        self.inline.zeroize();
    }
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipOffer {
    pub clip_id: [u8; 16],
    pub ttl_ms: u32,
    pub items: Vec<ClipItem>,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipFetch {
    pub clip_id: [u8; 16],
    pub item: u32,
}

/// Part of a clip's contents. Never reaches a log; wiped when dropped.
#[allow(missing_docs)]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipData {
    pub clip_id: [u8; 16],
    pub item: u32,
    pub offset: u64,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for ClipData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClipData")
            .field("item", &self.item)
            .field("offset", &self.offset)
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl Drop for ClipData {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

/// The same encoding as [`ClipData`], without another copy of the contents.
#[allow(missing_docs)]
#[derive(Serialize)]
pub struct ClipDataRef<'a> {
    pub clip_id: [u8; 16],
    pub item: u32,
    pub offset: u64,
    pub bytes: &'a [u8],
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipEnd {
    pub clip_id: [u8; 16],
    pub item: u32,
    pub hash: [u8; 32],
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[tokio::test]
    async fn round_trips_a_message() {
        let mut buf = Vec::new();
        let msg = Decision {
            transfer_id: [3; 16],
            accept: true,
            reason: None,
        };
        write_msg(&mut buf, FrameType::Decision, &msg)
            .await
            .unwrap();
        let frame = read_frame(&mut buf.as_slice()).await.unwrap();
        assert_eq!(frame.kind, FrameType::Decision);
        assert_eq!(frame.decode::<Decision>().unwrap(), msg);
    }

    #[tokio::test]
    async fn refuses_an_oversized_control_frame_before_allocating() {
        let header = [0x01, 0x04, 0x00, 0x01];
        let err = read_frame(&mut header.as_slice()).await.unwrap_err();
        assert!(matches!(err, WireError::FrameTooLarge));
    }

    #[tokio::test]
    async fn refuses_an_oversized_data_frame() {
        let header = [0x23, 0x10, 0x00, 0x01];
        let err = read_frame(&mut header.as_slice()).await.unwrap_err();
        assert!(matches!(err, WireError::FrameTooLarge));
    }

    #[tokio::test]
    async fn applies_the_pre_authentication_limit() {
        let header = [0x40, 0x00, 0x00, 0x81];
        let err = read_frame_limited(&mut header.as_slice(), MAX_PRE_AUTH_PAYLOAD)
            .await
            .unwrap_err();
        assert!(matches!(err, WireError::FrameTooLarge));
    }

    #[tokio::test]
    async fn refuses_an_unknown_mandatory_frame() {
        let header = [0x7F, 0x00, 0x00, 0x00];
        let err = read_frame(&mut header.as_slice()).await.unwrap_err();
        assert!(matches!(err, WireError::UnknownFrame(0x7F)));
    }

    #[tokio::test]
    async fn skips_an_unknown_extension_frame() {
        let bytes = [0x90, 0x00, 0x00, 0x01, 0xAA, 0x04, 0x00, 0x00, 0x00];
        let frame = read_frame(&mut bytes.as_slice()).await.unwrap();
        assert_eq!(frame.kind, FrameType::Ping);
    }

    #[test]
    fn clipboard_contents_never_reach_debug_output() {
        let secret = b"hunter2hunter2".to_vec();
        let item = ClipItem {
            format: ClipFormat::Text,
            size: secret.len() as u64,
            inline: Some(secret.clone()),
        };
        let data = ClipData {
            clip_id: [1; 16],
            item: 0,
            offset: 0,
            bytes: secret,
        };
        let shown = format!("{item:?} {data:?}");
        assert!(!shown.contains("104, 117, 110"), "{shown}");
        assert!(!shown.contains("hunter2"), "{shown}");
    }

    #[test]
    fn the_borrowed_clip_chunk_encodes_like_the_owned_one() {
        let owned = ClipData {
            clip_id: [7; 16],
            item: 3,
            offset: 300,
            bytes: vec![1, 2, 3, 250],
        };
        let borrowed = ClipDataRef {
            clip_id: owned.clip_id,
            item: owned.item,
            offset: owned.offset,
            bytes: &owned.bytes,
        };
        assert_eq!(
            postcard::to_stdvec(&owned).unwrap(),
            postcard::to_stdvec(&borrowed).unwrap()
        );
    }

    #[test]
    fn clip_formats_use_the_numbers_in_the_protocol() {
        for (format, number) in [
            (ClipFormat::Text, 0u8),
            (ClipFormat::Html, 1),
            (ClipFormat::Rtf, 2),
            (ClipFormat::Png, 3),
            (ClipFormat::Jpeg, 4),
            (ClipFormat::Files, 5),
        ] {
            assert_eq!(postcard::to_stdvec(&format).unwrap(), vec![number]);
        }
    }

    #[test]
    fn refuses_trailing_bytes() {
        let mut bytes = postcard::to_stdvec(&WelcomeEnd { event_count: 1 }).unwrap();
        bytes.push(0);
        assert!(decode::<WelcomeEnd>(&bytes).is_err());
    }

    #[test]
    fn parses_file_data() {
        let mut payload = file_data_header(7, 1 << 40).to_vec();
        payload.extend_from_slice(b"abc");
        assert_eq!(
            parse_file_data(&payload).unwrap(),
            (7, 1 << 40, &b"abc"[..])
        );
    }

    #[test]
    fn refuses_short_file_data() {
        assert!(parse_file_data(&[0; 11]).is_err());
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic_the_decoders(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = decode::<Hello>(&bytes);
            let _ = decode::<OfferItems>(&bytes);
            let _ = decode::<LedgerEvents>(&bytes);
            let _ = decode::<PakeB>(&bytes);
            let _ = parse_file_data(&bytes);
        }
    }
}
