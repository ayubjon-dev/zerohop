//! Pairing links, which arrive from a QR code or a pasted string.
#![no_main]

use libfuzzer_sys::fuzz_target;
use zerohop_core::pairing::QrPayload;

fn check(text: &str) {
    if let Some(link) = QrPayload::decode(text) {
        assert_eq!(QrPayload::decode(&link.encode()), Some(link));
    }
}

fuzz_target!(|text: &str| {
    check(text);
    // Without the fixed prefix the fuzzer would rarely get past the first comparison.
    check(&format!("zerohop://pair/1?{text}"));
});
