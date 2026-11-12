//! For a native program that runs the core while the Flutter interface is not
//! there, such as Zerohop on Windows with its window closed.
//!
//! It reads the core's messages and writes its requests here, so that it needs
//! no JSON of its own: the messages carry text and pictures copied on other
//! devices, which are safer parsed in Rust than by hand in C++. Requests from
//! here carry the number 0, and their answers are of no interest to the
//! interface.

use std::ffi::{CStr, CString, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

use data_encoding::BASE64;
use serde::Serialize;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::{ENGINE, read_text};

/// What a message is about, for the program that runs the core. Each function
/// may be absent, and every pointer it receives lasts only for the call.
#[repr(C)]
pub struct HostCalls {
    /// Passed back as the first argument of every call.
    pub context: *mut c_void,
    /// The newest snapshot: whether the shared clipboard is on, whether a
    /// pairing is under way, and what to announce on the network: an instance
    /// name, its tag and its port, or a null instance for nothing.
    pub snapshot: Option<extern "C" fn(*mut c_void, bool, bool, *const c_char, *const c_char, u16)>,
    /// Someone is waiting for the owner to accept or decline files.
    pub offer_arrived: Option<extern "C" fn(*mut c_void)>,
    /// Text copied on another device, to put on the clipboard for the given
    /// milliseconds, and the name of that device.
    pub clip_text: Option<extern "C" fn(*mut c_void, *const c_char, u64, *const c_char)>,
    /// A picture copied on another device, as PNG or JPEG bytes and their
    /// length, then the milliseconds and the name of that device.
    pub clip_picture: Option<extern "C" fn(*mut c_void, *const u8, usize, u64, *const c_char)>,
    /// Files copied on another device: the core's name for them, then the
    /// milliseconds and the name of that device. Only their list has come.
    pub clip_files: Option<extern "C" fn(*mut c_void, *const c_char, u64, *const c_char)>,
}

/// What [`zh_host_read`] found.
const FOR_INTERFACE: i32 = 0;
const OWN_ANSWER: i32 = 1;
const UNREADABLE: i32 = 2;

/// The number this side's requests carry.
const OWN_ID: u64 = 0;

/// Reads one message from the core and makes the call it is about. Returns 0
/// when the message is also for the interface, 1 for the answer to one of this
/// side's own requests, which goes no further, and 2 when it cannot be read.
///
/// # Safety
/// `message` must be a valid NUL-terminated string, and `calls` must point to
/// a valid [`HostCalls`] whose functions can be called on this thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_host_read(message: *const c_char, calls: *const HostCalls) -> i32 {
    // SAFETY: the caller guarantees the string is valid.
    let Some(text) = (unsafe { read_text(message) }) else {
        return UNREADABLE;
    };
    if calls.is_null() {
        return UNREADABLE;
    }
    // SAFETY: not null, and the caller guarantees it is valid.
    let calls = unsafe { &*calls };
    catch_unwind(AssertUnwindSafe(|| read(&text, calls))).unwrap_or(UNREADABLE)
}

