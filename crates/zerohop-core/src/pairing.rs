//! Adding a device to a circle (protocol section 8).
//!
//! There is one protocol. A QR code and a typed code differ only in how the
//! shared secret reaches the other screen.

use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{Instant, timeout, timeout_at};
use zeroize::Zeroizing;

use crate::address::LocalNetworks;
use crate::circle::{Circle, CircleError, KeySealing, random_bytes};
use crate::ledger::{CircleId, DeviceId, EventKind, Ledger, MAX_HELD, Platform, circle_id_for};
use crate::names;
use crate::session::CircleHandle;
use crate::tls::{self, ALPN_PAIRING, PeerRule, TlsError, peer_id};
use crate::wire::{
    Confirm, ErrorMsg, Frame, FrameType, LedgerEvents, MAX_PRE_AUTH_PAYLOAD, PakeA, PakeB,
    PeerInfo, SasCommit, SasNonce, WelcomeEnd, WireError, code, read_frame, read_frame_limited,
    write_msg,
};

const CONFIRM_INFO: &[u8] = b"zerohop-v1/pair-confirm";
const SAS_INFO: &[u8] = b"zerohop-v1/pair-sas";
const SAS_COMMIT: &[u8] = b"zerohop-v1/sas-commit\0";
const EXPORTER_LABEL: &[u8] = b"EXPORTER-zerohop-pair-v1";
const EVENTS_PER_FRAME: usize = 1000;

/// How long pairing mode stays open.
pub const PAIRING_WINDOW: Duration = Duration::from_secs(120);
const STEP_TIMEOUT: Duration = Duration::from_secs(10);
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);
const ACK_TIMEOUT: Duration = Duration::from_secs(30);

/// Longest display name in bytes.
pub const MAX_NAME_BYTES: usize = 63;

