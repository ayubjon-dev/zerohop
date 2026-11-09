//! The shared clipboard: what the owner allows, and moving copied content.
//!
//! Copied content is only ever in memory. It is never written to the settings,
//! the history, a snapshot, or a log, and this file keeps no copy of it.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use data_encoding::BASE64;
use serde::Serialize;
use serde_json::{Value, json};
use zerohop_core::clip::{ClipPart, ClipSink, MAX_CLIP_TTL, OutgoingClip, ReceivedClip};
use zerohop_core::ledger::DeviceId;
use zerohop_core::node::{Node, NodeError};
use zerohop_core::wire::ClipFormat;
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

/// What only this device remembers about the clipboard. Holds no content.
#[derive(Default)]
pub(super) struct ClipState {
    last_applied: Option<([u8; 32], Instant)>,
    generation: u64,
}

/// A change to one device's clipboard rules. Unset fields stay as they are.
pub(super) struct DeviceChange {
    pub send: Option<bool>,
    pub receive: Option<bool>,
    pub text: Option<bool>,
    pub images: Option<bool>,
}

#[derive(Clone, Copy)]
enum Kind {
    Text,
    Image,
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
        ClipFormat::Html | ClipFormat::Rtf | ClipFormat::Files => None,
    }
}

const fn limit_of(kind: Kind) -> usize {
    match kind {
        Kind::Text => MAX_TEXT,
        Kind::Image => MAX_IMAGE,
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
            (targets, inner.clip.generation)
        };
        if targets.is_empty() {
            return state("no_devices");
        }
        let total = targets.len();
        let clip = Arc::new(
            OutgoingClip::new(vec![part], LIFETIME).map_err(|_| AppError::new("bad_clip"))?,
        );
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
        let device = hex(peer);
        size <= limit_of(kind) as u64
            && allows(&self.0.state().settings, &device, kind, Direction::Receive)
    }

    fn deliver(&self, peer: &DeviceId, clip: ReceivedClip) {
        let text = clip.parts.iter().find(|p| p.format == ClipFormat::Text);
        if let Some(part) = text.or_else(|| clip.parts.first()) {
            self.0.apply_clip(peer, part, clip.ttl);
        }
    }
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
    fn files_and_rich_text_are_not_taken() {
        for format in [ClipFormat::Files, ClipFormat::Html, ClipFormat::Rtf] {
            assert!(kind_of(format).is_none());
        }
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
