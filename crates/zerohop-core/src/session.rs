//! An authenticated connection between two members (protocol sections 7 and 9.1).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufWriter, ReadHalf, WriteHalf,
};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::circle::{Circle, CircleError};
use crate::ledger::{AuthorHead, DeviceId, MAX_HELD, Platform, SignedEvent};
use crate::pairing::MAX_NAME_BYTES;
use crate::wire::{
    DiscoveryKey, ErrorMsg, Frame, FrameType, Hello, LedgerEvents, LedgerHave, WireError, code,
    read_frame, write_msg,
};

/// Silence after which a `Ping` is sent.
const PING_AFTER: Duration = Duration::from_secs(15);
/// How many silent intervals end a session.
const MAX_SILENT_INTERVALS: u32 = 3;
/// How long one write may stay blocked by backpressure.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(120);

const SOFTWARE: &str = concat!("zerohop/", env!("CARGO_PKG_VERSION"));
const HEADS_PER_FRAME: usize = 2000;
const EVENTS_PER_FRAME: usize = 1000;
const WRITE_BUFFER: usize = 256 * 1024;
const INBOUND_FRAMES: usize = 8;
const REMOVAL_LINGER: Duration = Duration::from_secs(2);
const REMOVAL_MAX_DISCARD: usize = 1 << 20;
/// No honest peer can have more to say in one direction of a sync.
const MAX_SYNC_ITEMS: usize = MAX_HELD;

/// A circle shared between connections.
pub type CircleHandle = Arc<Mutex<Circle>>;

/// Errors on a session.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// A frame could not be read or written.
    #[error(transparent)]
    Wire(#[from] WireError),
    /// The circle could not be read or saved.
    #[error(transparent)]
    Circle(#[from] CircleError),
    /// The connection ended.
    #[error("the connection was closed")]
    Closed,
    /// Nothing arrived for too long.
    #[error("the other device stopped responding")]
    Timeout,
    /// The peer sent a frame that is invalid here.
    #[error("the other device broke the protocol")]
    Protocol,
    /// The peer reported an error.
    #[error("the other device reported error {0}")]
    Remote(u16),
    /// The peer does not belong to the circle.
    #[error("the other device is not a member of this circle")]
    NotMember,
    /// This device has been removed from the circle.
    #[error("this device has been removed from the circle")]
    Removed,
    /// The circle state lock was poisoned by a crash in another task.
    #[error("internal state error")]
    Poisoned,
}

/// Locks a circle.
///
/// # Errors
/// Returns [`SessionError::Poisoned`] if another task panicked while holding it.
pub fn lock(circle: &CircleHandle) -> Result<MutexGuard<'_, Circle>, SessionError> {
    circle.lock().map_err(|_| SessionError::Poisoned)
}

/// What this device says about itself.
#[derive(Clone, Debug)]
pub struct LocalInfo {
    /// Display name.
    pub name: String,
    /// Platform.
    pub platform: Platform,
}

/// Something that happened on the connection.
pub enum Incoming {
    /// A frame arrived.
    Frame(Result<Frame, WireError>),
    /// The connection ended.
    Closed,
    /// Nothing arrived for a while.
    Silent,
    /// Membership changed and the peer is no longer in the circle.
    PeerRemoved,
}

/// An authenticated connection with a member.
pub struct Session<S> {
    frames: mpsc::Receiver<Result<Frame, WireError>>,
    /// The buffered write side of the connection.
    pub writer: BufWriter<WriteHalf<S>>,
    peer: DeviceId,
    members: watch::Receiver<BTreeSet<DeviceId>>,
    silent: u32,
    reader: JoinHandle<()>,
}

