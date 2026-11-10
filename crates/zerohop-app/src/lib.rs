//! The application layer shared by every Zerohop interface.
//!
//! An interface sends requests and receives messages, both as JSON text, so
//! that all behaviour lives here and can be tested without any interface.
//! Zerohop has not been independently audited.

mod clipboard;
mod store;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::oneshot;
use tokio::task::AbortHandle;
use zerohop_core::address::LocalNetworks;
use zerohop_core::circle::{Circle, CircleError, generate_key};
pub use zerohop_core::circle::{KeySealing, random_bytes};
use zerohop_core::discovery;
use zerohop_core::ledger::{DeviceId, Platform};
use zerohop_core::names::display_name;
use zerohop_core::node::{Node, NodeError};
use zerohop_core::pairing::{self, HostPin, PairingError, PairingPrompt, PairingSecret, short_id};
use zerohop_core::receive::{self, HeldItem, ReceiveFolder, StoreError};
use zerohop_core::session::{LocalInfo, SessionError};
use zerohop_core::transfer::{
    self, Approver, Budgets, Control, DEFAULT_AUTO_LIMIT, OfferOutcome, OfferSummary,
    ReceiveConfig, ReceiveMode, ReceiveSettings, ReceiveStart, SendReport, TransferError,
    Unfinished,
};
use zerohop_core::wire::{PeerInfo, code};
use zeroize::Zeroizing;

use store::{HistoryEntry, MAX_HISTORY, Settings};

const PREFERRED_PORT: u16 = 53_318;
const PREFERRED_PAIRING_PORT: u16 = 53_319;
const SETTINGS_FILE: &str = "settings.json";
const HISTORY_FILE: &str = "history.json";
const TICK: Duration = Duration::from_millis(250);
const TICKS_WHEN_IDLE: u64 = 4;
const TICKS_PER_PRESENCE: u64 = 20;
const TICKS_PER_PROBE: u64 = 40;
const TICKS_PER_SWEEP: u64 = 14_400;
const SYNC_AFTER_SUCCESS: Duration = Duration::from_secs(120);
const SYNC_AFTER_FAILURE: Duration = Duration::from_secs(20);
const NEARBY_AFTER_CONTACT: Duration = Duration::from_secs(150);
const MAX_UNTAGGED_ATTEMPTS: usize = 16;
const ASK_TIMEOUT: Duration = Duration::from_secs(300);
const CANCEL_GRACE: Duration = Duration::from_secs(4);
const KEEP_UNFINISHED: Duration = Duration::from_secs(24 * 60 * 60);

/// Where this device keeps things.
#[derive(Clone, Debug)]
pub struct Config {
    /// Private state: keys, circle, settings, history.
    pub state_dir: PathBuf,
    /// Where files go until the owner picks another folder.
    pub default_folder: PathBuf,
    /// What protects the device key on disk.
    pub sealing: KeySealing,
}

/// Errors starting the engine.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// The background runtime could not start.
    #[error("could not start: {0}")]
    Runtime(#[from] std::io::Error),
}

/// Receives each message, as JSON text.
pub type Sink = Arc<dyn Fn(String) + Send + Sync>;

/// The running application.
pub struct Engine {
    shared: Arc<Shared>,
    runtime: Runtime,
}

impl Engine {
    /// Starts the engine. Nothing happens on the network until `bootstrap`.
    ///
    /// # Errors
    /// Returns an error if the background runtime cannot start.
    pub fn start(config: Config, sink: Sink) -> Result<Self, StartError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(8)
            .enable_all()
            .build()?;
        let settings: Settings = store::load(&config.state_dir.join(SETTINGS_FILE));
        let history: Vec<HistoryEntry> = store::load(&config.state_dir.join(HISTORY_FILE));
        let shared = Arc::new(Shared {
            config,
            sink,
            handle: runtime.handle().clone(),
            inner: Mutex::new(Inner {
                settings,
                history,
                ..Inner::default()
            }),
        });
        Ok(Self { shared, runtime })
    }

    /// Handles a request. The answer arrives through the sink.
    pub fn request(&self, text: &str) {
        let shared = self.shared.clone();
        // A request may carry copied content.
        let text = Zeroizing::new(text.to_string());
        self.runtime
            .spawn(async move { shared.dispatch(&text).await });
    }
}

/// A failure the interface can explain to the owner.
#[derive(Debug)]
struct AppError {
    kind: &'static str,
    detail: String,
}

impl AppError {
    fn new(kind: &'static str) -> Self {
        Self {
            kind,
            detail: String::new(),
        }
    }

    fn with(kind: &'static str, detail: impl std::fmt::Display) -> Self {
        Self {
            kind,
            detail: detail.to_string(),
        }
    }
}

impl From<CircleError> for AppError {
    fn from(e: CircleError) -> Self {
        match e {
            CircleError::InUse => Self::new("in_use"),
            CircleError::KeyLocked => Self::new("key_locked"),
            other => Self::with("state", other),
        }
    }
}

impl From<StoreError> for AppError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Unknown => Self::new("not_found"),
            StoreError::NoSpace => Self::new("no_space"),
            other => Self::with("storage", other),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::with("storage", e)
    }
}

type Reply = Result<Value, AppError>;

#[derive(Deserialize)]
struct Envelope {
    id: u64,
    #[serde(flatten)]
    request: Request,
}

#[derive(Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
enum Request {
    LoadSettings,
    Bootstrap {
        #[serde(default)]
        allow_loopback: bool,
    },
    SetName {
        name: String,
    },
    SetSettings {
        default_mode: Option<String>,
        folder: Option<String>,
        visible: Option<bool>,
        confirm_new: Option<bool>,
        ui: Option<Value>,
    },
    SetDeviceMode {
        device: String,
        mode: Option<String>,
    },
    ConfirmDevice {
        device: String,
        accept: bool,
    },
    RemoveDevice {
        device: String,
    },
    StartOver,
    PairHost,
    PairJoin {
        address: String,
        code: String,
    },
    PairAnswer {
        accept: bool,
    },
    PairCancel,
    DiscoverySeen {
        instance: String,
        addresses: Vec<String>,
        port: u16,
        tag: Option<String>,
    },
    DiscoveryLost {
        instance: String,
    },
    Send {
        device: String,
        paths: Vec<String>,
    },
    Cancel {
        transfer: String,
    },
    Resume {
        transfer: String,
    },
    ForgetUnfinished {
        transfer: String,
    },
    AnswerOffer {
        offer: u64,
        accept: bool,
    },
    ReleaseHeld {
        transfer: String,
        name: String,
    },
    DiscardHeld {
        transfer: String,
        name: String,
    },
    ClearHistory,
    SetClipboard {
        enabled: bool,
    },
    SetClipboardDevice {
        device: String,
        send: Option<bool>,
        receive: Option<bool>,
        text: Option<bool>,
        images: Option<bool>,
    },
    ClipChanged {
        format: String,
        text: Option<String>,
        data: Option<String>,
    },
}

struct Device {
    node: Node,
    dir: PathBuf,
    receive: ReceiveSettings,
    port: u16,
    serve: AbortHandle,
}

impl Drop for Device {
    fn drop(&mut self) {
        self.serve.abort();
    }
}

struct Seen {
    addresses: Vec<SocketAddr>,
    tag: Option<String>,
}

struct Active {
    direction: &'static str,
    peer: DeviceId,
    peer_name: String,
    files: u32,
    total: u64,
    name: String,
    control: Control,
    task: Option<AbortHandle>,
    last_done: u64,
    last_at: Instant,
    speed: f64,
}

struct PendingOffer {
    transfer: [u8; 16],
    peer: DeviceId,
    peer_name: String,
    files: u32,
    total: u64,
    has_program: bool,
    first_program: Option<String>,
    names: Vec<String>,
    answer: oneshot::Sender<bool>,
}

#[derive(Default)]
struct PairingState {
    role: &'static str,
    phase: &'static str,
    code: Option<String>,
    room: Option<String>,
    port: Option<u16>,
    comparison: Option<String>,
    peer_name: Option<String>,
    peer_platform: Option<&'static str>,
    peer_short_id: Option<String>,
    deadline_ms: i64,
    answer: Option<oneshot::Sender<bool>>,
    task: Option<AbortHandle>,
}