/// Errors during pairing.
#[derive(Debug, thiserror::Error)]
pub enum PairingError {
    /// The connection failed.
    #[error(transparent)]
    Wire(#[from] WireError),
    /// Establishing the connection failed.
    #[error(transparent)]
    Tls(#[from] TlsError),
    /// Saving the circle failed.
    #[error(transparent)]
    Circle(#[from] CircleError),
    /// The code was wrong, or someone interfered.
    #[error("pairing failed: the code was wrong or the connection was tampered with")]
    Failed,
    /// The owner declined on one of the devices.
    #[error("pairing was declined")]
    Declined,
    /// A step took too long, or the window closed.
    #[error("pairing timed out")]
    Timeout,
    /// The ledger received from the host was invalid.
    #[error("the circle received from the other device is invalid")]
    BadLedger,
    /// The other device sent something that does not belong here.
    #[error("unexpected message during pairing")]
    Protocol,
}

/// The secret that authorizes a pairing.
#[derive(Clone)]
pub enum PairingSecret {
    /// Six digits typed by the owner. Burned on the first attempt.
    Code(Zeroizing<String>),
    /// 16 random bytes carried by a QR code.
    Qr(Zeroizing<[u8; 16]>),
}

impl PairingSecret {
    /// Generates a six-digit code.
    ///
    /// # Errors
    /// Returns an error if no secure randomness is available.
    pub fn new_code() -> Result<Self, CircleError> {
        // Rejection sampling keeps every code equally likely.
        const LIMIT: u32 = u32::MAX - (u32::MAX % 1_000_000);
        loop {
            let value = u32::from_be_bytes(random_bytes()?);
            if value < LIMIT {
                return Ok(Self::Code(Zeroizing::new(format!(
                    "{:06}",
                    value % 1_000_000
                ))));
            }
        }
    }

    /// Generates a QR secret.
    ///
    /// # Errors
    /// Returns an error if no secure randomness is available.
    pub fn new_qr() -> Result<Self, CircleError> {
        Ok(Self::Qr(Zeroizing::new(random_bytes()?)))
    }

    /// Rebuilds a secret read from a QR code.
    #[must_use]
    pub fn from_qr_bytes(bytes: [u8; 16]) -> Self {
        Self::Qr(Zeroizing::new(bytes))
    }

    /// The bytes to place in a QR code, if this is a QR secret.
    #[must_use]
    pub fn qr_bytes(&self) -> Option<[u8; 16]> {
        match self {
            Self::Code(_) => None,
            Self::Qr(b) => Some(**b),
        }
    }

    /// Interprets text typed by the owner as a code.
    #[must_use]
    pub fn from_typed(code: &str) -> Self {
        Self::Code(Zeroizing::new(code.trim().to_string()))
    }

    /// The code to show the owner, if this is a typed code.
    #[must_use]
    pub fn display_code(&self) -> Option<&str> {
        match self {
            Self::Code(c) => Some(c.as_str()),
            Self::Qr(_) => None,
        }
    }

    fn password(&self) -> &[u8] {
        match self {
            Self::Code(c) => c.as_bytes(),
            Self::Qr(b) => b.as_slice(),
        }
    }

    const fn burns_on_first_use(&self) -> bool {
        matches!(self, Self::Code(_))
    }
}

/// What the owner sees when asked to confirm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingPrompt {
    /// The six-digit comparison number.
    pub comparison: String,
    /// The other device's display name.
    pub peer_name: String,
    /// The other device's platform.
    pub peer_platform: Platform,
    /// A short identifier derived from the other device's key.
    pub peer_short_id: String,
}

/// A short, human-readable form of a device ID.
#[must_use]
pub fn short_id(id: &DeviceId) -> String {
    data_encoding::HEXUPPER.encode(id.get(..4).unwrap_or_default())
}

struct Transcript {
    exporter: [u8; 32],
    joiner: DeviceId,
    host: DeviceId,
}

struct Keys {
    host: Zeroizing<[u8; 32]>,
    joiner: Zeroizing<[u8; 32]>,
}

fn derive_keys(k: &[u8], t: &Transcript, a: &[u8; 33], b: &[u8; 33]) -> Result<Keys, PairingError> {
    let info = [CONFIRM_INFO, &t.exporter, &t.joiner, &t.host, a, b].concat();
    let mut okm = Zeroizing::new([0u8; 64]);
    Hkdf::<Sha256>::new(None, k)
        .expand(&info, okm.as_mut_slice())
        .map_err(|_| PairingError::Failed)?;
    let (h, j) = okm.split_at(32);
    Ok(Keys {
        host: Zeroizing::new(h.try_into().map_err(|_| PairingError::Failed)?),
        joiner: Zeroizing::new(j.try_into().map_err(|_| PairingError::Failed)?),
    })
}

fn tag(key: &[u8; 32], label: &[u8]) -> Result<[u8; 32], PairingError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| PairingError::Failed)?;
    mac.update(label);
    Ok(mac.finalize().into_bytes().into())
}

fn check_tag(key: &[u8; 32], label: &[u8], got: &[u8; 32]) -> Result<(), PairingError> {
    let want = tag(key, label)?;
    if bool::from(want.ct_eq(got)) {
        Ok(())
    } else {
        Err(PairingError::Failed)
    }
}

fn comparison_code(
    k: &[u8],
    t: &Transcript,
    nh: &[u8; 32],
    nj: &[u8; 32],
) -> Result<String, PairingError> {
    let info = [SAS_INFO, &t.exporter, &t.joiner, &t.host, nh, nj].concat();
    let mut okm = [0u8; 4];
    Hkdf::<Sha256>::new(None, k)
        .expand(&info, &mut okm)
        .map_err(|_| PairingError::Failed)?;
    Ok(format!("{:06}", u32::from_be_bytes(okm) % 1_000_000))
}

fn commitment(nonce: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(SAS_COMMIT);
    h.update(nonce);
    h.finalize().into()
}

fn fixed_33(v: &[u8]) -> Result<[u8; 33], PairingError> {
    v.try_into().map_err(|_| PairingError::Failed)
}

async fn step<T>(fut: impl Future<Output = Result<T, PairingError>>) -> Result<T, PairingError> {
    timeout(STEP_TIMEOUT, fut)
        .await
        .map_err(|_| PairingError::Timeout)?
}

async fn expect<S: AsyncRead + Unpin>(s: &mut S, kind: FrameType) -> Result<Frame, PairingError> {
    step(async {
        let frame = read_frame(s).await?;
        check_kind(&frame, kind)?;
        Ok(frame)
    })
    .await
}