impl<S> Drop for Session<S> {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

fn spawn_reader<S>(
    mut read: ReadHalf<S>,
) -> (mpsc::Receiver<Result<Frame, WireError>>, JoinHandle<()>)
where
    S: AsyncRead + Send + 'static,
{
    let (tx, rx) = mpsc::channel(INBOUND_FRAMES);
    let task = tokio::spawn(async move {
        loop {
            let frame = read_frame(&mut read).await;
            let stop = frame.is_err();
            if tx.send(frame).await.is_err() || stop {
                return;
            }
        }
    });
    (rx, task)
}

impl<S> Session<S>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    /// Exchanges `Hello` and syncs the ledger.
    ///
    /// # Errors
    /// Returns an error if the peer is in another circle, is not a member, or
    /// if this device turns out to have been removed.
    pub async fn establish(
        stream: S,
        peer: DeviceId,
        circle: &CircleHandle,
        info: &LocalInfo,
        members: watch::Receiver<BTreeSet<DeviceId>>,
    ) -> Result<Self, SessionError> {
        let (read, write) = tokio::io::split(stream);
        let (frames, reader) = spawn_reader(read);
        let mut session = Self {
            frames,
            writer: BufWriter::with_capacity(WRITE_BUFFER, write),
            peer,
            members,
            silent: 0,
            reader,
        };
        session.handshake(circle, info).await?;
        Ok(session)
    }

    async fn handshake(
        &mut self,
        circle: &CircleHandle,
        info: &LocalInfo,
    ) -> Result<(), SessionError> {
        let circle_id = *lock(circle)?.ledger().circle_id();
        let hello = Hello {
            software: SOFTWARE.to_string(),
            features: 0,
            name: info.name.clone(),
            platform: info.platform,
            circle_id,
        };
        write_msg(&mut self.writer, FrameType::Hello, &hello).await?;
        self.flush().await?;

        // In TLS 1.3 the client finishes first, so this `Hello` is what proves
        // the server accepted this device.
        let frame = self.next_frame().await?;
        if frame.kind != FrameType::Hello {
            return self.violation().await;
        }
        let theirs: Hello = frame.decode()?;
        if theirs.circle_id != circle_id || theirs.name.len() > MAX_NAME_BYTES {
            return self.violation().await;
        }

        // Two rounds, because a fork only becomes visible once both sides
        // hold the same number of events from its author.
        for _ in 0..2 {
            self.sync_round(circle).await?;
        }
        let (i_am_member, peer_is_member) = {
            let guard = lock(circle)?;
            let view = guard.ledger().membership();
            (
                view.members.contains(&guard.device_id()),
                view.members.contains(&self.peer),
            )
        };
        if !i_am_member {
            return Err(SessionError::Removed);
        }
        if !peer_is_member {
            self.send_error(code::NOT_MEMBER).await;
            return Err(SessionError::NotMember);
        }

        // Only here, once both are known to be members, so a removed device
        // never receives the key that replaced the one it had.
        let mine = lock(circle)?.own_discovery_key().clone();
        write_msg(&mut self.writer, FrameType::DiscoveryKey, &mine).await?;
        self.flush().await?;
        let frame = self.next_frame().await?;
        if frame.kind != FrameType::DiscoveryKey {
            return self.violation().await;
        }
        let theirs: DiscoveryKey = frame.decode()?;
        lock(circle)?.learn_discovery_key(self.peer, theirs)?;
        Ok(())
    }

    async fn sync_round(&mut self, circle: &CircleHandle) -> Result<(), SessionError> {
        let heads = lock(circle)?.ledger().heads();
        self.write_heads(&heads).await?;
        self.flush().await?;
        let peer_heads = self.read_heads(circle).await?;
        let events = lock(circle)?.events_for_peer(&peer_heads);
        self.write_events(&events).await?;
        self.flush().await?;
        let incoming = self.read_events().await?;
        if !incoming.is_empty() {
            lock(circle)?.apply(incoming)?;
        }
        Ok(())
    }

    async fn write_heads(&mut self, heads: &[AuthorHead]) -> Result<(), SessionError> {
        let mut chunks = heads.chunks(HEADS_PER_FRAME).peekable();
        loop {
            let chunk = chunks.next().unwrap_or(&[]);
            let msg = LedgerHave {
                authors: chunk.to_vec(),
            };
            write_msg(&mut self.writer, FrameType::LedgerHave, &msg).await?;
            if chunk.len() < HEADS_PER_FRAME {
                return Ok(());
            }
        }
    }