fn read(text: &str, calls: &HostCalls) -> i32 {
    let Ok(Value::Object(message)) = serde_json::from_str::<Value>(text) else {
        return UNREADABLE;
    };
    if let Some(reply) = message.get("reply") {
        return if reply.as_u64() == Some(OWN_ID) {
            OWN_ANSWER
        } else {
            FOR_INTERFACE
        };
    }
    let context = calls.context;
    let event = message.get("event").and_then(Value::as_str).unwrap_or("");
    let ttl = message.get("ttl_ms").and_then(Value::as_u64).unwrap_or(0);
    let from = c_text(message.get("from"));
    let from = from.as_ref().map_or(std::ptr::null(), |f| f.as_ptr());
    match event {
        "snapshot" => {
            if let Some(call) = calls.snapshot {
                let on = message
                    .get("settings")
                    .and_then(|s| s.get("clipboard"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let pairing = message.get("pairing").is_some_and(Value::is_object);
                let advert = message.get("advert").filter(|a| a.is_object());
                let instance = advert.and_then(|a| c_text(a.get("instance")));
                let tag = advert.and_then(|a| c_text(a.get("tag")));
                let port = advert
                    .and_then(|a| a.get("port"))
                    .and_then(Value::as_u64)
                    .and_then(|p| u16::try_from(p).ok());
                match (instance, tag, port) {
                    (Some(instance), Some(tag), Some(port)) => {
                        call(context, on, pairing, instance.as_ptr(), tag.as_ptr(), port);
                    }
                    _ => call(context, on, pairing, std::ptr::null(), std::ptr::null(), 0),
                }
            }
        }
        "offer_arrived" => {
            if let Some(call) = calls.offer_arrived {
                call(context);
            }
        }
        "clipboard_apply" => match message.get("format").and_then(Value::as_str) {
            Some("text") => {
                if let (Some(call), Some(clip)) =
                    (calls.clip_text, secret_text(message.get("text")))
                {
                    call(context, clip.as_ptr().cast(), ttl, from);
                }
            }
            Some("png" | "jpeg") => {
                let data = message.get("data").and_then(Value::as_str);
                let bytes = data
                    .and_then(|d| BASE64.decode(d.as_bytes()).ok())
                    .map(Zeroizing::new);
                if let (Some(call), Some(bytes)) = (calls.clip_picture, bytes)
                    && !bytes.is_empty()
                {
                    call(context, bytes.as_ptr(), bytes.len(), ttl, from);
                }
            }
            _ => {}
        },
        "clipboard_files" => {
            if let (Some(call), Some(id)) = (calls.clip_files, c_text(message.get("id"))) {
                call(context, id.as_ptr(), ttl, from);
            }
        }
        _ => {}
    }
    FOR_INTERFACE
}

/// A string from a message as C text. None when it is missing or holds NUL.
fn c_text(value: Option<&Value>) -> Option<CString> {
    CString::new(value?.as_str()?).ok()
}

/// Copied text as C text, wiped when dropped.
fn secret_text(value: Option<&Value>) -> Option<Zeroizing<Vec<u8>>> {
    let text = value?.as_str()?;
    if text.is_empty() || text.contains('\0') {
        return None;
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(text.len() + 1));
    bytes.extend_from_slice(text.as_bytes());
    bytes.push(0);
    Some(bytes)
}

// ---- Requests ----

#[derive(Serialize)]
struct Request<'a, P> {
    id: u64,
    method: &'a str,
    params: P,
}

/// Hands a request to the core without waiting. `room` is enough for all of
/// it, so that no shorter copy of copied content is left behind as it grows.
fn send<P: Serialize>(method: &str, params: P, room: usize) {
    let Some(engine) = ENGINE.get() else {
        return;
    };
    let mut out = Zeroizing::new(Vec::with_capacity(room));
    let request = Request {
        id: OWN_ID,
        method,
        params,
    };
    if serde_json::to_writer(&mut *out, &request).is_ok()
        && let Ok(text) = std::str::from_utf8(&out)
    {
        engine.request(text);
    }
}

/// Room for `content` bytes written as JSON, where one byte can take six.
fn room_for(content: usize) -> usize {
    content.saturating_mul(6).saturating_add(256)
}

/// Text copied on this computer, for the other devices.
///
/// # Safety
/// `text` must be a valid NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_host_clip_text(text: *const c_char) {
    // SAFETY: the caller guarantees the string is valid.
    let Some(text) = (unsafe { read_text(text) }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        #[derive(Serialize)]
        struct Clip<'a> {
            format: &'a str,
            text: &'a str,
        }
        let clip = Clip {
            format: "text",
            text: &text,
        };
        send("clip_changed", clip, room_for(text.len()));
    }));
}