async fn expect_pre_auth<S: AsyncRead + Unpin>(
    s: &mut S,
    kind: FrameType,
) -> Result<Frame, PairingError> {
    step(async {
        let frame = read_frame_limited(s, MAX_PRE_AUTH_PAYLOAD).await?;
        check_kind(&frame, kind)?;
        Ok(frame)
    })
    .await
}

fn check_kind(frame: &Frame, kind: FrameType) -> Result<(), PairingError> {
    if frame.kind == kind {
        return Ok(());
    }
    if frame.kind == FrameType::Error {
        let msg: ErrorMsg = frame.decode()?;
        return Err(match msg.code {
            code::PAIRING_DECLINED => PairingError::Declined,
            _ => PairingError::Failed,
        });
    }
    Err(PairingError::Protocol)
}

async fn send_error<S: AsyncWrite + Unpin>(s: &mut S, code: u16) {
    let msg = ErrorMsg {
        code,
        message: None,
    };
    let _ = write_msg(s, FrameType::Error, &msg).await;
    let _ = s.flush().await;
}

fn clean_peer_info(info: PeerInfo) -> Result<PeerInfo, PairingError> {
    if info.name.len() > MAX_NAME_BYTES {
        return Err(PairingError::Protocol);
    }
    // Cleaning can lengthen a name, and the ledger refuses one that is too long.
    let name = names::display_name(&info.name);
    if name.len() > MAX_NAME_BYTES {
        return Err(PairingError::Protocol);
    }
    Ok(PeerInfo {
        name,
        platform: info.platform,
    })
}

async fn exchange_info<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    mine: &PeerInfo,
) -> Result<PeerInfo, PairingError> {
    write_msg(s, FrameType::PeerInfo, mine).await?;
    s.flush().await.map_err(WireError::Io)?;
    clean_peer_info(expect(s, FrameType::PeerInfo).await?.decode()?)
}

/// Asks the owner, then exchanges approvals. Both must approve before anything is signed.
async fn approve_both<S, F, Fut>(
    s: &mut S,
    prompt: PairingPrompt,
    confirm: F,
) -> Result<(), PairingError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce(PairingPrompt) -> Fut,
    Fut: Future<Output = bool>,
{
    let accepted = timeout(APPROVAL_TIMEOUT, confirm(prompt))
        .await
        .map_err(|_| PairingError::Timeout)?;
    if !accepted {
        send_error(s, code::PAIRING_DECLINED).await;
        return Err(PairingError::Declined);
    }
    write_msg(s, FrameType::Approve, &()).await?;
    s.flush().await.map_err(WireError::Io)?;
    let frame = timeout(APPROVAL_TIMEOUT, read_frame(s))
        .await
        .map_err(|_| PairingError::Timeout)??;
    check_kind(&frame, FrameType::Approve)
}

struct HostOutcome {
    joiner: DeviceId,
    joiner_name: String,
}

/// How pairing reaches the circle: owned outright, as the command line does,
/// or shared with a device that keeps receiving in the meantime.
pub trait CircleAccess {
    /// Runs `f` with the circle. It must not be held across a wait.
    ///
    /// # Errors
    /// Returns an error if the circle cannot be reached.
    fn with<R>(&mut self, f: impl FnOnce(&mut Circle) -> R) -> Result<R, PairingError>;
}

impl CircleAccess for &mut Circle {
    fn with<R>(&mut self, f: impl FnOnce(&mut Circle) -> R) -> Result<R, PairingError> {
        Ok(f(self))
    }
}

impl CircleAccess for &CircleHandle {
    fn with<R>(&mut self, f: impl FnOnce(&mut Circle) -> R) -> Result<R, PairingError> {
        let mut guard = self.lock().map_err(|_| CircleError::Corrupt)?;
        Ok(f(&mut guard))
    }
}

