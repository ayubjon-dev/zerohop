//! Recognizing devices on the network (protocol section 6).
//!
//! An announcement is only a hint about which address to try. Who is really
//! there is decided by the TLS handshake against a pinned key.

use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::circle::{CircleError, random_bytes};
use crate::ledger::DeviceId;

/// The service devices announce while they can receive.
pub const SERVICE_TYPE: &str = "_zerohop._tcp";
/// The service announced only while pairing is open.
pub const PAIRING_SERVICE_TYPE: &str = "_zerohop-pair._tcp";

const EPOCH_SECONDS: u64 = 600;
const TAG_LABEL: &[u8] = b"zerohop-v1/tag\0";
const TAG_BYTES: usize = 8;

/// Which ten-minute period a moment falls in.
#[must_use]
pub fn epoch_at(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / EPOCH_SECONDS)
}

/// The tag a device with this key announces during an epoch.
#[must_use]
pub fn tag(key: &[u8; 32], epoch: u64) -> String {
    let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(key) else {
        return String::new();
    };
    mac.update(TAG_LABEL);
    mac.update(&epoch.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    data_encoding::HEXLOWER.encode(digest.get(..TAG_BYTES).unwrap_or_default())
}

/// Finds whose announcement this is, allowing for clocks a little apart.
pub fn recognize<'a>(
    keys: impl IntoIterator<Item = (&'a DeviceId, &'a [u8; 32])>,
    announced: &str,
    epoch: u64,
) -> Option<DeviceId> {
    let epochs = [epoch.saturating_sub(1), epoch, epoch.saturating_add(1)];
    keys.into_iter()
        .find(|(_, key)| epochs.iter().any(|e| tag(key, *e) == announced))
        .map(|(id, _)| *id)
}

/// A fresh instance name, which says nothing about the device.
///
/// # Errors
/// Returns an error if no secure randomness is available.
pub fn new_instance_name() -> Result<String, CircleError> {
    Ok(data_encoding::HEXLOWER.encode(&random_bytes::<8>()?))
}

/// A fresh room label for pairing.
///
/// # Errors
/// Returns an error if no secure randomness is available.
pub fn new_room_label() -> Result<String, CircleError> {
    Ok(data_encoding::HEXUPPER.encode(&random_bytes::<2>()?))
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_changes_with_the_epoch_and_the_key() {
        let key = [7u8; 32];
        assert_eq!(tag(&key, 5).len(), 16);
        assert_ne!(tag(&key, 5), tag(&key, 6));
        assert_ne!(tag(&key, 5), tag(&[8u8; 32], 5));
    }

    #[test]
    fn a_device_is_recognized_across_a_small_clock_difference() {
        let (id, key) = ([1u8; 32], [7u8; 32]);
        let keys = [(&id, &key)];
        assert_eq!(recognize(keys, &tag(&key, 100), 99), Some(id));
        assert_eq!(recognize(keys, &tag(&key, 100), 100), Some(id));
        assert_eq!(recognize(keys, &tag(&key, 100), 101), Some(id));
    }

    #[test]
    fn an_old_or_foreign_tag_is_not_recognized() {
        let (id, key) = ([1u8; 32], [7u8; 32]);
        let keys = [(&id, &key)];
        assert_eq!(recognize(keys, &tag(&key, 100), 102), None);
        assert_eq!(recognize(keys, &tag(&[9u8; 32], 100), 100), None);
        assert_eq!(recognize(keys, "not a tag", 100), None);
    }

    #[test]
    fn the_tag_matches_the_definition_in_the_protocol() {
        let key = [0u8; 32];
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key).unwrap();
        mac.update(b"zerohop-v1/tag\0");
        mac.update(&42u64.to_be_bytes());
        let digest = mac.finalize().into_bytes();
        assert_eq!(tag(&key, 42), data_encoding::HEXLOWER.encode(&digest[..8]));
    }
}