#[derive(Clone)]
struct Advert {
    instance: String,
    tag: String,
    port: u16,
}

#[derive(Default)]
struct Inner {
    settings: Settings,
    history: Vec<HistoryEntry>,
    device: Option<Device>,
    nets: LocalNetworks,
    allow_loopback: bool,
    bootstrapped: bool,
    problem: Option<&'static str>,
    instances: HashMap<String, Seen>,
    last_addresses: HashMap<DeviceId, Vec<SocketAddr>>,
    last_contact: HashMap<DeviceId, Instant>,
    next_sync: HashMap<DeviceId, Instant>,
    peer_locks: HashMap<DeviceId, Arc<tokio::sync::Mutex<()>>>,
    transfers: HashMap<[u8; 16], Active>,
    offers: BTreeMap<u64, PendingOffer>,
    next_offer: u64,
    asked: HashMap<[u8; 16], (String, u32, u64, String)>,
    pairing: Option<PairingState>,
    unfinished: Vec<Unfinished>,
    held: Vec<HeldItem>,
    advert: Option<Advert>,
    advert_epoch: u64,
    presence: BTreeSet<DeviceId>,
    last_members: BTreeSet<DeviceId>,
    clip: clipboard::ClipState,
}

struct Shared {
    config: Config,
    sink: Sink,
    handle: Handle,
    inner: Mutex<Inner>,
}

fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

fn unhex<const N: usize>(text: &str) -> Result<[u8; N], AppError> {
    data_encoding::HEXLOWER
        .decode(text.as_bytes())
        .ok()
        .and_then(|bytes| <[u8; N]>::try_from(bytes).ok())
        .ok_or_else(|| AppError::new("bad_request"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

const fn this_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::MacOs
    } else if cfg!(target_os = "ios") {
        Platform::Ios
    } else if cfg!(target_os = "android") {
        Platform::Android
    } else if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        Platform::Other
    }
}

const fn platform_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Windows => "windows",
        Platform::MacOs => "mac",
        Platform::Ios => "ios",
        Platform::Android => "android",
        Platform::Linux => "linux",
        Platform::Other => "other",
    }
}

fn parse_mode(text: &str) -> Result<ReceiveMode, AppError> {
    match text {
        "ask" => Ok(ReceiveMode::AlwaysAsk),
        "auto" => Ok(ReceiveMode::Automatic),
        "smart" => Ok(ReceiveMode::AutomaticExceptPrograms),
        "refuse" => Ok(ReceiveMode::Refuse),
        _ => Err(AppError::new("bad_request")),
    }
}

fn explain_pairing(error: &PairingError) -> &'static str {
    match error {
        PairingError::Declined => "declined",
        PairingError::Timeout => "timeout",
        PairingError::Failed => "wrong_code",
        PairingError::Tls(_) => "unreachable",
        _ => "failed",
    }
}

/// How a transfer ended, and whether it can be picked up again.
fn explain_transfer(error: &NodeError, cancelled_here: bool) -> (&'static str, bool) {
    match error {
        NodeError::Tls(_) => ("unreachable", false),
        NodeError::Session(e) => explain_session(e),
        NodeError::Transfer(e) => match e {
            TransferError::Session(e) => explain_session(e),
            TransferError::Declined(code::NO_SPACE) => ("no_space", false),
            TransferError::Declined(code::RATE_LIMITED) => ("busy", false),
            TransferError::Declined(code::INVALID_PATH) => ("invalid_name", false),
            TransferError::Declined(code::CANCELLED) => ("peer_storage", false),
            TransferError::Declined(_) => ("declined", false),
            TransferError::Cancelled(_) if cancelled_here => ("cancelled", false),
            TransferError::Cancelled(code::NO_SPACE) => ("no_space", false),
            TransferError::Cancelled(code::INTEGRITY_FAILED) => ("integrity", false),
            TransferError::Cancelled(_) => ("cancelled_by_peer", false),
            TransferError::SourceChanged => ("source_changed", false),
            TransferError::Source(_) => ("source_unreadable", true),
            TransferError::BadOffer => ("nothing_to_send", false),
            TransferError::Store(StoreError::NoSpace) => ("no_space", false),
            TransferError::Store(StoreError::Integrity) => ("integrity", false),
            TransferError::Store(_) => ("storage", false),
        },
    }
}

/// Writes a line for whoever is diagnosing a problem, only if they asked.
fn debug_log(message: impl std::fmt::Display) {
    if std::env::var_os("ZEROHOP_DEBUG").is_some() {
        eprintln!("zerohop: {message}");
    }
}