async fn host_session<S, A, F, Fut>(
    s: &mut S,
    circle: &mut A,
    t: &Transcript,
    secret: &PairingSecret,
    my_info: &PeerInfo,
    burned: &mut bool,
    confirm: &mut F,
) -> Result<HostOutcome, PairingError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    A: CircleAccess,
    F: FnMut(PairingPrompt) -> Fut,
    Fut: Future<Output = bool>,
{
    let a: PakeA = expect_pre_auth(s, FrameType::PakeA).await?.decode()?;
    // Spent as soon as an attempt arrives, before any answer could tell an
    // attacker whether the guess was right.
    if secret.burns_on_first_use() {
        *burned = true;
    }
    let (state, msg_b) = Spake2::<Ed25519Group>::start_b(
        &Password::new(secret.password()),
        &Identity::new(&t.joiner),
        &Identity::new(&t.host),
    );
    let msg_b = fixed_33(&msg_b)?;
    let k = Zeroizing::new(state.finish(&a.msg_a).map_err(|_| PairingError::Failed)?);
    let keys = derive_keys(&k, t, &a.msg_a, &msg_b)?;
    let reply = PakeB {
        msg_b,
        tag: tag(&keys.host, b"host")?,
    };
    write_msg(s, FrameType::PakeB, &reply).await?;
    s.flush().await.map_err(WireError::Io)?;

    let c: Confirm = expect_pre_auth(s, FrameType::Confirm).await?.decode()?;
    if let Err(e) = check_tag(&keys.joiner, b"joiner", &c.tag) {
        send_error(s, code::PAIRING_FAILED).await;
        return Err(e);
    }

    let nh: [u8; 32] = random_bytes()?;
    write_msg(
        s,
        FrameType::SasCommit,
        &SasCommit {
            commit: commitment(&nh),
        },
    )
    .await?;
    s.flush().await.map_err(WireError::Io)?;
    let nj: SasNonce = expect(s, FrameType::SasNonce).await?.decode()?;
    write_msg(s, FrameType::SasReveal, &SasNonce { nonce: nh }).await?;
    s.flush().await.map_err(WireError::Io)?;

    let peer = exchange_info(s, my_info).await?;
    let prompt = PairingPrompt {
        comparison: comparison_code(&k, t, &nh, &nj.nonce)?,
        peer_name: peer.name.clone(),
        peer_platform: peer.platform,
        peer_short_id: short_id(&t.joiner),
    };
    approve_both(s, prompt, confirm).await?;

    circle.with(|c| c.add_pending(t.joiner, &peer.name, peer.platform))??;
    let events: Vec<_> = circle.with(|c| c.ledger().all_events().cloned().collect())?;
    let count = u32::try_from(events.len()).map_err(|_| PairingError::BadLedger)?;
    let sent = async {
        for chunk in events.chunks(EVENTS_PER_FRAME) {
            let msg = LedgerEvents {
                events: chunk.to_vec(),
            };
            write_msg(s, FrameType::LedgerEvents, &msg).await?;
        }
        write_msg(s, FrameType::WelcomeEnd, &WelcomeEnd { event_count: count }).await?;
        s.flush().await.map_err(WireError::Io)?;
        timeout(ACK_TIMEOUT, read_frame(s))
            .await
            .map_err(|_| PairingError::Timeout)?
            .map_err(PairingError::from)
            .and_then(|f| check_kind(&f, FrameType::WelcomeAck))
    }
    .await;

    match sent {
        Ok(()) => {
            circle.with(Circle::confirm_pending)??;
            Ok(HostOutcome {
                joiner: t.joiner,
                joiner_name: peer.name,
            })
        }
        Err(e) => {
            circle.with(|c| c.abandon_pending(t.joiner))??;
            Err(e)
        }
    }
}

/// The result of a successful pairing on the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairedDevice {
    /// The new device's ID.
    pub device: DeviceId,
    /// Its display name.
    pub name: String,
}

