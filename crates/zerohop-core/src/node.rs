//! A running device: accepts connections and sends transfers.

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore, watch};

use crate::address::LocalNetworks;
use crate::circle::Circle;
use crate::clip::{self, OutgoingClip};
use crate::ledger::DeviceId;
use crate::session::{CircleHandle, LocalInfo, Session, SessionError, deliver_removal, lock};
use crate::tls::{self, ALPN_NORMAL, MAX_UNAUTHENTICATED, PeerRule, TlsError, peer_id};
use crate::transfer::{
    self, Control, OfferOutcome, Outgoing, ReceiveConfig, SendReport, TransferError,
};

/// Errors from running a device.
#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    /// Establishing a connection failed.
    #[error(transparent)]
    Tls(#[from] TlsError),
    /// The session failed.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// The transfer failed.
    #[error(transparent)]
    Transfer(#[from] TransferError),
}

/// One device taking part in a circle.
#[derive(Clone)]
pub struct Node {
    circle: CircleHandle,
    nets: Arc<RwLock<LocalNetworks>>,
    info: LocalInfo,
    state_dir: PathBuf,
    members_tx: Arc<watch::Sender<BTreeSet<DeviceId>>>,
    active: Arc<Mutex<HashMap<DeviceId, Arc<Notify>>>>,
}

impl Node {
    /// Wraps a loaded circle.
    #[must_use]
    pub fn new(circle: Circle, nets: LocalNetworks, info: LocalInfo, state_dir: &Path) -> Self {
        let members = circle.ledger().membership().members;
        let (tx, _) = watch::channel(members);
        Self {
            circle: Arc::new(Mutex::new(circle)),
            nets: Arc::new(RwLock::new(nets)),
            info,
            state_dir: state_dir.to_path_buf(),
            members_tx: Arc::new(tx),
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Access to the shared circle.
    #[must_use]
    pub const fn circle(&self) -> &CircleHandle {
        &self.circle
    }

    /// Replaces the set of local networks, for when the device moves between them.
    pub fn set_networks(&self, nets: LocalNetworks) {
        *self.nets.write().unwrap_or_else(PoisonError::into_inner) = nets;
    }

    fn networks(&self) -> LocalNetworks {
        self.nets
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Tells live sessions that membership may have changed.
    ///
    /// # Errors
    /// Returns an error if the circle cannot be read.
    pub fn publish_membership(&self) -> Result<(), SessionError> {
        let members = lock(&self.circle)?.ledger().membership().members;
        self.members_tx.send_if_modified(|current| {
            let changed = *current != members;
            if changed {
                current.clone_from(&members);
            }
            changed
        });
        Ok(())
    }

    fn accepts(circle: CircleHandle) -> PeerRule {
        PeerRule::Matching(Arc::new(move |id: &DeviceId| {
            circle
                .lock()
                .is_ok_and(|c| c.ledger().membership().ever.contains(id))
        }))
    }

    async fn handle_connection(
        &self,
        stream: tls::ServerStream,
        peer: DeviceId,
        cfg: &ReceiveConfig,
    ) -> Result<Vec<OfferOutcome>, NodeError> {
        let is_member = lock(&self.circle)?.is_member(&peer);
        if !is_member {
            // Authenticated, but no longer welcome: it gets the proof and
            // nothing it sends is ever read.
            deliver_removal(stream, &self.circle, &self.info).await?;
            return Ok(Vec::new());
        }
        let established = Session::establish(
            stream,
            peer,
            &self.circle,
            &self.info,
            self.members_tx.subscribe(),
        )
        .await;
        // Even a handshake that failed may have applied a removal.
        self.publish_membership()?;
        let mut session = established?;
        Ok(transfer::serve_session(&mut session, cfg).await?)
    }

    /// Accepts connections until the task is cancelled.
    ///
    /// # Errors
    /// Returns an error if the listener fails.
    pub async fn serve(&self, listener: TcpListener, cfg: ReceiveConfig) -> Result<(), NodeError> {
        let key = lock(&self.circle)?.key().clone();
        let tls_cfg = tls::server_config(&key, Self::accepts(self.circle.clone()), ALPN_NORMAL)?;
        let limit = Arc::new(Semaphore::new(MAX_UNAUTHENTICATED));
        loop {
            let (tcp, addr) = listener.accept().await.map_err(TlsError::Io)?;
            let Ok(permit) = limit.clone().try_acquire_owned() else {
                continue;
            };
            let node = self.clone();
            let tls_cfg = tls_cfg.clone();
            let cfg = cfg.clone();
            tokio::spawn(async move {
                let stream = tls::accept(&node.networks(), tcp, addr, tls_cfg).await;
                drop(permit);
                let Ok(stream) = stream else {
                    return;
                };
                let Ok(peer) = peer_id(stream.get_ref().1.peer_certificates()) else {
                    return;
                };
                let stop = node.claim_peer(peer);
                tokio::select! {
                    _ = node.handle_connection(stream, peer, &cfg) => {}
                    () = stop.notified() => {}
                }
                node.release_peer(&peer, &stop);
            });
        }
    }

    /// One session per peer: a new connection replaces the old one, which also
    /// clears away half-open connections.
    fn claim_peer(&self, peer: DeviceId) -> Arc<Notify> {
        let stop = Arc::new(Notify::new());
        if let Ok(mut active) = self.active.lock()
            && let Some(old) = active.insert(peer, stop.clone())
        {
            old.notify_one();
        }
        stop
    }

    fn release_peer(&self, peer: &DeviceId, stop: &Arc<Notify>) {
        if let Ok(mut active) = self.active.lock()
            && active.get(peer).is_some_and(|c| Arc::ptr_eq(c, stop))
        {
            active.remove(peer);
        }
    }

    /// Accepts one connection and returns its results. For tests and tools.
    ///
    /// # Errors
    /// Returns an error if the connection or transfer fails.
    pub async fn serve_one(
        &self,
        listener: &TcpListener,
        cfg: &ReceiveConfig,
    ) -> Result<Vec<OfferOutcome>, NodeError> {
        let key = lock(&self.circle)?.key().clone();
        let tls_cfg = tls::server_config(&key, Self::accepts(self.circle.clone()), ALPN_NORMAL)?;
        let (tcp, addr) = listener.accept().await.map_err(TlsError::Io)?;
        let stream = tls::accept(&self.networks(), tcp, addr, tls_cfg).await?;
        let peer = peer_id(stream.get_ref().1.peer_certificates())?;
        self.handle_connection(stream, peer, cfg).await
    }

    async fn dial(
        &self,
        peer: DeviceId,
        addr: SocketAddr,
    ) -> Result<Session<tls::ClientStream>, NodeError> {
        let key = lock(&self.circle)?.key().clone();
        let cfg = tls::client_config(&key, PeerRule::Exact(peer), ALPN_NORMAL)?;
        let stream = tls::connect(&self.networks(), addr, cfg).await?;
        let established = Session::establish(
            stream,
            peer,
            &self.circle,
            &self.info,
            self.members_tx.subscribe(),
        )
        .await;
        self.publish_membership()?;
        Ok(established?)
    }

    /// Connects to a member and sends a prepared transfer.
    ///
    /// # Errors
    /// Returns an error if the connection or transfer fails.
    pub async fn send(
        &self,
        peer: DeviceId,
        addr: SocketAddr,
        outgoing: Outgoing,
    ) -> Result<SendReport, NodeError> {
        self.send_with(peer, addr, outgoing, &Control::new()).await
    }

    /// Like [`Self::send`], reporting progress to and stopping on `control`.
    ///
    /// # Errors
    /// Returns an error if the connection or transfer fails.
    pub async fn send_with(
        &self,
        peer: DeviceId,
        addr: SocketAddr,
        outgoing: Outgoing,
        control: &Control,
    ) -> Result<SendReport, NodeError> {
        let mut session = self.dial(peer, addr).await?;
        let report = transfer::send_with(&mut session, outgoing, &self.state_dir, control).await?;
        session.goodbye().await;
        Ok(report)
    }

    /// Connects to a member and continues an interrupted transfer.
    ///
    /// # Errors
    /// Returns an error if the connection or transfer fails.
    pub async fn resume(
        &self,
        peer: DeviceId,
        addr: SocketAddr,
        transfer_id: [u8; 16],
    ) -> Result<SendReport, NodeError> {
        self.resume_with(peer, addr, transfer_id, &Control::new())
            .await
    }

    /// Like [`Self::resume`], reporting progress to and stopping on `control`.
    ///
    /// # Errors
    /// Returns an error if the connection or transfer fails.
    pub async fn resume_with(
        &self,
        peer: DeviceId,
        addr: SocketAddr,
        transfer_id: [u8; 16],
        control: &Control,
    ) -> Result<SendReport, NodeError> {
        let mut session = self.dial(peer, addr).await?;
        let report =
            transfer::resume_with(&mut session, transfer_id, &self.state_dir, control).await?;
        session.goodbye().await;
        Ok(report)
    }

    /// Connects to a member and offers it a copied clip.
    ///
    /// # Errors
    /// Returns an error if the connection fails or the member breaks the protocol.
    pub async fn send_clip(
        &self,
        peer: DeviceId,
        addr: SocketAddr,
        outgoing: &OutgoingClip,
    ) -> Result<(), NodeError> {
        let mut session = self.dial(peer, addr).await?;
        clip::offer(&mut session, outgoing).await?;
        session.goodbye().await;
        Ok(())
    }

    /// Connects to a member only to exchange ledger updates.
    ///
    /// # Errors
    /// Returns an error if the connection fails.
    pub async fn sync_with(&self, peer: DeviceId, addr: SocketAddr) -> Result<(), NodeError> {
        let mut session = self.dial(peer, addr).await?;
        session.goodbye().await;
        Ok(())
    }
}