const fn explain_session(error: &SessionError) -> (&'static str, bool) {
    match error {
        SessionError::Removed => ("removed", false),
        SessionError::NotMember => ("peer_removed", false),
        SessionError::Remote(code::UNKNOWN_TRANSFER) => ("cannot_resume", false),
        SessionError::Remote(code::DECLINED) => ("declined", false),
        SessionError::Remote(code::CANCELLED) => ("peer_storage", false),
        SessionError::Protocol | SessionError::Remote(_) => ("protocol", false),
        _ => ("connection_lost", true),
    }
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, value: &Value) {
        (self.sink)(value.to_string());
    }

    async fn dispatch(self: &Arc<Self>, text: &str) {
        let Ok(Envelope { id, request }) = serde_json::from_str(text) else {
            return;
        };
        let message = match self.handle_request(request).await {
            Ok(value) => json!({ "reply": id, "ok": value }),
            Err(e) => json!({ "reply": id, "error": { "kind": e.kind, "detail": e.detail } }),
        };
        self.emit(&message);
    }

    async fn handle_request(self: &Arc<Self>, request: Request) -> Reply {
        match request {
            Request::LoadSettings => {
                let settings = self.state().settings.clone();
                Ok(serde_json::to_value(settings).unwrap_or(Value::Null))
            }
            Request::Bootstrap { allow_loopback } => self.bootstrap(allow_loopback).await,
            Request::SetName { name } => self.set_name(name),
            Request::SetSettings {
                default_mode,
                folder,
                visible,
                confirm_new,
                ui,
            } => self.set_settings(default_mode, folder, visible, confirm_new, ui),
            Request::SetDeviceMode { device, mode } => self.set_device_mode(&device, mode),
            Request::ConfirmDevice { device, accept } => self.confirm_device(&device, accept),
            Request::RemoveDevice { device } => self.remove_device(&device),
            Request::StartOver => self.start_over(),
            Request::PairHost => self.pair_host().await,
            Request::PairJoin { address, code } => self.pair_join(&address, &code),
            Request::PairAnswer { accept } => self.pair_answer(accept),
            Request::PairCancel => self.pair_cancel(),
            Request::DiscoverySeen {
                instance,
                addresses,
                port,
                tag,
            } => self.discovery_seen(instance, &addresses, port, tag),
            Request::DiscoveryLost { instance } => {
                self.state().instances.remove(&instance);
                self.refresh_presence();
                Ok(Value::Null)
            }
            Request::Send { device, paths } => self.send(&device, paths).await,
            Request::Cancel { transfer } => self.cancel(&transfer),
            Request::Resume { transfer } => self.resume(&transfer),
            Request::ForgetUnfinished { transfer } => self.forget_unfinished(&transfer).await,
            Request::AnswerOffer { offer, accept } => self.answer_offer(offer, accept),
            Request::ReleaseHeld { transfer, name } => {
                self.settle_held(&transfer, name, true).await
            }
            Request::DiscardHeld { transfer, name } => {
                self.settle_held(&transfer, name, false).await
            }
            Request::ClearHistory => {
                self.state().history.clear();
                self.save_history();
                self.publish();
                Ok(Value::Null)
            }
            Request::SetClipboard { enabled } => self.set_clipboard(enabled),
            Request::SetClipboardDevice {
                device,
                send,
                receive,
                text,
                images,
            } => self.set_clipboard_device(
                &device,
                &clipboard::DeviceChange {
                    send,
                    receive,
                    text,
                    images,
                },
            ),
            Request::ClipChanged { format, text, data } => {
                self.clip_changed(&format, text, data).await
            }
        }
    }

    fn save_settings(&self) {
        let settings = self.state().settings.clone();
        let _ = store::save(&self.config.state_dir.join(SETTINGS_FILE), &settings);
    }

    fn save_history(&self) {
        let history = self.state().history.clone();
        let _ = store::save(&self.config.state_dir.join(HISTORY_FILE), &history);
    }

    fn circle_dir(&self) -> PathBuf {
        self.config
            .state_dir
            .join(&self.state().settings.circle_dir)
    }

    fn detect_networks(allow_loopback: bool) -> LocalNetworks {
        let nets = LocalNetworks::detect().unwrap_or_default();
        #[cfg(feature = "dev-loopback")]
        let nets = if allow_loopback {
            nets.with_loopback()
        } else {
            nets
        };
        #[cfg(not(feature = "dev-loopback"))]
        let _ = allow_loopback;
        nets
    }

    async fn bootstrap(self: &Arc<Self>, allow_loopback: bool) -> Reply {
        let first = {
            let mut inner = self.state();
            let first = !inner.bootstrapped;
            inner.bootstrapped = true;
            inner.allow_loopback = allow_loopback;
            inner.nets = Self::detect_networks(allow_loopback);
            first
        };
        std::fs::create_dir_all(&self.config.state_dir)?;
        let dir = self.circle_dir();
        let needs_device = self.state().device.is_none();
        // Before anything that can fail: a device whose key would not open
        // can still start over within this same run.
        if first {
            let this = self.clone();
            self.handle.spawn(async move { this.ticker().await });
        }
        if needs_device && Circle::exists(&dir) {
            let circle = Circle::load_sealed(&dir, &self.config.sealing)?;
            self.start_device(circle, dir.clone()).await?;
        }
        self.remove_abandoned_circles(&dir);
        self.sweep().await;
        self.reload_lists().await;
        self.publish();
        Ok(Value::Null)
    }

    /// Clears out the state of circles this device no longer belongs to.
    fn remove_abandoned_circles(&self, current: &Path) {
        let Ok(entries) = std::fs::read_dir(&self.config.state_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_circle = entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("device"));
            if is_circle && path.is_dir() && path != current {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }

    fn open_folder(&self) -> (ReceiveFolder, Option<&'static str>) {
        let chosen = self.state().settings.folder.clone();
        if let Some(path) = chosen
            && let Ok(folder) = ReceiveFolder::open(Path::new(&path))
        {
            return (folder, None);
        }
        let problem = self
            .state()
            .settings
            .folder
            .is_some()
            .then_some("folder_unavailable");
        match ReceiveFolder::open(&self.config.default_folder) {
            Ok(folder) => (folder, problem),
            Err(_) => (
                // Nothing better exists; offers will fail and say so.
                ReceiveFolder::open(&self.config.state_dir)
                    .unwrap_or_else(|_| fallback_folder(&self.config.state_dir)),
                Some("folder_unavailable"),
            ),
        }
    }

    async fn start_device(self: &Arc<Self>, circle: Circle, dir: PathBuf) -> Result<(), AppError> {
        let me = circle.device_id();
        let info = LocalInfo {
            name: circle.ledger().name_of(&me).unwrap_or("device").to_string(),
            platform: this_platform(),
        };
        let nets = self.state().nets.clone();
        let node = Node::new(circle, nets, info, &dir);
        let (folder, problem) = self.open_folder();
        let receive = {
            let mut inner = self.state();
            inner.problem = problem;
            let default = parse_mode(&inner.settings.default_mode)
                .unwrap_or(ReceiveMode::AutomaticExceptPrograms);
            let receive = ReceiveSettings::new(folder, default);
            for (id, mode) in &inner.settings.device_modes {
                if let (Ok(id), Ok(mode)) = (unhex::<32>(id), parse_mode(mode)) {
                    receive.set_device_mode(id, Some(mode));
                }
            }
            for id in &inner.settings.unconfirmed {
                if let Ok(id) = unhex::<32>(id) {
                    receive.set_device_mode(id, Some(ReceiveMode::Refuse));
                }
            }
            receive
        };
        let cfg = ReceiveConfig {
            settings: receive.clone(),
            approver: Arc::new(AppApprover(self.clone())),
            auto_limit: DEFAULT_AUTO_LIMIT,
            budgets: Arc::new(Budgets::default()),
            clip: Some(Arc::new(clipboard::AppClip(self.clone()))),
        };
        let listener = bind_preferring(PREFERRED_PORT).await?;
        let port = listener.local_addr()?.port();
        let serving = node.clone();
        let serve = self
            .handle
            .spawn(async move {
                let _ = serving.serve(listener, cfg).await;
            })
            .abort_handle();
        self.state().device = Some(Device {
            node,
            dir,
            receive,
            port,
            serve,
        });
        self.renew_advert();
        self.note_new_members();
        Ok(())
    }

    fn node(&self) -> Result<Node, AppError> {
        self.state()
            .device
            .as_ref()
            .map(|d| d.node.clone())
            .ok_or_else(|| AppError::new("no_circle"))
    }

    fn device_state_dir(&self) -> Result<PathBuf, AppError> {
        self.state()
            .device
            .as_ref()
            .map(|d| d.dir.clone())
            .ok_or_else(|| AppError::new("no_circle"))
    }

    fn renew_advert(&self) {
        let mut inner = self.state();
        let epoch = discovery::epoch_at(SystemTime::now());
        let Some(device) = inner.device.as_ref() else {
            inner.advert = None;
            return;
        };
        let port = device.port;
        let key = device
            .node
            .circle()
            .lock()
            .map(|c| c.own_discovery_key().key)
            .ok();
        let visible = inner.settings.visible;
        inner.advert = match (visible, key, discovery::new_instance_name()) {
            (true, Some(key), Ok(instance)) => Some(Advert {
                instance,
                tag: discovery::tag(&key, epoch),
                port,
            }),
            _ => None,
        };
        inner.advert_epoch = epoch;
    }

    fn set_name(&self, name: String) -> Reply {
        let name = display_name(name.trim());
        if name.is_empty() || name.len() > pairing::MAX_NAME_BYTES {
            return Err(AppError::new("bad_name"));
        }
        self.state().settings.name = Some(name);
        self.save_settings();
        self.publish();
        Ok(Value::Null)
    }

    fn set_settings(
        &self,
        default_mode: Option<String>,
        folder: Option<String>,
        visible: Option<bool>,
        confirm_new: Option<bool>,
        ui: Option<Value>,
    ) -> Reply {
        if let Some(mode) = default_mode {
            let parsed = parse_mode(&mode)?;
            let mut inner = self.state();
            if let Some(device) = inner.device.as_ref() {
                device.receive.set_default_mode(parsed);
            }
            inner.settings.default_mode = mode;
        }
        if let Some(path) = folder {
            let chosen = (!path.is_empty()).then_some(path);
            let target = chosen
                .clone()
                .map_or_else(|| self.config.default_folder.clone(), PathBuf::from);
            let opened = ReceiveFolder::open(&target)
                .map_err(|e| AppError::with("folder_unavailable", e))?;
            let mut inner = self.state();
            if let Some(device) = inner.device.as_ref() {
                device.receive.set_folder(opened);
            }
            inner.settings.folder = chosen;
            inner.problem = None;
        }
        let mut renew = false;
        if let Some(visible) = visible {
            self.state().settings.visible = visible;
            renew = true;
        }
        if let Some(confirm_new) = confirm_new {
            self.state().settings.confirm_new = confirm_new;
        }
        if let Some(ui) = ui {
            self.state().settings.ui = ui;
        }
        if renew {
            self.renew_advert();
        }
        self.save_settings();
        self.publish();
        Ok(Value::Null)
    }

    fn set_device_mode(&self, device: &str, mode: Option<String>) -> Reply {
        let id = unhex::<32>(device)?;
        let parsed = mode.as_deref().map(parse_mode).transpose()?;
        {
            let mut inner = self.state();
            if let Some(d) = inner.device.as_ref()
                && !inner.settings.unconfirmed.contains(device)
            {
                d.receive.set_device_mode(id, parsed);
            }
            match mode {
                Some(mode) => {
                    inner.settings.device_modes.insert(device.to_string(), mode);
                }
                None => {
                    inner.settings.device_modes.remove(device);
                }
            }
        }
        self.save_settings();
        self.publish();
        Ok(Value::Null)
    }

    /// Records devices that appeared since last time. With the setting on, one
    /// added through another device is refused here until the owner approves it.
    fn note_new_members(&self) {
        let view = {
            let inner = self.state();
            inner.device.as_ref().and_then(|d| {
                d.node.circle().lock().ok().map(|c| {
                    let me = c.device_id();
                    (
                        me,
                        c.ledger().membership().members,
                        c.ledger().devices_added_by(&me),
                    )
                })
            })
        };
        let Some((me, members, added_by_me)) = view else {
            return;
        };
        let mut changed = false;
        let mut inner = self.state();
        if inner.last_members != members {
            // News of a change travels by talking to the others soon.
            inner.last_members.clone_from(&members);
            inner.next_sync.clear();
        }
        let first_time = inner.settings.known.is_empty();
        for id in members.iter().filter(|id| **id != me) {
            let text = hex(id);
            if !inner.settings.known.insert(text.clone()) {
                continue;
            }
            changed = true;
            if inner.settings.confirm_new && !first_time && !added_by_me.contains(id) {
                inner.settings.unconfirmed.insert(text);
                if let Some(d) = inner.device.as_ref() {
                    d.receive.set_device_mode(*id, Some(ReceiveMode::Refuse));
                }
            }
        }
        drop(inner);
        if changed {
            self.save_settings();
        }
    }

    fn confirm_device(&self, device: &str, accept: bool) -> Reply {
        let id = unhex::<32>(device)?;
        if !accept {
            self.state().settings.unconfirmed.remove(device);
            self.save_settings();
            return self.remove_device(device);
        }
        {
            let mut inner = self.state();
            inner.settings.unconfirmed.remove(device);
            let own = inner
                .settings
                .device_modes
                .get(device)
                .and_then(|m| parse_mode(m).ok());
            if let Some(d) = inner.device.as_ref() {
                d.receive.set_device_mode(id, own);
            }
        }
        self.save_settings();
        self.publish();
        Ok(Value::Null)
    }

    fn remove_device(&self, device: &str) -> Reply {
        let id = unhex::<32>(device)?;
        let node = self.node()?;
        node.circle()
            .lock()
            .map_err(|_| AppError::new("state"))?
            .revoke(id)?;
        let _ = node.publish_membership();
        {
            let mut inner = self.state();
            inner.settings.device_modes.remove(device);
            inner.settings.unconfirmed.remove(device);
            inner.settings.clipboard_devices.remove(device);
            inner.last_addresses.remove(&id);
            inner.last_contact.remove(&id);
            // Anyone still here should hear about it soon.
            inner.next_sync.clear();
        }
        self.renew_advert();
        self.save_settings();
        self.publish();
        Ok(Value::Null)
    }

    /// Forgets the circle on this device so that it can start fresh.
    fn start_over(&self) -> Reply {
        {
            let mut inner = self.state();
            if let Some(device) = inner.device.as_ref()
                && let Ok(mut circle) = device.node.circle().lock()
            {
                // Leaving is best effort: others learn of it if a sync still happens.
                let me = circle.device_id();
                let _ = circle.revoke(me);
            }
            inner.device = None;
            inner.instances.clear();
            inner.last_addresses.clear();
            inner.last_contact.clear();
            inner.next_sync.clear();
            inner.advert = None;
            inner.settings.known.clear();
            inner.settings.unconfirmed.clear();
            inner.settings.device_modes.clear();
            inner.settings.clipboard_devices.clear();
            inner.clip = clipboard::ClipState::default();
            let next = format!(
                "device-{}",
                discovery::new_instance_name().unwrap_or_else(|_| now_ms().to_string())
            );
            inner.settings.circle_dir = next;
        }
        self.save_settings();
        self.publish();
        Ok(Value::Null)
    }

    fn set_pairing(&self, change: impl FnOnce(&mut PairingState)) {
        if let Some(state) = self.state().pairing.as_mut() {
            change(state);
        }
    }

    fn pairing_prompt(self: &Arc<Self>, prompt: &PairingPrompt) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let name = display_name(&prompt.peer_name);
        self.set_pairing(|p| {
            p.phase = "compare";
            p.comparison = Some(prompt.comparison.clone());
            p.peer_name = Some(name);
            p.peer_platform = Some(platform_name(prompt.peer_platform));
            p.peer_short_id = Some(prompt.peer_short_id.clone());
            p.answer = Some(tx);
        });
        self.publish();
        rx
    }

    fn end_pairing(&self, outcome: &str, device: Option<(String, String)>) {
        self.state().pairing = None;
        let (id, name) = device.unwrap_or_default();
        self.emit(
            &json!({ "event": "pairing_ended", "outcome": outcome, "device": id, "name": name }),
        );
        self.publish();
    }

    async fn pair_host(self: &Arc<Self>) -> Reply {
        if self.state().pairing.is_some() {
            return Err(AppError::new("pairing_busy"));
        }
        if self.state().device.is_none() {
            let name = self
                .state()
                .settings
                .name
                .clone()
                .ok_or_else(|| AppError::new("bad_name"))?;
            let dir = self.circle_dir();
            std::fs::create_dir_all(&dir)?;
            let circle = Circle::create_sealed(&dir, &name, &self.config.sealing)?;
            self.start_device(circle, dir).await?;
        }
        let node = self.node()?;
        let listener = bind_preferring(PREFERRED_PAIRING_PORT).await?;
        let port = listener.local_addr()?.port();
        let secret = PairingSecret::new_code()?;
        let code = secret.display_code().unwrap_or_default().to_string();
        let room = discovery::new_room_label()?;
        let (name, nets) = {
            let inner = self.state();
            (
                inner.settings.name.clone().unwrap_or_default(),
                inner.nets.clone(),
            )
        };
        let info = PeerInfo {
            name,
            platform: this_platform(),
        };
        let deadline_ms =
            now_ms() + i64::try_from(pairing::PAIRING_WINDOW.as_millis()).unwrap_or(120_000);
        self.state().pairing = Some(PairingState {
            role: "host",
            phase: "waiting",
            code: Some(code),
            room: Some(room),
            port: Some(port),
            deadline_ms,
            ..PairingState::default()
        });
        let this = self.clone();
        let task = self.handle.spawn(async move {
            let asker = this.clone();
            let handle = node.circle().clone();
            let confirm = move |prompt: PairingPrompt| {
                let rx = asker.pairing_prompt(&prompt);
                async move { rx.await.unwrap_or(false) }
            };
            let result = pairing::host(listener, &nets, &handle, &secret, info, confirm).await;
            match result {
                Ok(paired) => {
                    let _ = node.publish_membership();
                    let id = hex(&paired.device);
                    let platform = this.state().pairing.as_ref().and_then(|p| p.peer_platform);
                    if let Some(platform) = platform {
                        this.state()
                            .settings
                            .platforms
                            .insert(hex(&paired.device), platform.to_string());
                    }
                    this.note_new_members();
                    this.end_pairing("paired", Some((id, display_name(&paired.name))));
                    // Everyone should hear about the new device straight away.
                    this.state().next_sync.clear();
                    this.probe_peers();
                }
                Err(e) => this.end_pairing(explain_pairing(&e), None),
            }
        });
        self.set_pairing(|p| p.task = Some(task.abort_handle()));
        self.publish();
        Ok(Value::Null)
    }

    fn pair_join(self: &Arc<Self>, address: &str, typed: &str) -> Reply {
        if self.state().pairing.is_some() {
            return Err(AppError::new("pairing_busy"));
        }
        let address: SocketAddr = address.parse().map_err(|_| AppError::new("bad_address"))?;
        if typed.len() != 6 || !typed.bytes().all(|b| b.is_ascii_digit()) {
            return Err(AppError::new("wrong_code"));
        }
        // A device already sharing a circle with others must leave it first.
        let alone = {
            let inner = self.state();
            match inner.device.as_ref() {
                Some(d) => d
                    .node
                    .circle()
                    .lock()
                    .is_ok_and(|c| c.ledger().membership().members.len() <= 1),
                None => true,
            }
        };
        if !alone {
            return Err(AppError::new("already_in_circle"));
        }
        let name = self
            .state()
            .settings
            .name
            .clone()
            .ok_or_else(|| AppError::new("bad_name"))?;
        let dir_name = format!("device-{}", discovery::new_instance_name()?);
        let dir = self.config.state_dir.join(&dir_name);
        std::fs::create_dir_all(&dir)?;
        let nets = self.state().nets.clone();
        let info = PeerInfo {
            name,
            platform: this_platform(),
        };
        let secret = PairingSecret::from_typed(typed);
        let key = generate_key()?;
        let deadline_ms =
            now_ms() + i64::try_from(pairing::PAIRING_WINDOW.as_millis()).unwrap_or(120_000);
        self.state().pairing = Some(PairingState {
            role: "join",
            phase: "connecting",
            deadline_ms,
            ..PairingState::default()
        });
        let this = self.clone();
        let task = self.handle.spawn(async move {
            let asker = this.clone();
            let confirm = move |prompt: PairingPrompt| {
                let rx = asker.pairing_prompt(&prompt);
                async move { rx.await.unwrap_or(false) }
            };
            let sealing = this.config.sealing.clone();
            let joined = pairing::join_sealed(
                address,
                &nets,
                &dir,
                key,
                &secret,
                HostPin::None,
                info,
                &sealing,
                confirm,
            )
            .await;
            match joined {
                Ok(circle) => this.adopt_joined(circle, dir, dir_name).await,
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&dir);
                    this.end_pairing(explain_pairing(&e), None);
                }
            }
        });
        self.set_pairing(|p| p.task = Some(task.abort_handle()));
        self.publish();
        Ok(Value::Null)
    }

    async fn adopt_joined(self: &Arc<Self>, circle: Circle, dir: PathBuf, dir_name: String) {
        let platform = self.state().pairing.as_ref().and_then(|p| p.peer_platform);
        let host = circle.ledger().genesis_author();
        let old = {
            let mut inner = self.state();
            if let (Some(platform), Some(host)) = (platform, host) {
                inner
                    .settings
                    .platforms
                    .insert(hex(&host), platform.to_string());
            }
            inner.settings.circle_dir = dir_name;
            inner.settings.known.clear();
            inner.settings.unconfirmed.clear();
            inner.settings.device_modes.clear();
            inner.device.take().map(|d| d.dir.clone())
        };
        self.save_settings();
        if let Some(old) = old {
            let _ = std::fs::remove_dir_all(old);
        }
        match self.start_device(circle, dir).await {
            Ok(()) => {
                let name = host
                    .and_then(|h| {
                        self.node().ok().and_then(|n| {
                            n.circle()
                                .lock()
                                .ok()
                                .and_then(|c| c.ledger().name_of(&h).map(display_name))
                        })
                    })
                    .unwrap_or_default();
                self.end_pairing(
                    "paired",
                    Some((host.map(|h| hex(&h)).unwrap_or_default(), name)),
                );
                self.probe_peers();
            }
            Err(e) => self.end_pairing(e.kind, None),
        }
    }

    fn pair_answer(&self, accept: bool) -> Reply {
        let answer = self.state().pairing.as_mut().and_then(|p| {
            p.phase = "finishing";
            p.answer.take()
        });
        match answer {
            Some(answer) => {
                let _ = answer.send(accept);
                self.publish();
                Ok(Value::Null)
            }
            None => Err(AppError::new("not_found")),
        }
    }

    fn pair_cancel(&self) -> Reply {
        let task = self.state().pairing.as_mut().and_then(|p| p.task.take());
        if let Some(task) = task {
            task.abort();
        }
        self.end_pairing("cancelled", None);
        Ok(Value::Null)
    }

    fn discovery_seen(
        self: &Arc<Self>,
        instance: String,
        addresses: &[String],
        port: u16,
        tag: Option<String>,
    ) -> Reply {
        let nets = self.state().nets.clone();
        let mut parsed: Vec<SocketAddr> = addresses
            .iter()
            .filter_map(|a| a.parse::<IpAddr>().ok())
            // The listener is IPv4 only for now.
            .filter(IpAddr::is_ipv4)
            // An address the transport would refuse is not worth trying.
            .filter(|ip| nets.is_local(*ip))
            .map(|ip| SocketAddr::new(ip, port))
            .collect();
        parsed.sort();
        parsed.dedup();
        let new = self
            .state()
            .instances
            .insert(
                instance,
                Seen {
                    addresses: parsed,
                    tag,
                },
            )
            .is_none();
        self.refresh_presence();
        if new {
            self.probe_peers();
        }
        Ok(Value::Null)
    }

    /// Addresses to try for a device, best first. All of them are only hints:
    /// each attempt is pinned to this device's key.
    fn candidates(&self, peer: &DeviceId) -> Vec<SocketAddr> {
        let inner = self.state();
        let keys = inner
            .device
            .as_ref()
            .and_then(|d| d.node.circle().lock().ok().map(|c| c.peer_discovery_keys()))
            .unwrap_or_default();
        let epoch = discovery::epoch_at(SystemTime::now());
        let own = inner.advert.as_ref().map(|a| a.instance.as_str());
        let mut tagged = Vec::new();
        let mut untagged = Vec::new();
        for (name, seen) in &inner.instances {
            if Some(name.as_str()) == own {
                continue;
            }
            let owner = seen
                .tag
                .as_deref()
                .and_then(|t| discovery::recognize(keys.iter(), t, epoch));
            match owner {
                Some(id) if id == *peer => tagged.extend(seen.addresses.iter().copied()),
                Some(_) => {}
                None => untagged.extend(seen.addresses.iter().copied()),
            }
        }
        untagged.truncate(MAX_UNTAGGED_ATTEMPTS);
        let mut out = tagged;
        out.extend(
            inner
                .last_addresses
                .get(peer)
                .into_iter()
                .flatten()
                .copied(),
        );
        out.extend(untagged);
        let mut seen = BTreeSet::new();
        out.retain(|a| seen.insert(*a));
        out
    }

    fn remember_address(&self, peer: DeviceId, address: SocketAddr) {
        let mut inner = self.state();
        let list = inner.last_addresses.entry(peer).or_default();
        list.retain(|a| *a != address);
        list.insert(0, address);
        list.truncate(4);
        inner.last_contact.insert(peer, Instant::now());
    }

    fn peer_lock(&self, peer: DeviceId) -> Arc<tokio::sync::Mutex<()>> {
        self.state().peer_locks.entry(peer).or_default().clone()
    }

    fn peer_display_name(&self, node: &Node, peer: &DeviceId) -> String {
        node.circle()
            .lock()
            .ok()
            .and_then(|c| c.ledger().name_of(peer).map(display_name))
            .unwrap_or_else(|| short_id(peer))
    }

    fn check_can_send_to(&self, node: &Node, peer: &DeviceId) -> Result<(), AppError> {
        let member = node
            .circle()
            .lock()
            .map_err(|_| AppError::new("state"))?
            .is_member(peer);
        if !member {
            return Err(AppError::new("peer_removed"));
        }
        if self.state().settings.unconfirmed.contains(&hex(peer)) {
            return Err(AppError::new("awaiting_confirmation"));
        }
        Ok(())
    }

    async fn send(self: &Arc<Self>, device: &str, paths: Vec<String>) -> Reply {
        let peer = unhex::<32>(device)?;
        let node = self.node()?;
        self.check_can_send_to(&node, &peer)?;
        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        let outgoing = tokio::task::spawn_blocking(move || transfer::plan(&paths))
            .await
            .map_err(|_| AppError::new("state"))?
            .map_err(|e| match e {
                TransferError::Source(_) => AppError::with("source_unreadable", e),
                _ => AppError::with("nothing_to_send", e),
            })?;
        let id = outgoing.transfer_id;
        let files = outgoing.items.iter().filter(|i| i.source.is_some()).count();
        let control = Control::new();
        let name = outgoing
            .items
            .first()
            .and_then(|i| i.item.path.first())
            .map(|n| display_name(n))
            .unwrap_or_default();
        let total = outgoing.total_bytes;
        self.add_active(
            id,
            Active {
                direction: "sent",
                peer,
                peer_name: self.peer_display_name(&node, &peer),
                files: u32::try_from(files).unwrap_or(u32::MAX),
                total,
                name,
                control: control.clone(),
                task: None,
                last_done: 0,
                last_at: Instant::now(),
                speed: 0.0,
            },
        );
        self.publish();
        let this = self.clone();
        let task = self.handle.spawn(async move {
            let lock = this.peer_lock(peer);
            let guard = lock.lock().await;
            let mut result = None;
            for address in this.candidates(&peer) {
                if control.is_cancelled() {
                    break;
                }
                match node
                    .send_with(peer, address, outgoing.clone(), &control)
                    .await
                {
                    Err(NodeError::Tls(_)) => {}
                    other => {
                        if other.is_ok() {
                            this.remember_address(peer, address);
                        }
                        result = Some(other);
                        break;
                    }
                }
            }
            drop(guard);
            this.finish_outgoing(id, result).await;
        });
        if let Some(active) = self.state().transfers.get_mut(&id) {
            active.task = Some(task.abort_handle());
        }
        Ok(json!({ "transfer": hex(&id) }))
    }

    fn resume(self: &Arc<Self>, transfer: &str) -> Reply {
        let id = unhex::<16>(transfer)?;
        let node = self.node()?;
        let record = self
            .state()
            .unfinished
            .iter()
            .find(|u| u.transfer_id == id)
            .cloned()
            .ok_or_else(|| AppError::new("not_found"))?;
        self.check_can_send_to(&node, &record.peer)?;
        if self.state().transfers.contains_key(&id) {
            return Err(AppError::new("busy"));
        }
        let control = Control::new();
        let peer = record.peer;
        self.add_active(
            id,
            Active {
                direction: "sent",
                peer,
                peer_name: self.peer_display_name(&node, &peer),
                files: record.files,
                total: record.total_bytes,
                name: display_name(&record.first_name),
                control: control.clone(),
                task: None,
                last_done: 0,
                last_at: Instant::now(),
                speed: 0.0,
            },
        );
        self.state().unfinished.retain(|u| u.transfer_id != id);
        self.publish();
        let this = self.clone();
        let task = self.handle.spawn(async move {
            let lock = this.peer_lock(peer);
            let guard = lock.lock().await;
            let mut result = None;
            for address in this.candidates(&peer) {
                if control.is_cancelled() {
                    break;
                }
                match node.resume_with(peer, address, id, &control).await {
                    Err(NodeError::Tls(_)) => {}
                    other => {
                        if other.is_ok() {
                            this.remember_address(peer, address);
                        }
                        result = Some(other);
                        break;
                    }
                }
            }
            drop(guard);
            this.finish_outgoing(id, result).await;
        });
        if let Some(active) = self.state().transfers.get_mut(&id) {
            active.task = Some(task.abort_handle());
        }
        Ok(json!({ "transfer": transfer }))
    }

    fn add_active(&self, id: [u8; 16], active: Active) {
        self.state().transfers.insert(id, active);
    }

    fn record_history(&self, id: &[u8; 16], active: &Active, outcome: &str, held: u32) {
        let entry = HistoryEntry {
            id: hex(id),
            at_ms: now_ms(),
            direction: active.direction.to_string(),
            peer_name: active.peer_name.clone(),
            files: active.files,
            bytes: active.total,
            name: active.name.clone(),
            outcome: outcome.to_string(),
            held,
        };
        {
            let mut inner = self.state();
            inner.history.insert(0, entry);
            inner.history.truncate(MAX_HISTORY);
        }
        self.save_history();
    }

    fn announce_end(
        &self,
        id: &[u8; 16],
        active: &Active,
        outcome: &str,
        can_resume: bool,
        held: u32,
    ) {
        self.emit(&json!({
            "event": "transfer_ended",
            "id": hex(id),
            "direction": active.direction,
            "peer": hex(&active.peer),
            "peer_name": active.peer_name,
            "files": active.files,
            "bytes": active.total,
            "name": active.name,
            "outcome": outcome,
            "can_resume": can_resume,
            "held": held,
        }));
    }

    async fn finish_outgoing(
        self: &Arc<Self>,
        id: [u8; 16],
        result: Option<Result<SendReport, NodeError>>,
    ) {
        let Some(active) = self.state().transfers.remove(&id) else {
            return;
        };
        let cancelled_here = active.control.is_cancelled();
        let (outcome, can_resume, held) = match &result {
            Some(Ok(report)) => ("done", false, report.held),
            Some(Err(error)) => {
                debug_log(format_args!("sending stopped: {error}"));
                let (kind, can_resume) = explain_transfer(error, cancelled_here);
                (kind, can_resume, 0)
            }
            None if cancelled_here => ("cancelled", false, 0),
            None => ("unreachable", false, 0),
        };
        if !can_resume && let Ok(dir) = self.device_state_dir() {
            let _ = transfer::forget_unfinished(&dir, &id);
        }
        self.record_history(&id, &active, outcome, held);
        self.announce_end(&id, &active, outcome, can_resume, held);
        self.reload_lists().await;
        self.publish();
    }

    fn cancel(self: &Arc<Self>, transfer: &str) -> Reply {
        let id = unhex::<16>(transfer)?;
        {
            let inner = self.state();
            let active = inner
                .transfers
                .get(&id)
                .ok_or_else(|| AppError::new("not_found"))?;
            active.control.cancel();
        }
        // A stalled connection should not keep the owner waiting.
        let this = self.clone();
        self.handle.spawn(async move {
            tokio::time::sleep(CANCEL_GRACE).await;
            let stuck = this.state().transfers.get(&id).and_then(|a| a.task.clone());
            if let Some(task) = stuck {
                task.abort();
                this.finish_outgoing(id, None).await;
            }
        });
        Ok(Value::Null)
    }

    async fn forget_unfinished(&self, transfer: &str) -> Reply {
        let id = unhex::<16>(transfer)?;
        let dir = self.device_state_dir()?;
        transfer::forget_unfinished(&dir, &id)?;
        self.reload_lists().await;
        self.publish();
        Ok(Value::Null)
    }

    fn answer_offer(&self, offer: u64, accept: bool) -> Reply {
        let pending = self
            .state()
            .offers
            .remove(&offer)
            .ok_or_else(|| AppError::new("not_found"))?;
        let _ = pending.answer.send(accept);
        self.publish();
        Ok(Value::Null)
    }

    async fn settle_held(self: &Arc<Self>, transfer: &str, name: String, release: bool) -> Reply {
        let id = unhex::<16>(transfer)?;
        let folder = self
            .state()
            .device
            .as_ref()
            .map(|d| d.receive.folder())
            .ok_or_else(|| AppError::new("no_circle"))?;
        tokio::task::spawn_blocking(move || {
            if release {
                receive::release_held(&folder, &id, &name)
            } else {
                receive::discard_held(&folder, &id, &name)
            }
        })
        .await
        .map_err(|_| AppError::new("state"))??;
        self.reload_lists().await;
        self.publish();
        Ok(Value::Null)
    }

    async fn reload_lists(&self) {
        let context = self
            .state()
            .device
            .as_ref()
            .map(|d| (d.dir.clone(), d.receive.folder()));
        let Some((dir, folder)) = context else {
            let mut inner = self.state();
            inner.unfinished.clear();
            inner.held.clear();
            return;
        };
        let lists = tokio::task::spawn_blocking(move || {
            (
                transfer::unfinished(&dir).unwrap_or_default(),
                receive::list_held(&folder).unwrap_or_default(),
            )
        })
        .await;
        if let Ok((unfinished, held)) = lists {
            let mut inner = self.state();
            let running: BTreeSet<[u8; 16]> = inner.transfers.keys().copied().collect();
            inner.unfinished = unfinished
                .into_iter()
                .filter(|u| !running.contains(&u.transfer_id))
                .collect();
            inner.held = held;
        }
    }

    async fn sweep(&self) {
        let context = self
            .state()
            .device
            .as_ref()
            .map(|d| (d.dir.clone(), d.receive.folder()));
        let Some((dir, folder)) = context else {
            return;
        };
        let _ = tokio::task::spawn_blocking(move || {
            let _ = transfer::sweep_outgoing(&dir, KEEP_UNFINISHED);
            let _ = receive::sweep_expired(&folder, KEEP_UNFINISHED);
        })
        .await;
    }

    async fn ticker(self: Arc<Self>) {
        let mut beat = 0u64;
        loop {
            // With nothing moving there is nothing to report four times a second.
            let steps = if self.state().transfers.is_empty() {
                TICKS_WHEN_IDLE
            } else {
                1
            };
            tokio::time::sleep(TICK * u32::try_from(steps).unwrap_or(1)).await;
            let previous = beat;
            beat = beat.wrapping_add(steps);
            let due = |every: u64| beat / every != previous / every;
            self.report_progress();
            if due(TICKS_PER_PRESENCE) {
                self.refresh_networks();
                self.refresh_presence();
            }
            if due(TICKS_PER_PROBE) {
                self.probe_peers();
            }
            if due(TICKS_PER_SWEEP) {
                self.sweep().await;
                self.reload_lists().await;
                self.publish();
            }
        }
    }

    fn report_progress(&self) {
        let mut items = Vec::new();
        {
            let mut inner = self.state();
            for (id, active) in &mut inner.transfers {
                let done = active.control.bytes_done();
                let now = Instant::now();
                let elapsed = now.duration_since(active.last_at).as_secs_f64().max(0.001);
                #[expect(clippy::cast_precision_loss, reason = "only shown to a person")]
                let current = done.saturating_sub(active.last_done) as f64 / elapsed;
                active.speed = if active.speed == 0.0 {
                    current
                } else {
                    active.speed.mul_add(0.8, current * 0.2)
                };
                active.last_done = done;
                active.last_at = now;
                items.push(json!({ "id": hex(id), "done": done, "speed": active.speed }));
            }
        }
        if !items.is_empty() {
            self.emit(&json!({ "event": "progress", "items": items }));
        }
    }

    fn refresh_networks(&self) {
        let (allow, old) = {
            let inner = self.state();
            (inner.allow_loopback, inner.nets.addresses())
        };
        let nets = Self::detect_networks(allow);
        let moved = nets.addresses() != old;
        let mut inner = self.state();
        if moved {
            if let Some(d) = inner.device.as_ref() {
                d.node.set_networks(nets.clone());
            }
            inner.nets = nets;
            inner.instances.clear();
        }
        let stale = inner.advert_epoch != discovery::epoch_at(SystemTime::now());
        drop(inner);
        if moved || stale {
            self.renew_advert();
            self.publish();
        }
    }

    fn compute_presence(&self) -> BTreeSet<DeviceId> {
        let inner = self.state();
        let keys = inner
            .device
            .as_ref()
            .and_then(|d| d.node.circle().lock().ok().map(|c| c.peer_discovery_keys()))
            .unwrap_or_default();
        let epoch = discovery::epoch_at(SystemTime::now());
        let mut nearby: BTreeSet<DeviceId> = inner
            .instances
            .values()
            .filter_map(|s| {
                s.tag
                    .as_deref()
                    .and_then(|t| discovery::recognize(keys.iter(), t, epoch))
            })
            .collect();
        nearby.extend(
            inner
                .last_contact
                .iter()
                .filter(|(_, at)| at.elapsed() < NEARBY_AFTER_CONTACT)
                .map(|(id, _)| *id),
        );
        nearby
    }

    fn refresh_presence(&self) {
        self.note_new_members();
        let nearby = self.compute_presence();
        let changed = {
            let mut inner = self.state();
            let changed = inner.presence != nearby;
            inner.presence = nearby;
            changed
        };
        if changed {
            self.publish();
        }
    }

    /// Talks to each visible device now and then, which carries removals and
    /// new devices around the circle and teaches devices each other's keys.
    fn probe_peers(self: &Arc<Self>) {
        let Ok(node) = self.node() else {
            return;
        };
        let others: Vec<DeviceId> = node
            .circle()
            .lock()
            .map(|c| {
                let me = c.device_id();
                c.ledger()
                    .membership()
                    .members
                    .into_iter()
                    .filter(|id| *id != me)
                    .collect()
            })
            .unwrap_or_default();
        for peer in others {
            let due = self
                .state()
                .next_sync
                .get(&peer)
                .is_none_or(|at| *at <= Instant::now());
            if !due {
                continue;
            }
            let candidates = self.candidates(&peer);
            if candidates.is_empty() {
                continue;
            }
            self.state()
                .next_sync
                .insert(peer, Instant::now() + SYNC_AFTER_FAILURE);
            let lock = self.peer_lock(peer);
            let this = self.clone();
            let node = node.clone();
            self.handle.spawn(async move {
                // A second connection would replace one already carrying a transfer.
                let Ok(guard) = lock.try_lock() else {
                    return;
                };
                for address in candidates {
                    if node.sync_with(peer, address).await.is_ok() {
                        this.remember_address(peer, address);
                        this.state()
                            .next_sync
                            .insert(peer, Instant::now() + SYNC_AFTER_SUCCESS);
                        break;
                    }
                }
                drop(guard);
                let _ = node.publish_membership();
                this.refresh_presence();
                this.publish();
            });
        }
    }

    fn publish(&self) {
        let message = self.snapshot();
        self.emit(&message);
    }

    fn snapshot(&self) -> Value {
        let nearby = self.compute_presence();
        let inner = self.state();
        let mut me = Value::Null;
        let mut devices = Vec::new();
        if let Some(device) = inner.device.as_ref()
            && let Ok(circle) = device.node.circle().lock()
        {
            let my_id = circle.device_id();
            let view = circle.ledger().membership();
            me = json!({
                "id": hex(&my_id),
                "short_id": short_id(&my_id),
                "name": display_name(circle.ledger().name_of(&my_id).unwrap_or("")),
                "platform": platform_name(this_platform()),
                "removed": !view.members.contains(&my_id),
            });
            for id in view.members.iter().filter(|id| **id != my_id) {
                let text = hex(id);
                let platform = circle
                    .ledger()
                    .platform_of(id)
                    .map(platform_name)
                    .or_else(|| inner.settings.platforms.get(&text).map(String::as_str))
                    .unwrap_or("other");
                devices.push(json!({
                    "id": text,
                    "short_id": short_id(id),
                    "name": display_name(circle.ledger().name_of(id).unwrap_or("?")),
                    "platform": platform,
                    "nearby": nearby.contains(id),
                    "mode": inner.settings.device_modes.get(&text),
                    "awaiting_confirmation": inner.settings.unconfirmed.contains(&text),
                    "clipboard": inner.settings.clipboard_devices.get(&text).copied().unwrap_or_default(),
                }));
            }
            devices.sort_by_key(|d| d["name"].as_str().unwrap_or("").to_lowercase());
        }
        let folder = inner.device.as_ref().map_or_else(
            || {
                inner
                    .settings
                    .folder
                    .clone()
                    .unwrap_or_else(|| self.config.default_folder.display().to_string())
            },
            |d| d.receive.folder().path().display().to_string(),
        );
        let pairing = inner.pairing.as_ref().map_or(Value::Null, |p| {
            json!({
                "role": p.role,
                "phase": p.phase,
                "code": p.code,
                "room": p.room,
                "port": p.port,
                "comparison": p.comparison,
                "peer_name": p.peer_name,
                "peer_platform": p.peer_platform,
                "peer_short_id": p.peer_short_id,
                "deadline_ms": p.deadline_ms,
            })
        });
        let offers: Vec<Value> = inner
            .offers
            .iter()
            .map(|(id, o)| {
                json!({
                    "id": id,
                    "transfer": hex(&o.transfer),
                    "peer": hex(&o.peer),
                    "peer_name": o.peer_name,
                    "files": o.files,
                    "total_bytes": o.total,
                    "has_program": o.has_program,
                    "first_program": o.first_program,
                    "names": o.names,
                })
            })
            .collect();
        let transfers: Vec<Value> = inner
            .transfers
            .iter()
            .map(|(id, a)| {
                json!({
                    "id": hex(id),
                    "direction": a.direction,
                    "peer": hex(&a.peer),
                    "peer_name": a.peer_name,
                    "files": a.files,
                    "total_bytes": a.total,
                    "name": a.name,
                    "done": a.control.bytes_done(),
                    "speed": a.speed,
                })
            })
            .collect();
        let name_of = |id: &DeviceId| {
            inner
                .device
                .as_ref()
                .and_then(|d| {
                    d.node
                        .circle()
                        .lock()
                        .ok()
                        .and_then(|c| c.ledger().name_of(id).map(display_name))
                })
                .unwrap_or_else(|| short_id(id))
        };
        let unfinished: Vec<Value> = inner
            .unfinished
            .iter()
            .map(|u| {
                json!({
                    "id": hex(&u.transfer_id),
                    "peer": hex(&u.peer),
                    "peer_name": name_of(&u.peer),
                    "files": u.files,
                    "total_bytes": u.total_bytes,
                    "name": display_name(&u.first_name),
                })
            })
            .collect();
        let held: Vec<Value> = inner
            .held
            .iter()
            .map(|h| {
                json!({
                    "transfer": hex(&h.transfer_id),
                    "peer_name": name_of(&h.peer),
                    "name": display_name(&h.name),
                    "raw_name": h.name,
                    "is_dir": h.is_dir,
                    "bytes": h.bytes,
                    "arrived_ms": h.arrived_ms,
                })
            })
            .collect();
        let advert = inner.advert.as_ref().map_or(
            Value::Null,
            |a| json!({ "instance": a.instance, "tag": a.tag, "port": a.port }),
        );
        json!({
            "event": "snapshot",
            "name": inner.settings.name,
            "me": me,
            "devices": devices,
            "settings": {
                "default_mode": inner.settings.default_mode,
                "folder": folder,
                "folder_is_default": inner.settings.folder.is_none(),
                "visible": inner.settings.visible,
                "confirm_new": inner.settings.confirm_new,
                "clipboard": inner.settings.clipboard_enabled,
                "ui": inner.settings.ui,
            },
            "problem": inner.problem,
            "pairing": pairing,
            "offers": offers,
            "transfers": transfers,
            "unfinished": unfinished,
            "held": held,
            "history": inner.history,
            "advert": advert,
        })
    }
}