/// A picture copied on this computer, as PNG, for the other devices.
///
/// # Safety
/// `png` must point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_host_clip_png(png: *const u8, len: usize) {
    if png.is_null() || len == 0 {
        return;
    }
    // SAFETY: not null, and the caller guarantees `len` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(png, len) };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        #[derive(Serialize)]
        struct Clip<'a> {
            format: &'a str,
            data: &'a str,
        }
        let mut data = Zeroizing::new(String::with_capacity(BASE64.encode_len(len)));
        BASE64.encode_append(bytes, &mut data);
        let clip = Clip {
            format: "png",
            data: &data,
        };
        send("clip_changed", clip, data.len().saturating_add(256));
    }));
}

/// Files and folders copied on this computer, as full paths. Only the list
/// goes now: the files move when the owner pastes them elsewhere.
///
/// # Safety
/// `paths` must point to `count` valid NUL-terminated UTF-8 strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_host_clip_files(paths: *const *const c_char, count: usize) {
    // SAFETY: the caller guarantees `count` valid strings.
    let Some(paths) = (unsafe { texts(paths, count) }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        #[derive(Serialize)]
        struct Clip<'a> {
            paths: &'a [String],
        }
        let room = room_for(paths.iter().map(String::len).sum());
        send("clip_files", Clip { paths: &paths }, room);
    }));
}

/// A device seen on the network: its instance name, its addresses, its port
/// and its tag, which may be null. Only a hint about where to connect: the
/// core decides who is really there.
///
/// # Safety
/// `instance` must be a valid NUL-terminated string, `addresses` must point to
/// `count` of them, and `tag` must be null or one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_host_seen(
    instance: *const c_char,
    addresses: *const *const c_char,
    count: usize,
    port: u16,
    tag: *const c_char,
) {
    // SAFETY: the caller guarantees all of these are valid.
    let (instance, addresses, tag) = unsafe {
        (
            read_plain(instance),
            texts(addresses, count),
            read_plain(tag),
        )
    };
    let (Some(instance), Some(addresses)) = (instance, addresses) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        #[derive(Serialize)]
        struct Seen<'a> {
            instance: &'a str,
            addresses: &'a [String],
            port: u16,
            tag: Option<&'a str>,
        }
        let seen = Seen {
            instance: &instance,
            addresses: &addresses,
            port,
            tag: tag.as_deref(),
        };
        send("discovery_seen", seen, 1024);
    }));
}

/// A device no longer seen on the network.
///
/// # Safety
/// `instance` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_host_lost(instance: *const c_char) {
    // SAFETY: the caller guarantees the string is valid.
    let Some(instance) = (unsafe { read_plain(instance) }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        #[derive(Serialize)]
        struct Lost<'a> {
            instance: &'a str,
        }
        send(
            "discovery_lost",
            Lost {
                instance: &instance,
            },
            512,
        );
    }));
}

/// Ends a pairing under way, as when the window that showed it closes.
#[unsafe(no_mangle)]
pub extern "C" fn zh_host_pair_cancel() {
    let _ = catch_unwind(|| send("pair_cancel", Value::Null, 64));
}

