//! C interface between the Zerohop core and the Flutter app.
//!
//! Everything crosses this boundary as JSON text. Nothing here interprets it,
//! and every function stops panics from unwinding into the caller.

use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use zerohop_app::{Config, Engine, KeySealing};
use zeroize::Zeroizing;

#[cfg(windows)]
mod windows_secret;

/// Receives each message. The receiver owns it and returns it with [`zh_free`].
pub type Callback = extern "C" fn(*mut c_char);

const SECRET_LEN: usize = 32;

static ENGINE: OnceLock<Engine> = OnceLock::new();
static CALLBACK: Mutex<Option<Callback>> = Mutex::new(None);

fn deliver(message: String) {
    let callback = *CALLBACK.lock().unwrap_or_else(PoisonError::into_inner);
    if let (Some(callback), Ok(text)) = (callback, CString::new(message)) {
        callback(text.into_raw());
    }
}

/// # Safety
/// `text` must be null or point to a valid NUL-terminated string.
unsafe fn read_text(text: *const c_char) -> Option<Zeroizing<String>> {
    if text.is_null() {
        return None;
    }
    // SAFETY: not null, and the caller guarantees NUL termination.
    let text = unsafe { CStr::from_ptr(text) };
    text.to_str()
        .ok()
        .map(|text| Zeroizing::new(text.to_string()))
}

/// Starts the engine, or points a running one at a new callback.
/// Returns 0 on success.
///
/// `secret` is what seals the device key on disk: 32 bytes from the system's
/// key store, or null to leave the key in a file only its owner can read.
///
/// # Safety
/// Both paths must be valid NUL-terminated strings. `secret` must be null or
/// point to `secret_len` readable bytes. `callback` must stay callable from
/// any thread until the next call to this function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_start(
    state_dir: *const c_char,
    default_folder: *const c_char,
    secret: *const u8,
    secret_len: usize,
    callback: Callback,
) -> i32 {
    let sealing = if secret.is_null() {
        KeySealing::default()
    } else {
        if secret_len != SECRET_LEN {
            return 4;
        }
        let mut bytes = Zeroizing::new([0u8; SECRET_LEN]);
        // SAFETY: not null, and the caller guarantees `secret_len` readable bytes.
        bytes.copy_from_slice(unsafe { std::slice::from_raw_parts(secret, SECRET_LEN) });
        KeySealing::with_secret(bytes)
    };
    // SAFETY: the caller guarantees both strings are valid.
    let paths = unsafe { (read_text(state_dir), read_text(default_folder)) };
    let (Some(state_dir), Some(default_folder)) = paths else {
        return 1;
    };
    let started = catch_unwind(AssertUnwindSafe(|| {
        *CALLBACK.lock().unwrap_or_else(PoisonError::into_inner) = Some(callback);
        if ENGINE.get().is_some() {
            return 0;
        }
        let config = Config {
            state_dir: PathBuf::from(state_dir.as_str()),
            default_folder: PathBuf::from(default_folder.as_str()),
            sealing,
        };
        match Engine::start(config, Arc::new(deliver)) {
            Ok(engine) => i32::from(ENGINE.set(engine).is_err()),
            Err(_) => 2,
        }
    }));
    started.unwrap_or(3)
}

/// Writes the secret that seals the device key for this Windows account into
/// `out`, making it on first use and keeping it in `state_dir`.
///
/// Returns 0 on success, 1 for an unreadable path, 2 when Windows will not
/// open or store the secret, 3 after a panic, 4 for a wrong buffer, and 5 on
/// systems whose interface brings its own key store.
///
/// # Safety
/// `state_dir` must be a valid NUL-terminated string. `out` must be null or
/// point to `out_len` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_system_secret(
    state_dir: *const c_char,
    out: *mut u8,
    out_len: usize,
) -> i32 {
    #[cfg(windows)]
    {
        if out.is_null() || out_len != SECRET_LEN {
            return 4;
        }
        // SAFETY: the caller guarantees the string is valid.
        let Some(state_dir) = (unsafe { read_text(state_dir) }) else {
            return 1;
        };
        let found = catch_unwind(AssertUnwindSafe(|| {
            windows_secret::load_or_create(std::path::Path::new(state_dir.as_str()))
        }));
        match found {
            Ok(Ok(secret)) => {
                // SAFETY: not null, and the caller guarantees `SECRET_LEN`
                // writable bytes, which cannot overlap the secret.
                unsafe { std::ptr::copy_nonoverlapping(secret.as_ptr(), out, SECRET_LEN) };
                0
            }
            Ok(Err(_)) => 2,
            Err(_) => 3,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (state_dir, out, out_len);
        5
    }
}

/// Hands a request to the engine. The answer arrives through the callback.
///
/// # Safety
/// `request` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_request(request: *const c_char) {
    // SAFETY: the caller guarantees the string is valid.
    let Some(request) = (unsafe { read_text(request) }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(engine) = ENGINE.get() {
            engine.request(&request);
        }
    }));
}

/// Handles a request on the calling thread and returns the answer, which the
/// caller frees with [`zh_free`]. For a caller that has to wait for the
/// answer, such as the system asking for files the owner pasted. Null if the
/// engine is not running or the request cannot be read.
///
/// # Safety
/// `request` must be a valid NUL-terminated string. The calling thread must
/// not be one of the engine's own.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_call(request: *const c_char) -> *mut c_char {
    // SAFETY: the caller guarantees the string is valid.
    let Some(request) = (unsafe { read_text(request) }) else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        let answer = ENGINE.get()?.request_now(&request);
        if answer.is_empty() {
            return None;
        }
        CString::new(answer).ok().map(CString::into_raw)
    }))
    .ok()
    .flatten()
    .unwrap_or(std::ptr::null_mut())
}

/// Frees a message delivered through the callback or returned by [`zh_call`].
///
/// # Safety
/// `message` must have come from the callback and not have been freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_free(message: *mut c_char) {
    if !message.is_null() {
        // SAFETY: the pointer came from `CString::into_raw` in `deliver`.
        let message = unsafe { CString::from_raw(message) };
        // A message may carry copied content.
        drop(Zeroizing::new(message.into_bytes_with_nul()));
    }
}