    async fn read_heads(&mut self, circle: &CircleHandle) -> Result<Vec<AuthorHead>, SessionError> {
        let mut heads = Vec::new();
        loop {
            let frame = self.next_frame_or_proof(circle).await?;
            if frame.kind != FrameType::LedgerHave {
                return self.violation().await;
            }
            let msg: LedgerHave = frame.decode()?;
            if heads.len() + msg.authors.len() > MAX_SYNC_ITEMS {
                return self.violation().await;
            }
            let last = msg.authors.len() < HEADS_PER_FRAME;
            heads.extend(msg.authors);
            if last {
                return Ok(heads);
            }
        }
    }

    /// Reads the next frame. A server that knows this device was removed
    /// sends the proof where the ledger position would be.
    async fn next_frame_or_proof(&mut self, circle: &CircleHandle) -> Result<Frame, SessionError> {
        let frame = self.next_frame().await?;
        if frame.kind != FrameType::LedgerEvents {
            return Ok(frame);
        }
        let mut events = frame.decode::<LedgerEvents>()?.events;
        loop {
            match self.next_frame().await {
                Ok(f) if f.kind == FrameType::LedgerEvents => {
                    events.extend(f.decode::<LedgerEvents>()?.events);
                    if events.len() > MAX_SYNC_ITEMS {
                        return Err(SessionError::Protocol);
                    }
                }
                Ok(_) => return Err(SessionError::Protocol),
                Err(SessionError::Remote(code::NOT_MEMBER) | SessionError::Closed) => break,
                Err(e) => return Err(e),
            }
        }
        let mut guard = lock(circle)?;
        guard.apply(events)?;
        let me = guard.device_id();
        if guard.is_member(&me) {
            Err(SessionError::NotMember)
        } else {
            Err(SessionError::Removed)
        }
    }

    async fn write_events(&mut self, events: &[SignedEvent]) -> Result<(), SessionError> {
        write_event_frames(&mut self.writer, events).await
    }

    async fn read_events(&mut self) -> Result<Vec<SignedEvent>, SessionError> {
        let mut events = Vec::new();
        loop {
            let frame = self.next_frame().await?;
            if frame.kind != FrameType::LedgerEvents {
                return self.violation().await;
            }
            let msg: LedgerEvents = frame.decode()?;
            if events.len() + msg.events.len() > MAX_SYNC_ITEMS {
                return self.violation().await;
            }
            let last = msg.events.len() < EVENTS_PER_FRAME;
            events.extend(msg.events);
            if last {
                return Ok(events);
            }
        }
    }

    /// Waits for something to happen. Safe to use inside `select!`.
    pub async fn wait(&mut self) -> Incoming {
        let peer = self.peer;
        tokio::select! {
            frame = self.frames.recv() => match frame {
                Some(frame) => Incoming::Frame(frame),
                None => Incoming::Closed,
            },
            () = tokio::time::sleep(PING_AFTER) => Incoming::Silent,
            () = wait_for_removal(&mut self.members, peer) => Incoming::PeerRemoved,
        }
    }

    /// Handles liveness traffic. Returns a frame if one needs processing.
    ///
    /// # Errors
    /// Returns an error if the connection has failed or the peer reported one.
    pub async fn process(&mut self, event: Incoming) -> Result<Option<Frame>, SessionError> {
        match event {
            Incoming::Closed => Err(SessionError::Closed),
            Incoming::PeerRemoved => {
                self.send_error(code::NOT_MEMBER).await;
                Err(SessionError::NotMember)
            }
            Incoming::Silent => {
                self.silent += 1;
                if self.silent >= MAX_SILENT_INTERVALS {
                    return Err(SessionError::Timeout);
                }
                write_msg(&mut self.writer, FrameType::Ping, &()).await?;
                self.flush().await?;
                Ok(None)
            }
            Incoming::Frame(frame) => {
                let frame = frame.map_err(|e| match e {
                    WireError::Io(io) if io.kind() == std::io::ErrorKind::UnexpectedEof => {
                        SessionError::Closed
                    }
                    other => SessionError::Wire(other),
                })?;
                self.silent = 0;
                match frame.kind {
                    FrameType::Ping => {
                        write_msg(&mut self.writer, FrameType::Pong, &()).await?;
                        self.flush().await?;
                        Ok(None)
                    }
                    FrameType::Pong => Ok(None),
                    FrameType::Error => {
                        let msg: ErrorMsg = frame.decode()?;
                        Err(SessionError::Remote(msg.code))
                    }
                    _ => Ok(Some(frame)),
                }
            }
        }
    }

