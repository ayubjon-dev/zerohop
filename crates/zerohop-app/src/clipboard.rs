//! The shared clipboard: what the owner allows, and moving copied content.
//!
//! Copied content is only ever in memory. It is never written to the settings,
//! the history, a snapshot, or a log, and this file keeps no copy of it.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use data_encoding::BASE64;
use serde::Serialize;
use serde_json::{Value, json};
use zerohop_core::circle::random_bytes;
use zerohop_core::clip::{ClipPart, ClipSink, MAX_CLIP_TTL, OutgoingClip, ReceivedClip};
use zerohop_core::ledger::DeviceId;
use zerohop_core::names::TargetOs;
use zerohop_core::node::{Node, NodeError};
use zerohop_core::receive::{ReceiveFolder, SafeItem, validate_manifest};
use zerohop_core::transfer::{
    self, Approver, Budgets, OfferOutcome, OfferSummary, Outgoing, ReceiveConfig, ReceiveMode,
    ReceiveSettings, TransferError,
};
use zerohop_core::wire::{ClipFiles, ClipFormat, ClipPull, ItemKind};
use zeroize::Zeroizing;

use super::store::{ClipDevice, Settings};
use super::{AppError, Reply, Shared, hex, unhex};

/// How long a copy stays on the other device.
const LIFETIME: Duration = MAX_CLIP_TTL;
/// Text this long is not something anyone means to paste on another device.
const MAX_TEXT: usize = 8 * 1024 * 1024;
/// Pictures cross the interface as text, which makes larger ones costly.
const MAX_IMAGE: usize = 24 * 1024 * 1024;
/// How long one device may take once the connection is free.
const SEND_LIMIT: Duration = Duration::from_secs(30);
/// How long the caller waits to hear how it went. Sending carries on regardless.
const REPORT_AFTER: Duration = Duration::from_secs(6);
/// The list of copied files, a few hundred bytes a file.
const MAX_FILE_LIST: usize = 16 * 1024 * 1024;
/// Where pasted files wait in the device's state folder while the system's
/// paste copies them on.
const PASTE_DIR: &str = "paste";
/// How long pasted files are kept for a paste that is still copying.
const PASTE_KEEP: Duration = Duration::from_secs(60 * 60);

/// What only this device remembers about the clipboard. Holds no content.
#[derive(Default)]
pub(super) struct ClipState {
    last_applied: Option<([u8; 32], Instant)>,
    generation: u64,
    /// Files copied here, which the devices they were offered to may fetch
    /// until the clip's lifetime ends.
    offered_files: Option<OfferedFiles>,
    /// Files copied on another device, which the owner may paste here.
    incoming_files: Option<IncomingFiles>,
}

struct OfferedFiles {
    clip_id: [u8; 16],
    peers: Vec<DeviceId>,
    outgoing: Outgoing,
    until: Instant,
}

struct IncomingFiles {
    clip_id: [u8; 16],
    peer: DeviceId,
    manifest: [u8; 32],
    /// The list with every name already made safe for this system, as the
    /// paste will write it.
    items: Arc<Vec<SafeItem>>,
    until: Instant,
    /// What an earlier paste of the same clip left, so pasting twice
    /// fetches once.
    pasted: Option<Pasted>,
}

/// Where a paste put the files: the folder made for it, and what is at its
/// top level.
#[derive(Clone)]
struct Pasted {
    folder: String,
    paths: Vec<String>,
}

/// A change to one device's clipboard rules. Unset fields stay as they are.
pub(super) struct DeviceChange {
    pub send: Option<bool>,
    pub receive: Option<bool>,
    pub text: Option<bool>,
    pub images: Option<bool>,
    pub files: Option<bool>,
}

#[derive(Clone, Copy)]
enum Kind {
    Text,
    Image,
    Files,
}

#[derive(Clone, Copy)]
enum Direction {
    Send,
    Receive,
}

