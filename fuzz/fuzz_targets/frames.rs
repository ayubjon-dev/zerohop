//! Every decoder that sees bytes from the network.
#![no_main]

use std::fmt::Debug;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::runtime::Runtime;
use zerohop_core::wire::*;

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_current_thread().build().unwrap())
}

fn round_trip<T: DeserializeOwned + Serialize + PartialEq + Debug>(payload: &[u8]) {
    if let Ok(value) = decode::<T>(payload) {
        let encoded = postcard::to_stdvec(&value).unwrap();
        let again: T = decode(&encoded).unwrap();
        assert_eq!(value, again);
    }
}

fn all_messages(p: &[u8]) {
    round_trip::<ErrorMsg>(p);
    round_trip::<Hello>(p);
    round_trip::<LedgerHave>(p);
    round_trip::<LedgerEvents>(p);
    round_trip::<Offer>(p);
    round_trip::<OfferItems>(p);
    round_trip::<Decision>(p);
    round_trip::<SegmentEnd>(p);
    round_trip::<FileEnd>(p);
    round_trip::<Progress>(p);
    round_trip::<Complete>(p);
    round_trip::<Done>(p);
    round_trip::<Cancel>(p);
    round_trip::<ResumeRequest>(p);
    round_trip::<ResumeState>(p);
    round_trip::<PakeA>(p);
    round_trip::<PakeB>(p);
    round_trip::<Confirm>(p);
    round_trip::<SasCommit>(p);
    round_trip::<SasNonce>(p);
    round_trip::<PeerInfo>(p);
    round_trip::<WelcomeEnd>(p);
    round_trip::<ClipOffer>(p);
    round_trip::<ClipFetch>(p);
    round_trip::<ClipData>(p);
    round_trip::<ClipEnd>(p);
    if let Ok((index, offset, body)) = parse_file_data(p) {
        assert_eq!(body.len(), p.len() - FILE_DATA_HEADER);
        assert_eq!(file_data_header(index, offset).as_slice(), &p[..FILE_DATA_HEADER]);
    }
}

fuzz_target!(|data: &[u8]| {
    all_messages(data);
    runtime().block_on(async {
        let mut input = data;
        for _ in 0..32 {
            let Ok(frame) = read_frame(&mut input).await else { break };
            assert!(frame.payload.len() <= MAX_DATA_PAYLOAD);
            all_messages(&frame.payload);
        }
        let mut input = data;
        if let Ok(frame) = read_frame_limited(&mut input, MAX_PRE_AUTH_PAYLOAD).await {
            assert!(frame.payload.len() <= MAX_PRE_AUTH_PAYLOAD);
        }
    });
});