fn fallback_folder(state: &Path) -> ReceiveFolder {
    // `open` only fails when the directory cannot be created, and then every
    // later use reports the same error to the sender as a failed offer.
    match ReceiveFolder::open(&state.join("received")) {
        Ok(folder) => folder,
        Err(_) => ReceiveFolder::unchecked(state),
    }
}

async fn bind_preferring(port: u16) -> std::io::Result<TcpListener> {
    match TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await {
        Ok(listener) => Ok(listener),
        Err(_) => TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).await,
    }
}

struct AppApprover(Arc<Shared>);

struct OfferGuard {
    shared: Arc<Shared>,
    offer: u64,
}

impl Drop for OfferGuard {
    fn drop(&mut self) {
        let removed = self.shared.state().offers.remove(&self.offer).is_some();
        if removed {
            self.shared.publish();
        }
    }
}

impl AppApprover {
    fn peer_name(&self, peer: &DeviceId) -> String {
        self.0
            .node()
            .map_or_else(|_| short_id(peer), |n| self.0.peer_display_name(&n, peer))
    }
}

impl Approver for AppApprover {
    fn approve(&self, offer: OfferSummary) -> Pin<Box<dyn Future<Output = bool> + Send>> {
        let shared = self.0.clone();
        let peer_name = self.peer_name(&offer.peer);
        Box::pin(async move {
            let (tx, rx) = oneshot::channel();
            let id = {
                let mut inner = shared.state();
                inner.next_offer += 1;
                let id = inner.next_offer;
                inner.asked.insert(
                    offer.transfer_id,
                    (
                        peer_name.clone(),
                        offer.files,
                        offer.total_bytes,
                        offer.names.first().cloned().unwrap_or_default(),
                    ),
                );
                inner.offers.insert(
                    id,
                    PendingOffer {
                        transfer: offer.transfer_id,
                        peer: offer.peer,
                        peer_name,
                        files: offer.files,
                        total: offer.total_bytes,
                        has_program: offer.has_program,
                        first_program: offer.first_program,
                        names: offer.names,
                        answer: tx,
                    },
                );
                id
            };
            let _guard = OfferGuard {
                shared: shared.clone(),
                offer: id,
            };
            shared.emit(&json!({ "event": "offer_arrived", "id": id }));
            shared.publish();
            tokio::time::timeout(ASK_TIMEOUT, rx)
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or(false)
        })
    }