const fn kind_of(format: ClipFormat) -> Option<Kind> {
    match format {
        ClipFormat::Text => Some(Kind::Text),
        ClipFormat::Png | ClipFormat::Jpeg => Some(Kind::Image),
        ClipFormat::Files => Some(Kind::Files),
        ClipFormat::Html | ClipFormat::Rtf => None,
    }
}

const fn limit_of(kind: Kind) -> usize {
    match kind {
        Kind::Text => MAX_TEXT,
        Kind::Image => MAX_IMAGE,
        Kind::Files => MAX_FILE_LIST,
    }
}

fn allows(settings: &Settings, device: &str, kind: Kind, direction: Direction) -> bool {
    if !settings.clipboard_enabled || settings.unconfirmed.contains(device) {
        return false;
    }
    let rules = settings
        .clipboard_devices
        .get(device)
        .copied()
        .unwrap_or_default();
    let way = match direction {
        Direction::Send => rules.send,
        Direction::Receive => rules.receive,
    };
    let content = match kind {
        Kind::Text => rules.text,
        Kind::Image => rules.images,
        Kind::Files => rules.files,
    };
    way && content
}

fn digest_of(part: &ClipPart) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[u8::from(matches!(part.format, ClipFormat::Text))]);
    hasher.update(&part.bytes);
    *hasher.finalize().as_bytes()
}

/// What the interface handed over, or why it cannot go anywhere.
enum Copied {
    Part(ClipPart),
    Empty,
    TooLarge,
}

fn read_copied(
    format: &str,
    text: Option<String>,
    data: Option<String>,
) -> Result<Copied, AppError> {
    let text = text.map(Zeroizing::new);
    let data = data.map(Zeroizing::new);
    let (format, bytes) = match format {
        "text" => {
            let text = text.ok_or_else(|| AppError::new("bad_clip"))?;
            (ClipFormat::Text, Zeroizing::new(text.as_bytes().to_vec()))
        }
        "png" | "jpeg" => {
            let encoded = data.ok_or_else(|| AppError::new("bad_clip"))?;
            // A rough size first, so nothing enormous is decoded just to be refused.
            if encoded.len() / 4 * 3 > MAX_IMAGE + 3 {
                return Ok(Copied::TooLarge);
            }
            let bytes = BASE64
                .decode(encoded.as_bytes())
                .map_err(|_| AppError::new("bad_clip"))?;
            let kind = if format == "png" {
                ClipFormat::Png
            } else {
                ClipFormat::Jpeg
            };
            (kind, Zeroizing::new(bytes))
        }
        _ => return Err(AppError::new("bad_clip")),
    };
    if bytes.is_empty() {
        return Ok(Copied::Empty);
    }
    let kind = kind_of(format).ok_or_else(|| AppError::new("bad_clip"))?;
    if bytes.len() > limit_of(kind) {
        return Ok(Copied::TooLarge);
    }
    Ok(Copied::Part(ClipPart { format, bytes }))
}

const fn format_name(format: ClipFormat) -> &'static str {
    match format {
        ClipFormat::Text => "text",
        ClipFormat::Png => "png",
        ClipFormat::Jpeg => "jpeg",
        ClipFormat::Html => "html",
        ClipFormat::Rtf => "rtf",
        ClipFormat::Files => "files",
    }
}

fn state(name: &str) -> Reply {
    Ok(json!({ "state": name }))
}

impl Shared {
    pub(super) fn set_clipboard(&self, enabled: bool) -> Reply {
        {
            let mut inner = self.state();
            inner.settings.clipboard_enabled = enabled;
            inner.clip = ClipState::default();
        }
        self.save_settings();
        self.publish();
        Ok(Value::Null)
    }