/// Finishes starting: loads this device's key and starts listening. Waits for
/// it, so it must not be called from one of the engine's own threads. Returns
/// 0 when done, 1 when the core says why not, 2 when the engine is not running.
#[unsafe(no_mangle)]
pub extern "C" fn zh_host_bootstrap() -> i32 {
    catch_unwind(|| {
        let Some(engine) = ENGINE.get() else {
            return 2;
        };
        let answer = engine
            .request_now(r#"{"id":0,"method":"bootstrap","params":{"allow_loopback":false}}"#);
        match serde_json::from_str::<Value>(&answer) {
            Ok(reply) if reply.get("error").is_none() => 0,
            _ => 1,
        }
    })
    .unwrap_or(2)
}

/// # Safety
/// `text` must be null or point to a valid NUL-terminated string.
unsafe fn read_plain(text: *const c_char) -> Option<String> {
    if text.is_null() {
        return None;
    }
    // SAFETY: not null, and the caller guarantees NUL termination.
    unsafe { CStr::from_ptr(text) }
        .to_str()
        .ok()
        .map(str::to_string)
}

/// # Safety
/// `list` must be null or point to `count` valid NUL-terminated strings.
unsafe fn texts(list: *const *const c_char, count: usize) -> Option<Vec<String>> {
    if list.is_null() {
        return (count == 0).then(Vec::new);
    }
    // SAFETY: not null, and the caller guarantees `count` readable pointers.
    let pointers = unsafe { std::slice::from_raw_parts(list, count) };
    pointers
        .iter()
        // SAFETY: the caller guarantees each string is valid.
        .map(|&p| unsafe { read_plain(p) })
        .collect()
}

// ---- Pictures on the Windows clipboard ----

/// The most a copied picture may take, as on the Mac.
const MAX_PICTURE: usize = 24 * 1024 * 1024;
/// Larger pictures are not unpacked, so that one copy cannot use up memory.
const MAX_PIXELS: u64 = 40_000_000;

const FILE_HEADER: usize = 14;
const INFO_HEADER: usize = 40;
const PLAIN: u32 = 0;
const BIT_FIELDS: u32 = 3;
const ALPHA_BIT_FIELDS: u32 = 6;

/// Puts a file header in front of a clipboard bitmap so that the system's
/// decoder can read it. None when the header does not add up.
fn bmp_from_dib(dib: &[u8]) -> Option<Vec<u8>> {
    let u16_at = |at: usize| Some(u16::from_le_bytes(dib.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(dib.get(at..at + 4)?.try_into().ok()?));
    if dib.len() < INFO_HEADER || dib.len() > MAX_PICTURE {
        return None;
    }
    let header = usize::try_from(u32_at(0)?).ok()?;
    let width = i64::from(u32_at(4)?.cast_signed());
    let height = i64::from(u32_at(8)?.cast_signed());
    let planes = u16_at(12)?;
    let bits = u16_at(14)?;
    let compression = u32_at(16)?;
    let mut colors = u64::from(u32_at(32)?);
    if ![40, 52, 56, 108, 124].contains(&header)
        || header > dib.len()
        || planes != 1
        || ![1, 4, 8, 16, 24, 32].contains(&bits)
        || ![0, 1, 2, 3, 6].contains(&compression)
        || width <= 0
        || height == 0
        || colors > 256
    {
        return None;
    }
    if u64::try_from(width)
        .ok()?
        .checked_mul(height.unsigned_abs())?
        > MAX_PIXELS
    {
        return None;
    }
    if bits <= 8 && colors == 0 {
        colors = 1 << bits;
    }
    // The smallest header keeps its colour masks after it; larger ones inside.
    let masks = match (header, compression) {
        (INFO_HEADER, BIT_FIELDS) => 12,
        (INFO_HEADER, ALPHA_BIT_FIELDS) => 16,
        _ => 0,
    };
    let offset = header + masks + usize::try_from(colors).ok()? * 4;
    if offset > dib.len() {
        return None;
    }
    if compression == PLAIN || compression == BIT_FIELDS {
        let row_bits = u64::try_from(width).ok()?.checked_mul(u64::from(bits))?;
        let stride = row_bits.checked_add(31)? / 32 * 4;
        let pixels = stride.checked_mul(height.unsigned_abs())?;
        if u64::try_from(offset).ok()?.checked_add(pixels)? > u64::try_from(dib.len()).ok()? {
            return None;
        }
    }
    let total = FILE_HEADER + dib.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&u32::try_from(total).ok()?.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&u32::try_from(FILE_HEADER + offset).ok()?.to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

/// A 32-bit bitmap with its rows from the bottom up, the layout every Windows
/// program reads, from rows of blue, green, red and alpha from the top down.
fn dib_from_bgra(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return None;
    }
    let stride = usize::try_from(width).ok()?.checked_mul(4)?;
    let rows = usize::try_from(height).ok()?;
    let size = stride.checked_mul(rows)?;
    if pixels.len() != size {
        return None;
    }
    let mut out = Vec::with_capacity(INFO_HEADER + size);
    out.extend_from_slice(&u32::try_from(INFO_HEADER).ok()?.to_le_bytes());
    out.extend_from_slice(&i32::try_from(width).ok()?.to_le_bytes());
    out.extend_from_slice(&i32::try_from(height).ok()?.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&PLAIN.to_le_bytes());
    out.extend_from_slice(&u32::try_from(size).ok()?.to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    for row in pixels.chunks_exact(stride).rev() {
        out.extend_from_slice(row);
    }
    Some(out)
}

/// Hands bytes over to C. They are freed with [`zh_free_bytes`].
fn hand_over(bytes: Vec<u8>, out_len: *mut usize) -> *mut u8 {
    if out_len.is_null() {
        return std::ptr::null_mut();
    }
    let boxed = bytes.into_boxed_slice();
    // SAFETY: not null, and the caller guarantees it is writable.
    unsafe { *out_len = boxed.len() };
    Box::into_raw(boxed).cast()
}

/// A BMP file made from a clipboard bitmap (the bytes of `CF_DIB` or
/// `CF_DIBV5`), for the system's picture decoder, with its length in
/// `out_len`. Null when the bitmap's header does not add up.
///
/// # Safety
/// `dib` must point to `len` readable bytes, and `out_len` to a writable size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_bmp_from_dib(
    dib: *const u8,
    len: usize,
    out_len: *mut usize,
) -> *mut u8 {
    if dib.is_null() || len == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: not null, and the caller guarantees `len` readable bytes.
    let dib = unsafe { std::slice::from_raw_parts(dib, len) };
    catch_unwind(|| bmp_from_dib(dib))
        .ok()
        .flatten()
        .map_or(std::ptr::null_mut(), |bmp| hand_over(bmp, out_len))
}

/// A clipboard bitmap (`CF_DIB`) made from decoded pixels: rows of blue,
/// green, red and alpha bytes from the top down, `width` by `height`. Its
/// length goes in `out_len`. Null when the sizes do not match.
///
/// # Safety
/// `pixels` must point to `width * height * 4` readable bytes, and `out_len`
/// to a writable size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_dib_from_bgra(
    pixels: *const u8,
    width: u32,
    height: u32,
    out_len: *mut usize,
) -> *mut u8 {
    let Some(len) = usize::try_from(u64::from(width) * u64::from(height) * 4).ok() else {
        return std::ptr::null_mut();
    };
    if pixels.is_null() || len == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: not null, and the caller guarantees `len` readable bytes.
    let pixels = unsafe { std::slice::from_raw_parts(pixels, len) };
    catch_unwind(|| dib_from_bgra(pixels, width, height))
        .ok()
        .flatten()
        .map_or(std::ptr::null_mut(), |dib| hand_over(dib, out_len))
}

/// Frees bytes from [`zh_bmp_from_dib`] or [`zh_dib_from_bgra`].
///
/// # Safety
/// `bytes` and `len` must be exactly as one of those returned them, and the
/// bytes must not have been freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zh_free_bytes(bytes: *mut u8, len: usize) {
    if bytes.is_null() {
        return;
    }
    // SAFETY: the pointer and length came from `hand_over`.
    let boxed = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(bytes, len)) };
    // Pictures copied by the owner.
    drop(Zeroizing::new(boxed.into_vec()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Heard {
        calls: Vec<String>,
    }

    thread_local! {
        static HEARD: RefCell<Heard> = RefCell::new(Heard::default());
    }

    fn text_of(p: *const c_char) -> String {
        if p.is_null() {
            return "null".into();
        }
        // SAFETY: the reader passes valid strings for the length of the call.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }

    fn note(line: String) {
        HEARD.with(|h| h.borrow_mut().calls.push(line));
    }

    extern "C" fn on_snapshot(
        _: *mut c_void,
        on: bool,
        pairing: bool,
        instance: *const c_char,
        tag: *const c_char,
        port: u16,
    ) {
        note(format!(
            "snapshot {on} {pairing} {} {} {port}",
            text_of(instance),
            text_of(tag)
        ));
    }
    extern "C" fn on_offer(_: *mut c_void) {
        note("offer".into());
    }
    extern "C" fn on_text(_: *mut c_void, text: *const c_char, ttl: u64, from: *const c_char) {
        note(format!("text {} {ttl} {}", text_of(text), text_of(from)));
    }
    extern "C" fn on_picture(
        _: *mut c_void,
        bytes: *const u8,
        len: usize,
        ttl: u64,
        from: *const c_char,
    ) {
        // SAFETY: the reader passes `len` readable bytes.
        let bytes = unsafe { std::slice::from_raw_parts(bytes, len) };
        note(format!("picture {bytes:?} {ttl} {}", text_of(from)));
    }
    extern "C" fn on_files(_: *mut c_void, id: *const c_char, ttl: u64, from: *const c_char) {
        note(format!("files {} {ttl} {}", text_of(id), text_of(from)));
    }

    fn calls() -> HostCalls {
        HostCalls {
            context: std::ptr::null_mut(),
            snapshot: Some(on_snapshot),
            offer_arrived: Some(on_offer),
            clip_text: Some(on_text),
            clip_picture: Some(on_picture),
            clip_files: Some(on_files),
        }
    }

    fn heard(message: &str) -> (i32, Vec<String>) {
        HEARD.with(|h| h.borrow_mut().calls.clear());
        let text = CString::new(message).unwrap_or_default();
        let calls = calls();
        // SAFETY: both are valid for the call.
        let found = unsafe { zh_host_read(text.as_ptr(), &raw const calls) };
        (found, HEARD.with(|h| h.borrow().calls.clone()))
    }

    #[test]
    fn a_snapshot_says_what_to_announce() {
        let (found, calls) = heard(
            r#"{"event":"snapshot","settings":{"clipboard":true},"pairing":null,
               "advert":{"instance":"a1b2","tag":"ff00","port":50123}}"#,
        );
        assert_eq!(
            (found, calls),
            (0, vec!["snapshot true false a1b2 ff00 50123".to_string()])
        );
    }

    #[test]
    fn a_snapshot_without_an_advert_announces_nothing() {
        let (_, calls) = heard(
            r#"{"event":"snapshot","settings":{"clipboard":false},"pairing":{"phase":"waiting"},"advert":null}"#,
        );
        assert_eq!(calls, vec!["snapshot false true null null 0".to_string()]);
    }

    #[test]
    fn copied_text_arrives_with_its_lifetime_and_sender() {
        let (_, calls) = heard(
            r#"{"event":"clipboard_apply","format":"text","text":"héllo","ttl_ms":120000,"from":"Mac"}"#,
        );
        assert_eq!(calls, vec!["text héllo 120000 Mac".to_string()]);
    }

    #[test]
    fn a_copied_picture_arrives_as_its_bytes() {
        let (_, calls) = heard(
            r#"{"event":"clipboard_apply","format":"png","data":"AQID","ttl_ms":5,"from":"Mac"}"#,
        );
        assert_eq!(calls, vec!["picture [1, 2, 3] 5 Mac".to_string()]);
    }

    #[test]
    fn a_picture_that_is_not_base64_is_dropped() {
        let (_, calls) = heard(
            r#"{"event":"clipboard_apply","format":"png","data":"%%%","ttl_ms":5,"from":"Mac"}"#,
        );
        assert!(calls.is_empty());
    }

    #[test]
    fn text_holding_nul_is_dropped() {
        let (_, calls) = heard(
            r#"{"event":"clipboard_apply","format":"text","text":"a\u0000b","ttl_ms":5,"from":"Mac"}"#,
        );
        assert!(calls.is_empty());
    }

    #[test]
    fn copied_files_arrive_as_their_name() {
        let (_, calls) = heard(r#"{"event":"clipboard_files","id":"abcd","ttl_ms":7,"from":"PC"}"#);
        assert_eq!(calls, vec!["files abcd 7 PC".to_string()]);
    }

    #[test]
    fn an_offer_is_announced() {
        assert_eq!(
            heard(r#"{"event":"offer_arrived","id":3}"#).1,
            vec!["offer".to_string()]
        );
    }

    #[test]
    fn answers_to_own_requests_go_no_further() {
        assert_eq!(heard(r#"{"reply":0,"ok":null}"#), (1, vec![]));
    }

    #[test]
    fn answers_to_the_interface_pass_on() {
        assert_eq!(
            heard(r#"{"reply":1794500000000000,"ok":null}"#),
            (0, vec![])
        );
    }

    #[test]
    fn other_events_pass_on_untouched() {
        assert_eq!(heard(r#"{"event":"progress","items":[]}"#), (0, vec![]));
    }

    #[test]
    fn broken_messages_are_refused() {
        assert_eq!(heard("not json").0, 2);
    }

    /// A bitmap header like the one Windows puts on the clipboard.
    #[derive(Clone, Copy)]
    struct Header {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bits: u16,
        compression: u32,
        colors: u32,
        /// Bytes after the header.
        extra: usize,
    }

    const PLAIN_HEADER: Header = Header {
        size: 40,
        width: 1,
        height: 1,
        planes: 1,
        bits: 32,
        compression: 0,
        colors: 0,
        extra: 4,
    };

    fn header(h: Header) -> Vec<u8> {
        let mut dib = vec![0u8; h.size as usize + h.extra];
        dib[0..4].copy_from_slice(&h.size.to_le_bytes());
        dib[4..8].copy_from_slice(&h.width.to_le_bytes());
        dib[8..12].copy_from_slice(&h.height.to_le_bytes());
        dib[12..14].copy_from_slice(&h.planes.to_le_bytes());
        dib[14..16].copy_from_slice(&h.bits.to_le_bytes());
        dib[16..20].copy_from_slice(&h.compression.to_le_bytes());
        dib[32..36].copy_from_slice(&h.colors.to_le_bytes());
        dib
    }

    fn plain() -> Vec<u8> {
        header(PLAIN_HEADER)
    }

    fn pixel_offset(bmp: &[u8]) -> u32 {
        u32::from_le_bytes([bmp[10], bmp[11], bmp[12], bmp[13]])
    }

    #[test]
    fn a_plain_bitmap_gets_a_file_header_pointing_past_its_own() {
        let bmp = bmp_from_dib(&plain()).unwrap_or_default();
        assert_eq!(
            (&bmp[0..2], bmp.len(), pixel_offset(&bmp)),
            (&b"BM"[..], 58, 54)
        );
    }

    #[test]
    fn the_colour_masks_after_the_smallest_header_are_skipped() {
        let bmp = bmp_from_dib(&header(Header {
            compression: 3,
            extra: 16,
            ..PLAIN_HEADER
        }))
        .unwrap_or_default();
        assert_eq!(pixel_offset(&bmp), 14 + 40 + 12);
    }

    #[test]
    fn the_masks_inside_a_larger_header_are_not_counted_twice() {
        let bmp = bmp_from_dib(&header(Header {
            size: 124,
            compression: 3,
            ..PLAIN_HEADER
        }))
        .unwrap_or_default();
        assert_eq!(pixel_offset(&bmp), 14 + 124);
    }

    #[test]
    fn the_colour_table_of_a_paletted_bitmap_is_skipped() {
        let bmp = bmp_from_dib(&header(Header {
            bits: 8,
            extra: 256 * 4 + 4,
            ..PLAIN_HEADER
        }))
        .unwrap_or_default();
        assert_eq!(pixel_offset(&bmp), 14 + 40 + 256 * 4);
    }

    #[test]
    fn rows_stored_top_down_are_accepted() {
        assert!(
            bmp_from_dib(&header(Header {
                height: -1,
                ..PLAIN_HEADER
            }))
            .is_some()
        );
    }

    #[test]
    fn bitmaps_that_do_not_add_up_are_refused() {
        let refused = [
            vec![0u8; 39],
            header(Header {
                size: 12,
                extra: 40,
                ..PLAIN_HEADER
            }),
            header(Header {
                size: 124,
                extra: 0,
                ..PLAIN_HEADER
            })[..100]
                .to_vec(),
            header(Header {
                width: 0,
                ..PLAIN_HEADER
            }),
            header(Header {
                height: 0,
                ..PLAIN_HEADER
            }),
            header(Header {
                planes: 2,
                ..PLAIN_HEADER
            }),
            header(Header {
                bits: 7,
                ..PLAIN_HEADER
            }),
            header(Header {
                compression: 5,
                ..PLAIN_HEADER
            }),
            header(Header {
                colors: 257,
                extra: 2000,
                ..PLAIN_HEADER
            }),
            header(Header {
                bits: 8,
                extra: 100,
                ..PLAIN_HEADER
            }),
            header(Header {
                width: 2,
                height: 2,
                extra: 8,
                ..PLAIN_HEADER
            }),
            header(Header {
                width: 100_000,
                height: 1000,
                ..PLAIN_HEADER
            }),
            header(Header {
                width: i32::MAX,
                height: i32::MIN,
                ..PLAIN_HEADER
            }),
        ];
        for dib in refused {
            assert!(
                bmp_from_dib(&dib).is_none(),
                "{:?}",
                &dib[..dib.len().min(40)]
            );
        }
    }

    // Top row: red, green. Bottom row: blue, half-transparent white. In the
    // order Windows keeps them: blue, green, red, alpha.
    const BGRA: [u8; 16] = [
        0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 128,
    ];

    #[test]
    fn a_bitmap_from_pixels_has_the_header_windows_expects() {
        let dib = dib_from_bgra(&BGRA, 2, 2).unwrap_or_default();
        let u32_at =
            |at: usize| u32::from_le_bytes([dib[at], dib[at + 1], dib[at + 2], dib[at + 3]]);
        let u16_at = |at: usize| u16::from_le_bytes([dib[at], dib[at + 1]]);
        assert_eq!(
            [
                u32_at(0),
                u32_at(4),
                u32_at(8),
                u32::from(u16_at(12)),
                u32::from(u16_at(14)),
                u32_at(16),
                u32_at(20),
                dib.len() as u32
            ],
            [40, 2, 2, 1, 32, 0, 16, 56]
        );
    }

    #[test]
    fn a_bitmap_from_pixels_stores_its_rows_bottom_up() {
        let dib = dib_from_bgra(&BGRA, 2, 2).unwrap_or_default();
        assert_eq!(
            &dib[40..],
            &[
                255, 0, 0, 255, 255, 255, 255, 128, 0, 0, 255, 255, 0, 255, 0, 255
            ]
        );
    }

    #[test]
    fn pixels_of_the_wrong_size_give_no_bitmap() {
        assert!(dib_from_bgra(&BGRA[..12], 2, 2).is_none());
    }

    #[test]
    fn the_bytes_handed_over_can_be_freed() {
        let mut len = 0usize;
        let dib = plain();
        // SAFETY: valid bytes and a writable length.
        let bmp = unsafe { zh_bmp_from_dib(dib.as_ptr(), dib.len(), &raw mut len) };
        assert_eq!((bmp.is_null(), len), (false, 58));
        // SAFETY: exactly as handed over.
        unsafe { zh_free_bytes(bmp, len) };
    }
}