/// Runs pairing mode until it succeeds, is burned, or the window closes.
///
/// The listener exists only for the duration of this call and has its own TLS
/// configuration, because the pairing verifier accepts any key.
///
/// # Errors
/// Returns an error if pairing does not succeed.
pub async fn host<A, F, Fut>(
    listener: TcpListener,
    nets: &LocalNetworks,
    mut circle: A,
    secret: &PairingSecret,
    my_info: PeerInfo,
    mut confirm: F,
) -> Result<PairedDevice, PairingError>
where
    A: CircleAccess,
    F: FnMut(PairingPrompt) -> Fut,
    Fut: Future<Output = bool>,
{
    let (key, host_id) = circle.with(|c| (c.key().clone(), c.device_id()))?;
    let cfg = tls::server_config(&key, PeerRule::AnyForPairing, ALPN_PAIRING)?;
    let deadline = Instant::now() + PAIRING_WINDOW;
    loop {
        let (tcp, addr) = timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| PairingError::Timeout)?
            .map_err(WireError::Io)?;
        let Ok(mut stream) = tls::accept(nets, tcp, addr, cfg.clone()).await else {
            continue;
        };
        let (_, conn) = stream.get_ref();
        let Ok(joiner) = peer_id(conn.peer_certificates()) else {
            continue;
        };
        let exporter = conn
            .export_keying_material([0u8; 32], EXPORTER_LABEL, Some(b""))
            .map_err(TlsError::Config)?;
        let t = Transcript {
            exporter,
            joiner,
            host: host_id,
        };
        let mut burned = false;
        let result = host_session(
            &mut stream,
            &mut circle,
            &t,
            secret,
            &my_info,
            &mut burned,
            &mut confirm,
        )
        .await;
        let _ = stream.shutdown().await;
        match result {
            Ok(o) => {
                return Ok(PairedDevice {
                    device: o.joiner,
                    name: o.joiner_name,
                });
            }
            // A spent code ends pairing, and so does a decision by the owner.
            Err(e) if burned || matches!(e, PairingError::Declined) => return Err(e),
            // A stranger that never tried, or a failed QR attempt, leaves the window open.
            Err(_) => {}
        }
    }
}

fn circle_id_from(events: &[crate::ledger::SignedEvent]) -> Option<CircleId> {
    events.iter().find_map(|e| match &e.body.kind {
        EventKind::Genesis { nonce, .. } if e.body.seq == 1 => {
            Some(circle_id_for(&e.body.author, nonce))
        }
        _ => None,
    })
}

async fn joiner_session<S, F, Fut>(
    s: &mut S,
    t: &Transcript,
    secret: &PairingSecret,
    my_info: &PeerInfo,
    confirm: F,
) -> Result<Ledger, PairingError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce(PairingPrompt) -> Fut,
    Fut: Future<Output = bool>,
{
    let (state, msg_a) = Spake2::<Ed25519Group>::start_a(
        &Password::new(secret.password()),
        &Identity::new(&t.joiner),
        &Identity::new(&t.host),
    );
    let msg_a = fixed_33(&msg_a)?;
    write_msg(s, FrameType::PakeA, &PakeA { msg_a }).await?;
    s.flush().await.map_err(WireError::Io)?;

    let b: PakeB = expect_pre_auth(s, FrameType::PakeB).await?.decode()?;
    let k = Zeroizing::new(state.finish(&b.msg_b).map_err(|_| PairingError::Failed)?);
    let keys = derive_keys(&k, t, &msg_a, &b.msg_b)?;
    if let Err(e) = check_tag(&keys.host, b"host", &b.tag) {
        send_error(s, code::PAIRING_FAILED).await;
        return Err(e);
    }
    write_msg(
        s,
        FrameType::Confirm,
        &Confirm {
            tag: tag(&keys.joiner, b"joiner")?,
        },
    )
    .await?;
    s.flush().await.map_err(WireError::Io)?;

    let commit: SasCommit = expect(s, FrameType::SasCommit).await?.decode()?;
    let nj: [u8; 32] = random_bytes()?;
    write_msg(s, FrameType::SasNonce, &SasNonce { nonce: nj }).await?;
    s.flush().await.map_err(WireError::Io)?;
    let reveal: SasNonce = expect(s, FrameType::SasReveal).await?.decode()?;
    if !bool::from(commitment(&reveal.nonce).ct_eq(&commit.commit)) {
        send_error(s, code::PAIRING_FAILED).await;
        return Err(PairingError::Failed);
    }

    let peer = exchange_info(s, my_info).await?;
    let prompt = PairingPrompt {
        comparison: comparison_code(&k, t, &reveal.nonce, &nj)?,
        peer_name: peer.name,
        peer_platform: peer.platform,
        peer_short_id: short_id(&t.host),
    };
    approve_both(s, prompt, confirm).await?;

    let mut events = Vec::new();
    let event_count = loop {
        let frame = timeout(ACK_TIMEOUT, read_frame(s))
            .await
            .map_err(|_| PairingError::Timeout)??;
        match frame.kind {
            FrameType::LedgerEvents => {
                let msg: LedgerEvents = frame.decode()?;
                events.extend(msg.events);
                if events.len() > MAX_HELD {
                    return Err(PairingError::BadLedger);
                }
            }
            FrameType::DiscoveryKey => {}
            FrameType::WelcomeEnd => break frame.decode::<WelcomeEnd>()?.event_count,
            _ => return Err(PairingError::Protocol),
        }
    };
    if usize::try_from(event_count).ok() != Some(events.len()) {
        return Err(PairingError::BadLedger);
    }

    let circle_id = circle_id_from(&events).ok_or(PairingError::BadLedger)?;
    let mut ledger = Ledger::empty(circle_id);
    for event in events {
        ledger.insert(event).map_err(|_| PairingError::BadLedger)?;
    }
    let view = ledger.membership();
    if !view.members.contains(&t.host) || !view.members.contains(&t.joiner) {
        return Err(PairingError::BadLedger);
    }
    Ok(ledger)
}