    pub(super) fn set_clipboard_device(&self, device: &str, change: &DeviceChange) -> Reply {
        unhex::<32>(device)?;
        {
            let mut inner = self.state();
            let rules: &mut ClipDevice = inner
                .settings
                .clipboard_devices
                .entry(device.to_string())
                .or_default();
            rules.send = change.send.unwrap_or(rules.send);
            rules.receive = change.receive.unwrap_or(rules.receive);
            rules.text = change.text.unwrap_or(rules.text);
            rules.images = change.images.unwrap_or(rules.images);
            rules.files = change.files.unwrap_or(rules.files);
        }
        self.save_settings();
        self.publish();
        Ok(Value::Null)
    }

    /// Passes what was just copied here to the devices allowed to have it.
    pub(super) async fn clip_changed(
        self: &Arc<Self>,
        format: &str,
        text: Option<String>,
        data: Option<String>,
    ) -> Reply {
        let part = match read_copied(format, text, data)? {
            Copied::Part(part) => part,
            Copied::Empty => return state("empty"),
            Copied::TooLarge => return state("too_large"),
        };
        let kind = kind_of(part.format).ok_or_else(|| AppError::new("bad_clip"))?;
        if !self.state().settings.clipboard_enabled {
            return state("off");
        }
        let digest = digest_of(&part);
        // Nothing has been paired yet, so there is nobody to send to.
        let Ok(node) = self.node() else {
            return state("no_devices");
        };
        let nearby = self.compute_presence();
        let members = node
            .circle()
            .lock()
            .map_err(|_| AppError::new("state"))?
            .ledger()
            .membership()
            .members;
        let (targets, generation) = {
            let mut inner = self.state();
            // What arrived from another device is not news to pass along.
            if inner
                .clip
                .last_applied
                .is_some_and(|(seen, at)| seen == digest && at.elapsed() < LIFETIME)
            {
                return state("same");
            }
            let targets: Vec<DeviceId> = members
                .iter()
                .filter(|id| nearby.contains(*id))
                .filter(|id| allows(&inner.settings, &hex(*id), kind, Direction::Send))
                .copied()
                .collect();
            inner.clip.generation += 1;
            // A newer copy replaces any files offered before it.
            inner.clip.offered_files = None;
            (targets, inner.clip.generation)
        };
        if targets.is_empty() {
            return state("no_devices");
        }
        let clip =
            OutgoingClip::new(vec![part], LIFETIME).map_err(|_| AppError::new("bad_clip"))?;
        self.offer_clip(&node, targets, clip, generation).await
    }