    /// Waits for the next frame that needs processing.
    ///
    /// # Errors
    /// Returns an error if the connection fails or goes silent.
    pub async fn next_frame(&mut self) -> Result<Frame, SessionError> {
        loop {
            let event = self.wait().await;
            if let Some(frame) = self.process(event).await? {
                return Ok(frame);
            }
        }
    }

    /// Flushes buffered frames, bounded by the backpressure limit.
    ///
    /// # Errors
    /// Returns an error if the write fails or stays blocked too long.
    pub async fn flush(&mut self) -> Result<(), SessionError> {
        tokio::time::timeout(WRITE_TIMEOUT, self.writer.flush())
            .await
            .map_err(|_| SessionError::Timeout)?
            .map_err(WireError::Io)?;
        Ok(())
    }

    /// Reports a protocol violation and fails.
    ///
    /// # Errors
    /// Always returns [`SessionError::Protocol`].
    pub async fn violation<T>(&mut self) -> Result<T, SessionError> {
        self.send_error(code::PROTOCOL_VIOLATION).await;
        Err(SessionError::Protocol)
    }

    /// Best-effort delivery of an error frame.
    pub async fn send_error(&mut self, code: u16) {
        let msg = ErrorMsg {
            code,
            message: None,
        };
        let _ = write_msg(&mut self.writer, FrameType::Error, &msg).await;
        let _ = self.flush().await;
    }

    /// Ends the session politely.
    pub async fn goodbye(&mut self) {
        let _ = write_msg(&mut self.writer, FrameType::Goodbye, &()).await;
        let _ = self.flush().await;
        let _ = self.writer.shutdown().await;
    }

    /// The device on the other end.
    #[must_use]
    pub const fn peer(&self) -> &DeviceId {
        &self.peer
    }
}

/// Resolves only when `peer` has actually left the circle. A change that
/// concerns someone else is not silence on this connection.
async fn wait_for_removal(members: &mut watch::Receiver<BTreeSet<DeviceId>>, peer: DeviceId) {
    loop {
        if members.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
        if !members.borrow().contains(&peer) {
            return;
        }
    }
}

async fn write_event_frames<W>(writer: &mut W, events: &[SignedEvent]) -> Result<(), SessionError>
where
    W: AsyncWrite + Unpin,
{
    let mut chunks = events.chunks(EVENTS_PER_FRAME);
    loop {
        let chunk = chunks.next().unwrap_or(&[]);
        let msg = LedgerEvents {
            events: chunk.to_vec(),
        };
        write_msg(writer, FrameType::LedgerEvents, &msg).await?;
        if chunk.len() < EVENTS_PER_FRAME {
            return Ok(());
        }
    }
}

/// Tells a removed device why it is no longer welcome, without reading anything from it.
///
/// # Errors
/// Returns an error if the connection fails.
pub async fn deliver_removal<S>(
    mut stream: S,
    circle: &CircleHandle,
    info: &LocalInfo,
) -> Result<(), SessionError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (circle_id, events) = {
        let guard = lock(circle)?;
        (
            *guard.ledger().circle_id(),
            guard.ledger().proof_of_removal(),
        )
    };
    let hello = Hello {
        software: SOFTWARE.to_string(),
        features: 0,
        name: info.name.clone(),
        platform: info.platform,
        circle_id,
    };
    write_msg(&mut stream, FrameType::Hello, &hello).await?;
    write_event_frames(&mut stream, &events).await?;
    let msg = ErrorMsg {
        code: code::NOT_MEMBER,
        message: None,
    };
    write_msg(&mut stream, FrameType::Error, &msg).await?;
    stream.flush().await.map_err(WireError::Io)?;
    let _ = stream.shutdown().await;
    // Hanging up with its frames unread would reset the connection, and the
    // reset can destroy the proof on its way. They are thrown away, not parsed.
    let _ = tokio::time::timeout(REMOVAL_LINGER, async {
        let mut discard = [0u8; 4096];
        let mut left = REMOVAL_MAX_DISCARD;
        while left > 0 {
            match stream.read(&mut discard).await {
                Ok(n) if n > 0 => left = left.saturating_sub(n),
                _ => break,
            }
        }
    })
    .await;
    Ok(())
}