/// How a joining device checks the host it reaches.
#[derive(Clone, Copy, Debug)]
pub enum HostPin {
    /// No pin: the code alone authenticates the pairing.
    None,
    /// The host must have this key, taken from a QR code.
    Key(DeviceId),
}

/// Joins a circle. On success the device's state is saved in `dir`.
///
/// # Errors
/// Returns an error if pairing does not succeed.
#[allow(clippy::too_many_arguments)]
pub async fn join<F, Fut>(
    addr: SocketAddr,
    nets: &LocalNetworks,
    dir: &Path,
    key: SigningKey,
    secret: &PairingSecret,
    pin: HostPin,
    my_info: PeerInfo,
    confirm: F,
) -> Result<Circle, PairingError>
where
    F: FnOnce(PairingPrompt) -> Fut,
    Fut: Future<Output = bool>,
{
    join_sealed(
        addr,
        nets,
        dir,
        key,
        secret,
        pin,
        my_info,
        &KeySealing::default(),
        confirm,
    )
    .await
}

/// Like [`join`], sealing the device key on disk.
///
/// # Errors
/// Returns an error if pairing does not succeed.
#[allow(clippy::too_many_arguments)]
pub async fn join_sealed<F, Fut>(
    addr: SocketAddr,
    nets: &LocalNetworks,
    dir: &Path,
    key: SigningKey,
    secret: &PairingSecret,
    pin: HostPin,
    my_info: PeerInfo,
    sealing: &KeySealing,
    confirm: F,
) -> Result<Circle, PairingError>
where
    F: FnOnce(PairingPrompt) -> Fut,
    Fut: Future<Output = bool>,
{
    let rule = match pin {
        HostPin::None => PeerRule::AnyForPairing,
        HostPin::Key(k) => PeerRule::Exact(k),
    };
    let cfg = tls::client_config(&key, rule, ALPN_PAIRING)?;
    let mut stream = tls::connect(nets, addr, cfg).await?;
    let (_, conn) = stream.get_ref();
    let host_id = peer_id(conn.peer_certificates())?;
    let exporter = conn
        .export_keying_material([0u8; 32], EXPORTER_LABEL, Some(b""))
        .map_err(TlsError::Config)?;
    let joiner_id = key.verifying_key().to_bytes();
    let t = Transcript {
        exporter,
        joiner: joiner_id,
        host: host_id,
    };
    let ledger = joiner_session(&mut stream, &t, secret, &my_info, confirm).await?;
    let circle = Circle::adopt_sealed(dir, key, ledger, 1, sealing)?;
    write_msg(&mut stream, FrameType::WelcomeAck, &()).await?;
    stream.flush().await.map_err(WireError::Io)?;
    let _ = stream.shutdown().await;
    Ok(circle)
}