    fn receiving(&self, start: &ReceiveStart) -> Control {
        let control = Control::new();
        self.0.add_active(
            start.transfer_id,
            Active {
                direction: "received",
                peer: start.peer,
                peer_name: self.peer_name(&start.peer),
                files: start.files,
                total: start.total_bytes,
                name: start.names.first().cloned().unwrap_or_default(),
                control: control.clone(),
                task: None,
                last_done: start.already,
                last_at: Instant::now(),
                speed: 0.0,
            },
        );
        self.0
            .state()
            .last_contact
            .insert(start.peer, Instant::now());
        self.0.publish();
        control
    }

    fn finished(&self, _peer: &DeviceId, transfer: &[u8; 16], outcome: &OfferOutcome) {
        let shared = self.0.clone();
        let id = *transfer;
        let asked = shared.state().asked.remove(&id);
        match *outcome {
            OfferOutcome::Received(counts) => {
                let Some(active) = shared.state().transfers.remove(&id) else {
                    return;
                };
                shared.record_history(&id, &active, "done", counts.held);
                shared.announce_end(&id, &active, "done", false, counts.held);
            }
            OfferOutcome::Declined(reason) => {
                if reason != code::DECLINED {
                    return;
                }
                let Some((peer_name, files, bytes, name)) = asked else {
                    return;
                };
                {
                    let mut inner = shared.state();
                    inner.history.insert(
                        0,
                        HistoryEntry {
                            id: hex(&id),
                            at_ms: now_ms(),
                            direction: "received".to_string(),
                            peer_name,
                            files,
                            bytes,
                            name,
                            outcome: "declined".to_string(),
                            held: 0,
                        },
                    );
                    inner.history.truncate(MAX_HISTORY);
                }
                shared.save_history();
            }
        }
        let handle = shared.handle.clone();
        handle.spawn(async move {
            shared.reload_lists().await;
            shared.publish();
        });
    }

    fn interrupted(&self, _peer: &DeviceId, transfer: &[u8; 16], why: Option<&TransferError>) {
        let shared = self.0.clone();
        let id = *transfer;
        let Some(active) = shared.state().transfers.remove(&id) else {
            return;
        };
        let outcome = why.map_or("connection_lost", |error| {
            debug_log(format_args!("receiving stopped: {error}"));
            match error {
                TransferError::Cancelled(_) if active.control.is_cancelled() => "cancelled",
                TransferError::Cancelled(_) => "cancelled_by_peer",
                TransferError::Session(e) => explain_session(e).0,
                TransferError::Store(StoreError::NoSpace) => "no_space",
                TransferError::Store(StoreError::Integrity) => "integrity",
                TransferError::Store(_) => "storage",
                _ => "connection_lost",
            }
        });
        shared.record_history(&id, &active, outcome, 0);
        shared.announce_end(&id, &active, outcome, false, 0);
        shared.publish();
    }
}