    /// Offers a clip to each target at once, and reports what happened within
    /// a few seconds. Sending carries on after that.
    async fn offer_clip(
        self: &Arc<Self>,
        node: &Node,
        targets: Vec<DeviceId>,
        clip: OutgoingClip,
        generation: u64,
    ) -> Reply {
        let total = targets.len();
        let clip = Arc::new(clip);
        let delivered = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = targets
            .into_iter()
            .map(|peer| {
                let (this, node, clip) = (self.clone(), node.clone(), clip.clone());
                let delivered = delivered.clone();
                self.handle.spawn(async move {
                    if this.deliver_clip(&node, peer, &clip, generation).await {
                        delivered.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();
        drop(clip);
        let waiting = async {
            for task in tasks {
                let _ = task.await;
            }
        };
        let finished = tokio::time::timeout(REPORT_AFTER, waiting).await.is_ok();
        let delivered = delivered.load(Ordering::Relaxed);
        let name = match (delivered, finished) {
            (0, true) => "failed",
            (0, false) => "sending",
            _ => "sent",
        };
        Ok(json!({ "state": name, "targets": total, "delivered": delivered }))
    }

    /// Offers files just copied here. Only their list goes now: a device
    /// fetches the files themselves when the owner pastes them there.
    pub(super) async fn clip_files_changed(self: &Arc<Self>, paths: Vec<String>) -> Reply {
        if !self.state().settings.clipboard_enabled {
            return state("off");
        }
        if paths.is_empty() {
            return state("empty");
        }
        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        let planned = tokio::task::spawn_blocking(move || transfer::plan(&paths))
            .await
            .map_err(|_| AppError::new("state"))?;
        let outgoing = match planned {
            Ok(outgoing) => outgoing,
            Err(TransferError::BadOffer) => return state("too_large"),
            Err(_) => return state("empty"),
        };
        let list = ClipFiles {
            items: outgoing.items.iter().map(|i| i.item.clone()).collect(),
            total_bytes: outgoing.total_bytes,
        };
        let bytes = list.encode().map_err(|_| AppError::new("bad_clip"))?;
        if bytes.len() > MAX_FILE_LIST {
            return state("too_large");
        }
        let part = ClipPart {
            format: ClipFormat::Files,
            bytes: Zeroizing::new(bytes),
        };
        let Ok(node) = self.node() else {
            return state("no_devices");
        };
        let nearby = self.compute_presence();
        let members = node
            .circle()
            .lock()
            .map_err(|_| AppError::new("state"))?
            .ledger()
            .membership()
            .members;
        let clip =
            OutgoingClip::new(vec![part], LIFETIME).map_err(|_| AppError::new("bad_clip"))?;
        // The list goes to every device allowed to have it. A device that
        // cannot paste files leaves it behind, which costs only the list.
        let (targets, generation) = {
            let mut inner = self.state();
            let targets: Vec<DeviceId> = members
                .iter()
                .filter(|id| nearby.contains(*id))
                .filter(|id| allows(&inner.settings, &hex(*id), Kind::Files, Direction::Send))
                .copied()
                .collect();
            if targets.is_empty() {
                return state("no_devices");
            }
            inner.clip.generation += 1;
            inner.clip.offered_files = Some(OfferedFiles {
                clip_id: clip.id(),
                peers: targets.clone(),
                outgoing,
                until: Instant::now() + LIFETIME,
            });
            (targets, inner.clip.generation)
        };
        self.offer_clip(&node, targets, clip, generation).await
    }

    /// Fetches files copied on another device into a folder of their own,
    /// because the owner pasted them here, and returns where they are. The
    /// system's paste copies them on from there.
    pub(super) async fn paste_files(self: &Arc<Self>, id: &str) -> Reply {
        let clip_id = unhex::<16>(id)?;
        let (peer, manifest) = {
            let inner = self.state();
            let files = inner
                .clip
                .incoming_files
                .as_ref()
                .filter(|f| f.clip_id == clip_id && f.until > Instant::now())
                .ok_or_else(|| AppError::new("clip_gone"))?;
            if let Some(pasted) = &files.pasted {
                return Ok(json!({ "paths": pasted.paths, "folder": pasted.folder }));
            }
            (files.peer, files.manifest)
        };
        let node = self.node()?;
        let base = self.device_state_dir()?.join(PASTE_DIR);
        sweep_pastes(&base);
        let name = random_bytes::<8>().map_err(|_| AppError::new("state"))?;
        let folder_path = base.join(hex(&name));
        let folder = ReceiveFolder::open(&folder_path).map_err(|_| AppError::new("folder"))?;
        let cfg = ReceiveConfig {
            // The owner asked for these by pasting: nothing waits for a
            // second answer, and every other receive rule still applies.
            settings: ReceiveSettings::new(folder, ReceiveMode::Automatic),
            approver: Arc::new(Pasting),
            auto_limit: u64::MAX,
            budgets: Arc::new(Budgets::default()),
            clip: None,
        };
        let request = ClipPull { clip_id, item: 0 };
        let lock = self.peer_lock(peer);
        let Ok(_guard) = tokio::time::timeout(LIFETIME, lock.lock()).await else {
            let _ = std::fs::remove_dir_all(&folder_path);
            return Err(AppError::new("unreachable"));
        };
        // A paste of several files can ask for them all at once; the first
        // fetches, and the others find its result once they get the lock.
        if let Some(pasted) = self
            .state()
            .clip
            .incoming_files
            .as_ref()
            .filter(|f| f.clip_id == clip_id)
            .and_then(|f| f.pasted.clone())
        {
            let _ = std::fs::remove_dir_all(&folder_path);
            return Ok(json!({ "paths": pasted.paths, "folder": pasted.folder }));
        }
        for address in self.candidates(&peer) {
            match node
                .pull_clip(peer, address, &request, manifest, &cfg)
                .await
            {
                Ok(OfferOutcome::Received(_)) => {
                    self.remember_address(peer, address);
                    let pasted = Pasted {
                        folder: folder_path.to_string_lossy().into_owned(),
                        paths: pasted_paths(&folder_path),
                    };
                    if let Some(files) = self
                        .state()
                        .clip
                        .incoming_files
                        .as_mut()
                        .filter(|f| f.clip_id == clip_id)
                    {
                        files.pasted = Some(pasted.clone());
                    }
                    return Ok(json!({ "paths": pasted.paths, "folder": pasted.folder }));
                }
                Ok(OfferOutcome::Declined(_)) => {
                    let _ = std::fs::remove_dir_all(&folder_path);
                    return Err(AppError::new("clip_gone"));
                }
                Err(NodeError::Tls(_)) => {}
                Err(_) => break,
            }
        }
        let _ = std::fs::remove_dir_all(&folder_path);
        Err(AppError::new("unreachable"))
    }

    /// The files copied on another device, by the names a paste here will
    /// give them. A Mac shows them in Finder as placeholders before any of
    /// their contents has come.
    pub(super) fn clip_files_list(&self, id: &str) -> Reply {
        let clip_id = unhex::<16>(id)?;
        let items = self
            .state()
            .clip
            .incoming_files
            .as_ref()
            .filter(|f| f.clip_id == clip_id && f.until > Instant::now())
            .map(|f| Arc::clone(&f.items))
            .ok_or_else(|| AppError::new("clip_gone"))?;
        let list: Vec<Value> = items
            .iter()
            .map(|i| {
                json!({
                    "index": i.index,
                    "path": i.path,
                    "dir": i.kind == ItemKind::Directory,
                    "size": i.size,
                    "mtime_ms": i.mtime_ms,
                    "created_ms": i.created_ms,
                })
            })
            .collect();
        Ok(json!({ "items": list }))
    }

    /// Whether the owner can paste files here that have not arrived yet, the
    /// paste itself fetching them. Test builds allow it everywhere.
    fn can_paste_files_here(&self) -> bool {
        cfg!(any(windows, target_os = "macos"))
            || (cfg!(feature = "dev-loopback") && self.state().allow_loopback)
    }

    /// Takes the list of files copied on another device, and tells the
    /// interface a paste of them is possible. Nothing of the files moves yet.
    fn files_arrived(&self, peer: &DeviceId, clip_id: [u8; 16], part: &ClipPart, ttl: Duration) {
        let Ok(list) = ClipFiles::decode(&part.bytes) else {
            return;
        };
        let Ok(manifest) = transfer::manifest_of(&list.items) else {
            return;
        };
        // The same check a paste makes, done now: a list that could never
        // be pasted is not offered, and the names shown are the ones a paste
        // will write.
        let Ok(valid) = validate_manifest(&list.items, list.total_bytes, TargetOs::current())
        else {
            return;
        };
        let Some(node) = self.node().ok() else {
            return;
        };
        let files = valid.file_count;
        let folders = valid
            .items
            .iter()
            .filter(|i| i.kind == ItemKind::Directory && i.path.len() == 1)
            .count();
        self.state().clip.incoming_files = Some(IncomingFiles {
            clip_id,
            peer: *peer,
            manifest,
            items: Arc::new(valid.items),
            until: Instant::now() + ttl,
            pasted: None,
        });
        self.emit(&json!({
            "event": "clipboard_files",
            "id": hex(&clip_id),
            "files": files,
            "folders": folders,
            "total_bytes": list.total_bytes,
            "ttl_ms": u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX),
            "from": self.peer_display_name(&node, peer),
        }));
    }

    async fn deliver_clip(
        &self,
        node: &Node,
        peer: DeviceId,
        clip: &OutgoingClip,
        generation: u64,
    ) -> bool {
        // A second connection would replace one already carrying a transfer.
        let lock = self.peer_lock(peer);
        let Ok(guard) = tokio::time::timeout(LIFETIME, lock.lock()).await else {
            return false;
        };
        if self.state().clip.generation != generation {
            return false;
        }
        for address in self.candidates(&peer) {
            let sending = node.send_clip(peer, address, clip);
            match tokio::time::timeout(SEND_LIMIT, sending).await {
                Ok(Ok(())) => {
                    drop(guard);
                    self.remember_address(peer, address);
                    return true;
                }
                Ok(Err(NodeError::Tls(_))) => {}
                Ok(Err(_)) | Err(_) => return false,
            }
        }
        false
    }

    /// Hands a clip to the interface and forgets it.
    fn apply_clip(&self, peer: &DeviceId, part: &ClipPart, ttl: Duration) {
        let Some(node) = self.node().ok() else {
            return;
        };
        let text = match part.format {
            ClipFormat::Text => match std::str::from_utf8(&part.bytes) {
                Ok(text) => Some(text),
                Err(_) => return,
            },
            _ => None,
        };
        let data = text
            .is_none()
            .then(|| Zeroizing::new(BASE64.encode(&part.bytes)));
        self.state().clip.last_applied = Some((digest_of(part), Instant::now()));
        let event = ApplyEvent {
            event: "clipboard_apply",
            format: format_name(part.format),
            text,
            data: data.as_ref().map(|d| d.as_str()),
            ttl_ms: u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX),
            from: self.peer_display_name(&node, peer),
        };
        // One allocation that is large enough, so no shorter copy is left behind.
        let room = part.bytes.len() * 2 + 256;
        let mut out = Vec::with_capacity(room);
        if serde_json::to_writer(&mut out, &event).is_ok()
            && let Ok(message) = String::from_utf8(out)
        {
            (self.sink)(message);
        }
    }
}

#[derive(Serialize)]
struct ApplyEvent<'a> {
    event: &'static str,
    format: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<&'a str>,
    ttl_ms: u64,
    from: String,
}

/// Receives clips for the application.
pub(super) struct AppClip(pub Arc<Shared>);

impl ClipSink for AppClip {
    fn accepts(&self, peer: &DeviceId, format: ClipFormat, size: u64) -> bool {
        let Some(kind) = kind_of(format) else {
            return false;
        };
        if matches!(kind, Kind::Files) && !self.0.can_paste_files_here() {
            return false;
        }
        let device = hex(peer);
        size <= limit_of(kind) as u64
            && allows(&self.0.state().settings, &device, kind, Direction::Receive)
    }

    fn deliver(&self, peer: &DeviceId, clip: ReceivedClip) {
        if let Some(part) = clip.parts.iter().find(|p| p.format == ClipFormat::Files) {
            self.0.files_arrived(peer, clip.clip_id, part, clip.ttl);
            return;
        }
        let text = clip.parts.iter().find(|p| p.format == ClipFormat::Text);
        if let Some(part) = text.or_else(|| clip.parts.first()) {
            self.0.apply_clip(peer, part, clip.ttl);
        }
    }

    fn files_for(
        &self,
        peer: &DeviceId,
        clip_id: &[u8; 16],
        item: u32,
    ) -> Option<(Outgoing, PathBuf)> {
        if item != 0 {
            return None;
        }
        let inner = self.0.state();
        let offered = inner.clip.offered_files.as_ref()?;
        let alive = offered.until > Instant::now();
        if !alive || offered.clip_id != *clip_id || !offered.peers.contains(peer) {
            return None;
        }
        let dir = inner.device.as_ref()?.dir.clone();
        Some((offered.outgoing.clone(), dir))
    }
}

/// Accepts what a paste asked for. Asked only when a peer's limits would
/// otherwise call for the owner, who has just answered by pasting.
struct Pasting;

impl Approver for Pasting {
    fn approve(&self, _: OfferSummary) -> Pin<Box<dyn Future<Output = bool> + Send>> {
        Box::pin(async { true })
    }
}

/// Removes pastes that are surely done copying. A newer one may still be
/// in the middle of the system's copy, so it stays.
fn sweep_pastes(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age > PASTE_KEEP);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Removes every paste left from an earlier run.
pub(super) fn clear_pastes(device_dir: &Path) {
    let _ = std::fs::remove_dir_all(device_dir.join(PASTE_DIR));
}

/// The top-level files and folders a paste brought, leaving out the
/// receive machinery's own working names.
fn pasted_paths(folder: &Path) -> Vec<String> {
    let mut paths: Vec<String> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with(".zerohop"))
        .filter_map(|e| e.path().to_str().map(str::to_string))
        .collect();
    paths.sort();
    paths
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    const DEVICE: &str = "aa";

    fn enabled() -> Settings {
        Settings {
            clipboard_enabled: true,
            ..Settings::default()
        }
    }

    #[test]
    fn nothing_moves_by_default() {
        let settings = Settings::default();
        assert!(!allows(&settings, DEVICE, Kind::Text, Direction::Send));
        assert!(!allows(&settings, DEVICE, Kind::Text, Direction::Receive));
    }

    #[test]
    fn turned_on_it_covers_text_and_pictures_both_ways() {
        let settings = enabled();
        for kind in [Kind::Text, Kind::Image] {
            assert!(allows(&settings, DEVICE, kind, Direction::Send));
            assert!(allows(&settings, DEVICE, kind, Direction::Receive));
        }
    }

    #[test]
    fn a_device_waiting_for_approval_is_left_out() {
        let mut settings = enabled();
        settings.unconfirmed.insert(DEVICE.to_string());
        assert!(!allows(&settings, DEVICE, Kind::Text, Direction::Send));
        assert!(!allows(&settings, DEVICE, Kind::Text, Direction::Receive));
    }

    #[test]
    fn each_rule_closes_only_what_it_names() {
        let mut settings = enabled();
        settings.clipboard_devices.insert(
            DEVICE.to_string(),
            ClipDevice {
                send: false,
                images: false,
                ..ClipDevice::default()
            },
        );
        assert!(!allows(&settings, DEVICE, Kind::Text, Direction::Send));
        assert!(allows(&settings, DEVICE, Kind::Text, Direction::Receive));
        assert!(!allows(&settings, DEVICE, Kind::Image, Direction::Receive));
        assert!(allows(&settings, "bb", Kind::Image, Direction::Send));
    }

    #[test]
    fn rich_text_is_not_taken() {
        for format in [ClipFormat::Html, ClipFormat::Rtf] {
            assert!(kind_of(format).is_none());
        }
    }

    #[test]
    fn files_have_a_rule_of_their_own() {
        let mut settings = enabled();
        assert!(allows(&settings, DEVICE, Kind::Files, Direction::Receive));
        settings.clipboard_devices.insert(
            DEVICE.to_string(),
            ClipDevice {
                files: false,
                ..ClipDevice::default()
            },
        );
        assert!(!allows(&settings, DEVICE, Kind::Files, Direction::Receive));
        assert!(allows(&settings, DEVICE, Kind::Text, Direction::Receive));
    }

    #[test]
    fn the_same_bytes_in_another_kind_are_different_content() {
        let part = |format| ClipPart {
            format,
            bytes: Zeroizing::new(b"abc".to_vec()),
        };
        assert_ne!(
            digest_of(&part(ClipFormat::Text)),
            digest_of(&part(ClipFormat::Png))
        );
    }
}
