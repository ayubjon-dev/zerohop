//! Settings and history kept on this device.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Most transfers remembered.
pub const MAX_HISTORY: usize = 200;

/// What the owner has chosen.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// This device's name, used when it first joins or starts a circle.
    pub name: Option<String>,
    /// Which directory under the state folder holds the current circle.
    pub circle_dir: String,
    /// Where received files go. `None` means the default folder.
    pub folder: Option<String>,
    /// How devices without their own mode are treated.
    pub default_mode: String,
    /// Modes chosen for single devices, by hexadecimal ID.
    pub device_modes: BTreeMap<String, String>,
    /// Whether this device announces itself on the network.
    pub visible: bool,
    /// Whether a device added through another device waits for approval here.
    pub confirm_new: bool,
    /// Devices this one already knows about, by hexadecimal ID.
    pub known: BTreeSet<String>,
    /// Devices waiting for the owner's approval, by hexadecimal ID.
    pub unconfirmed: BTreeSet<String>,
    /// Platforms learned while pairing, by hexadecimal ID.
    pub platforms: BTreeMap<String, String>,
    /// Whether copied content moves between devices at all.
    pub clipboard_enabled: bool,
    /// Clipboard rules chosen for single devices, by hexadecimal ID.
    pub clipboard_devices: BTreeMap<String, ClipDevice>,
    /// Preferences only the interface reads.
    pub ui: serde_json::Value,
}

/// How the shared clipboard treats one device, once it is turned on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipDevice {
    /// Copies made here may go to it.
    pub send: bool,
    /// Copies made there may arrive here.
    pub receive: bool,
    /// Text and links.
    pub text: bool,
    /// Pictures.
    pub images: bool,
    /// Copied files, which move only when the owner pastes them.
    pub files: bool,
}

impl Default for ClipDevice {
    fn default() -> Self {
        Self {
            send: true,
            receive: true,
            text: true,
            images: true,
            files: true,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: None,
            circle_dir: "device".to_string(),
            folder: None,
            default_mode: "smart".to_string(),
            device_modes: BTreeMap::new(),
            visible: true,
            confirm_new: false,
            known: BTreeSet::new(),
            unconfirmed: BTreeSet::new(),
            platforms: BTreeMap::new(),
            clipboard_enabled: false,
            clipboard_devices: BTreeMap::new(),
            ui: serde_json::Value::Object(serde_json::Map::new()),
        }
    }
}

/// One finished transfer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// The transfer's identifier.
    pub id: String,
    /// When it ended, in milliseconds since 1970.
    pub at_ms: i64,
    /// `sent` or `received`.
    pub direction: String,
    /// The other device's name at the time.
    pub peer_name: String,
    /// Number of files.
    pub files: u32,
    /// Total size.
    pub bytes: u64,
    /// Name of the first item.
    pub name: String,
    /// How it ended.
    pub outcome: String,
    /// Items held back for the owner.
    pub held: u32,
}

/// Reads a JSON file, treating a missing or damaged one as empty.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> T {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Writes a JSON file so that a crash leaves either the old one or the new one.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub fn save<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = Settings {
            name: Some("Mac".to_string()),
            visible: false,
            ..Settings::default()
        };
        save(&path, &settings).unwrap();
        let loaded: Settings = load(&path);
        assert_eq!(loaded.name.as_deref(), Some("Mac"));
        assert!(!loaded.visible);
    }

    #[test]
    fn a_damaged_file_counts_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, b"{ not json").unwrap();
        let loaded: Settings = load(&path);
        assert_eq!(loaded.default_mode, "smart");
    }

    #[test]
    fn a_missing_field_takes_its_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, br#"{"name":"Mac"}"#).unwrap();
        let loaded: Settings = load(&path);
        assert_eq!(loaded.name.as_deref(), Some("Mac"));
        assert!(loaded.visible);
    }
}
