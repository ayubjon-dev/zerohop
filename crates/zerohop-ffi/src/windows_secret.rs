//! The secret that seals the device key on Windows.
//!
//! Windows has no per-app key store like the macOS keychain or the Android
//! Keystore. Its data protection API ties a blob to the signed-in account
//! instead: another account, or someone holding only the disk, cannot open it.
//! The secret is made once, protected that way, and kept next to the state it
//! protects. Programs running as the same account can still ask Windows to
//! open it, as they can ask the keychain on a Mac.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use zeroize::{Zeroize, Zeroizing};

use crate::SECRET_LEN;

const FILE: &str = "device-secret.dpapi";

/// Keeps this blob apart from those other programs protect for the account.
const ENTROPY: &[u8] = b"dev.zerohop device key secret v1";

/// Returns the secret kept in `state_dir`, making it on first use.
///
/// A secret that exists but cannot be opened is reported, never replaced:
/// replacing it would lock the device key away for good.
pub(crate) fn load_or_create(state_dir: &Path) -> io::Result<Zeroizing<[u8; SECRET_LEN]>> {
    let path = state_dir.join(FILE);
    match fs::read(&path) {
        Ok(blob) => return unprotect(&blob),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    fs::create_dir_all(state_dir)?;
    let secret = Zeroizing::new(
        zerohop_app::random_bytes::<SECRET_LEN>()
            .map_err(|_| io::Error::other("no secure randomness"))?,
    );
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(&protect(&*secret)?)?;
    file.sync_all()?;
    drop(file);
    // A link never replaces a file, so a secret already in use is never lost,
    // and the name only ever points at a complete blob.
    let linked = fs::hard_link(&tmp, &path);
    let _ = fs::remove_file(&tmp);
    match linked {
        Ok(()) => Ok(secret),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => unprotect(&fs::read(&path)?),
        Err(e) => Err(e),
    }
}

fn blob_of(bytes: &[u8]) -> io::Result<CRYPT_INTEGER_BLOB> {
    Ok(CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).map_err(|_| io::ErrorKind::InvalidInput)?,
        // Windows only reads input blobs.
        pbData: bytes.as_ptr().cast_mut(),
    })
}

fn protect(secret: &[u8]) -> io::Result<Vec<u8>> {
    let input = blob_of(secret)?;
    let entropy = blob_of(ENTROPY)?;
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: every pointer refers to a live blob for the whole call, the
    // optional arguments are null, and `output` is written by Windows alone.
    let ok = unsafe {
        CryptProtectData(
            &input,
            ptr::null(),
            &entropy,
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let sealed = Allocated {
        blob: output,
        wipe: false,
    };
    Ok(sealed.bytes().to_vec())
}

fn unprotect(blob: &[u8]) -> io::Result<Zeroizing<[u8; SECRET_LEN]>> {
    let input = blob_of(blob)?;
    let entropy = blob_of(ENTROPY)?;
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: as in `protect`; the description is not asked for.
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            &entropy,
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let opened = Allocated {
        blob: output,
        wipe: true,
    };
    let mut secret = Zeroizing::new([0u8; SECRET_LEN]);
    let bytes = opened.bytes();
    if bytes.len() != SECRET_LEN {
        return Err(io::ErrorKind::InvalidData.into());
    }
    secret.copy_from_slice(bytes);
    Ok(secret)
}

/// Memory Windows handed back, freed when this is dropped.
struct Allocated {
    blob: CRYPT_INTEGER_BLOB,
    /// Whether the memory holds the secret itself.
    wipe: bool,
}

impl Allocated {
    fn len(&self) -> usize {
        usize::try_from(self.blob.cbData).unwrap_or(0)
    }

    fn bytes(&self) -> &[u8] {
        if self.blob.pbData.is_null() {
            return &[];
        }
        // SAFETY: Windows put `cbData` bytes at `pbData`, kept until `drop`.
        unsafe { std::slice::from_raw_parts(self.blob.pbData, self.len()) }
    }
}

impl Drop for Allocated {
    fn drop(&mut self) {
        if self.blob.pbData.is_null() {
            return;
        }
        if self.wipe {
            // SAFETY: as in `bytes`, and nothing else refers to this memory.
            unsafe { std::slice::from_raw_parts_mut(self.blob.pbData, self.len()) }.zeroize();
        }
        // SAFETY: the data protection API allocated this with `LocalAlloc`,
        // and it is freed only here.
        unsafe { LocalFree(self.blob.pbData.cast()) };
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_same_secret_comes_back_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let first = load_or_create(dir.path()).unwrap();
        let second = load_or_create(dir.path()).unwrap();
        assert_eq!(*first, *second);
    }

    #[test]
    fn the_secret_is_not_kept_in_the_clear() {
        let dir = tempfile::tempdir().unwrap();
        let secret = load_or_create(dir.path()).unwrap();
        let stored = fs::read(dir.path().join(FILE)).unwrap();
        assert!(!stored.windows(SECRET_LEN).any(|part| part == &secret[..]));
        assert!(!dir.path().join(FILE).with_extension("tmp").exists());
    }

    #[test]
    fn each_state_folder_gets_its_own_secret() {
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        let a = load_or_create(one.path()).unwrap();
        let b = load_or_create(two.path()).unwrap();
        assert_ne!(*a, *b);
    }

    #[test]
    fn a_damaged_secret_is_reported_and_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        fs::write(&path, b"not a protected blob").unwrap();
        assert!(load_or_create(dir.path()).is_err());
        let kept = fs::read(&path).unwrap();
        assert_eq!(kept, b"not a protected blob");
    }

    #[test]
    fn a_blob_protected_for_another_purpose_does_not_open() {
        let dir = tempfile::tempdir().unwrap();
        let input = blob_of(&[7u8; SECRET_LEN]).unwrap();
        let mut output = CRYPT_INTEGER_BLOB::default();
        // SAFETY: as in `protect`, without entropy.
        let ok = unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        assert_ne!(ok, 0);
        let other = Allocated {
            blob: output,
            wipe: false,
        };
        fs::write(dir.path().join(FILE), other.bytes()).unwrap();
        assert!(load_or_create(dir.path()).is_err());
    }
}