/// A QR payload as defined in protocol section 8.2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrPayload {
    /// The host's device ID.
    pub host: DeviceId,
    /// The pairing secret.
    pub secret: [u8; 16],
    /// Addresses at which the host is listening.
    pub addresses: Vec<SocketAddr>,
}

impl QrPayload {
    /// Formats the payload as a URI.
    #[must_use]
    pub fn encode(&self) -> String {
        let b64 = data_encoding::BASE64URL_NOPAD;
        let mut out = format!(
            "zerohop://pair/1?k={}&s={}",
            b64.encode(&self.host),
            b64.encode(&self.secret)
        );
        for a in &self.addresses {
            out.push_str("&a=");
            out.push_str(&a.to_string());
        }
        out
    }

    /// Parses a URI, refusing anything malformed.
    #[must_use]
    pub fn decode(uri: &str) -> Option<Self> {
        let query = uri.strip_prefix("zerohop://pair/1?")?;
        let b64 = data_encoding::BASE64URL_NOPAD;
        let (mut host, mut secret, mut addresses) = (None, None, Vec::new());
        for part in query.split('&') {
            let (key, value) = part.split_once('=')?;
            match key {
                "k" => host = Some(<[u8; 32]>::try_from(b64.decode(value.as_bytes()).ok()?).ok()?),
                "s" => {
                    secret = Some(<[u8; 16]>::try_from(b64.decode(value.as_bytes()).ok()?).ok()?)
                }
                "a" => addresses.push(value.parse().ok()?),
                _ => return None,
            }
        }
        Some(Self {
            host: host?,
            secret: secret?,
            addresses,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn transcript() -> Transcript {
        Transcript {
            exporter: [1; 32],
            joiner: [2; 32],
            host: [3; 32],
        }
    }

    #[test]
    fn a_code_is_six_digits() {
        for _ in 0..200 {
            let secret = PairingSecret::new_code().unwrap();
            let code = secret.display_code().unwrap();
            assert_eq!(code.len(), 6);
            assert!(code.bytes().all(|b| b.is_ascii_digit()));
        }
    }

    #[test]
    fn different_exporters_give_different_confirmation_keys() {
        let t1 = transcript();
        let mut t2 = transcript();
        t2.exporter = [9; 32];
        let k1 = derive_keys(b"secret", &t1, &[4; 33], &[5; 33]).unwrap();
        let k2 = derive_keys(b"secret", &t2, &[4; 33], &[5; 33]).unwrap();
        assert_ne!(*k1.host, *k2.host);
    }

    #[test]
    fn host_and_joiner_keys_differ() {
        let k = derive_keys(b"secret", &transcript(), &[4; 33], &[5; 33]).unwrap();
        assert_ne!(*k.host, *k.joiner);
    }

    #[test]
    fn a_wrong_tag_is_rejected() {
        let key = [7; 32];
        assert!(check_tag(&key, b"host", &[0; 32]).is_err());
        let good = tag(&key, b"host").unwrap();
        assert!(check_tag(&key, b"host", &good).is_ok());
        assert!(check_tag(&key, b"joiner", &good).is_err());
    }

    #[test]
    fn comparison_code_depends_on_both_nonces() {
        let t = transcript();
        let a = comparison_code(b"k", &t, &[1; 32], &[2; 32]).unwrap();
        let b = comparison_code(b"k", &t, &[1; 32], &[3; 32]).unwrap();
        let c = comparison_code(b"k", &t, &[9; 32], &[2; 32]).unwrap();
        assert_eq!(a.len(), 6);
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn qr_payload_round_trips() {
        let p = QrPayload {
            host: [4; 32],
            secret: [5; 16],
            addresses: vec!["192.168.1.2:4000".parse().unwrap()],
        };
        assert_eq!(QrPayload::decode(&p.encode()), Some(p));
    }

    #[test]
    fn qr_decoder_refuses_malformed_input() {
        assert_eq!(QrPayload::decode("https://example.com"), None);
        assert_eq!(QrPayload::decode("zerohop://pair/1?k=abc&s=def"), None);
        assert_eq!(QrPayload::decode("zerohop://pair/1?zzz=1"), None);
    }
}
