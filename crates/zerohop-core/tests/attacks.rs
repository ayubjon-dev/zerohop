//! End-to-end behaviour and attack tests over real TLS on the loopback interface.
#![cfg(feature = "dev-loopback")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    missing_docs
)]

use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use zerohop_core::address::LocalNetworks;
use zerohop_core::circle::{Circle, KeySealing, generate_key};
use zerohop_core::clip::{ClipPart, ClipSink, MAX_CLIP_TTL, OutgoingClip, ReceivedClip};
use zerohop_core::ledger::{DeviceId, Platform};
use zerohop_core::node::{Node, NodeError};
use zerohop_core::pairing::{
    self, HostPin, PairingError, PairingListener, PairingPrompt, PairingSecret,
};
use zerohop_core::receive::{ReceiveFolder, SEGMENT_SIZE};
use zerohop_core::session::{LocalInfo, Session, SessionError, lock};
use zerohop_core::tls::{self, ALPN_NORMAL, ALPN_PAIRING, PeerRule, TlsError};
use zerohop_core::transfer::{
    self, Approver, Budgets, Control, DEFAULT_AUTO_LIMIT, OfferOutcome, OfferSummary,
    ReceiveConfig, ReceiveMode, ReceiveSettings, ReceiveStart, TransferError,
};
use zerohop_core::wire::{
    self, ClipData, ClipEnd, ClipFetch, ClipFormat, ClipItem, ClipOffer, Decision, FrameType, Item,
    ItemKind, Offer, OfferItems, PeerInfo, code, write_msg,
};
use zeroize::Zeroizing;

fn nets() -> LocalNetworks {
    LocalNetworks::from_subnets(Vec::new()).with_loopback()
}

fn info(name: &str) -> PeerInfo {
    PeerInfo {
        name: name.to_string(),
        platform: Platform::Other,
    }
}

fn local(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.to_string(),
        platform: Platform::Other,
    }
}

async fn listener() -> (TcpListener, SocketAddr) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap();
    (l, a)
}

async fn yes(_: PairingPrompt) -> bool {
    true
}

struct Pair {
    host: Node,
    joiner: Node,
    host_id: DeviceId,
    joiner_id: DeviceId,
    _host_dir: TempDir,
    _joiner_dir: TempDir,
}

async fn paired() -> Pair {
    let host_dir = tempfile::tempdir().unwrap();
    let joiner_dir = tempfile::tempdir().unwrap();
    let mut circle = Circle::create(host_dir.path(), "host").unwrap();
    let (l, addr) = listener().await;
    let secret = PairingSecret::new_code().unwrap();
    let typed = secret.clone();
    let join_path = joiner_dir.path().to_path_buf();
    let joining = tokio::spawn(async move {
        pairing::join(
            addr,
            &nets(),
            &join_path,
            generate_key().unwrap(),
            &typed,
            HostPin::None,
            info("joiner"),
            yes,
        )
        .await
    });
    pairing::host(l, &nets(), &mut circle, &secret, info("host"), yes)
        .await
        .unwrap();
    let joined = joining.await.unwrap().unwrap();
    let host_id = circle.device_id();
    let joiner_id = joined.device_id();
    Pair {
        host: Node::new(circle, nets(), local("host"), host_dir.path()),
        joiner: Node::new(joined, nets(), local("joiner"), joiner_dir.path()),
        host_id,
        joiner_id,
        _host_dir: host_dir,
        _joiner_dir: joiner_dir,
    }
}

struct Counting {
    answer: bool,
    asked: Arc<AtomicUsize>,
}

impl Approver for Counting {
    fn approve(&self, _: OfferSummary) -> Pin<Box<dyn Future<Output = bool> + Send>> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let answer = self.answer;
        Box::pin(async move { answer })
    }
}

fn config(folder: &Path, mode: ReceiveMode, answer: bool) -> (ReceiveConfig, Arc<AtomicUsize>) {
    let asked = Arc::new(AtomicUsize::new(0));
    let cfg = ReceiveConfig {
        settings: ReceiveSettings::new(ReceiveFolder::open(folder).unwrap(), mode),
        approver: Arc::new(Counting {
            answer,
            asked: asked.clone(),
        }),
        auto_limit: DEFAULT_AUTO_LIMIT,
        budgets: Arc::new(Budgets::default()),
        clip: None,
    };
    (cfg, asked)
}

fn write_file(dir: &Path, name: &str, data: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, data).unwrap();
    p
}

// ---- Pairing ----

#[tokio::test]
async fn pairing_adds_the_device_to_the_circle() {
    let pair = paired().await;
    let host = lock(pair.host.circle()).unwrap();
    let joiner = lock(pair.joiner.circle()).unwrap();
    assert!(host.is_member(&pair.joiner_id));
    assert!(joiner.is_member(&pair.host_id));
    assert!(joiner.is_member(&pair.joiner_id));
}

#[tokio::test]
async fn both_devices_see_the_same_comparison_number() {
    let host_dir = tempfile::tempdir().unwrap();
    let joiner_dir = tempfile::tempdir().unwrap();
    let mut circle = Circle::create(host_dir.path(), "host").unwrap();
    let (l, addr) = listener().await;
    let secret = PairingSecret::new_code().unwrap();
    let typed = secret.clone();
    let path = joiner_dir.path().to_path_buf();
    let (tx_h, rx_h) = tokio::sync::oneshot::channel();
    let (tx_j, rx_j) = tokio::sync::oneshot::channel();
    let joining = tokio::spawn(async move {
        pairing::join(
            addr,
            &nets(),
            &path,
            generate_key().unwrap(),
            &typed,
            HostPin::None,
            info("joiner"),
            move |p: PairingPrompt| async move {
                tx_j.send(p.comparison).unwrap();
                true
            },
        )
        .await
    });
    let mut tx_h = Some(tx_h);
    pairing::host(
        l,
        &nets(),
        &mut circle,
        &secret,
        info("host"),
        |p: PairingPrompt| {
            let tx = tx_h.take().unwrap();
            async move {
                tx.send(p.comparison).unwrap();
                true
            }
        },
    )
    .await
    .unwrap();
    joining.await.unwrap().unwrap();
    assert_eq!(rx_h.await.unwrap(), rx_j.await.unwrap());
}

#[tokio::test]
async fn a_wrong_code_fails_and_burns_the_real_one() {
    let host_dir = tempfile::tempdir().unwrap();
    let joiner_dir = tempfile::tempdir().unwrap();
    let mut circle = Circle::create(host_dir.path(), "host").unwrap();
    let (l, addr) = listener().await;
    let secret = PairingSecret::from_typed("123456");
    let path = joiner_dir.path().to_path_buf();
    let joining = tokio::spawn(async move {
        pairing::join(
            addr,
            &nets(),
            &path,
            generate_key().unwrap(),
            &PairingSecret::from_typed("000000"),
            HostPin::None,
            info("attacker"),
            yes,
        )
        .await
    });
    let result = pairing::host(l, &nets(), &mut circle, &secret, info("host"), yes).await;
    assert!(result.is_err(), "pairing mode must end after one attempt");
    assert!(joining.await.unwrap().is_err());
    assert_eq!(circle.ledger().membership().members.len(), 1);
}

#[tokio::test]
async fn declining_on_the_host_adds_nothing() {
    let host_dir = tempfile::tempdir().unwrap();
    let joiner_dir = tempfile::tempdir().unwrap();
    let mut circle = Circle::create(host_dir.path(), "host").unwrap();
    let (l, addr) = listener().await;
    let secret = PairingSecret::new_code().unwrap();
    let typed = secret.clone();
    let path = joiner_dir.path().to_path_buf();
    let joining = tokio::spawn(async move {
        pairing::join(
            addr,
            &nets(),
            &path,
            generate_key().unwrap(),
            &typed,
            HostPin::None,
            info("joiner"),
            yes,
        )
        .await
    });
    let result = pairing::host(l, &nets(), &mut circle, &secret, info("host"), |_| async {
        false
    })
    .await;
    assert!(matches!(result, Err(PairingError::Declined)));
    assert!(matches!(
        joining.await.unwrap(),
        Err(PairingError::Declined)
    ));
    assert_eq!(circle.ledger().membership().members.len(), 1);
    assert!(!Circle::exists(joiner_dir.path()));
}

#[tokio::test]
async fn declining_on_the_joiner_adds_nothing_on_the_host() {
    let host_dir = tempfile::tempdir().unwrap();
    let joiner_dir = tempfile::tempdir().unwrap();
    let mut circle = Circle::create(host_dir.path(), "host").unwrap();
    let (l, addr) = listener().await;
    let secret = PairingSecret::new_code().unwrap();
    let typed = secret.clone();
    let path = joiner_dir.path().to_path_buf();
    let joining = tokio::spawn(async move {
        pairing::join(
            addr,
            &nets(),
            &path,
            generate_key().unwrap(),
            &typed,
            HostPin::None,
            info("joiner"),
            |_: PairingPrompt| async { false },
        )
        .await
    });
    let result = pairing::host(l, &nets(), &mut circle, &secret, info("host"), yes).await;
    assert!(matches!(result, Err(PairingError::Declined)));
    assert!(joining.await.unwrap().is_err());
    assert_eq!(
        circle.ledger().all_events().count(),
        1,
        "the host must not have signed anything"
    );
}

#[tokio::test]
async fn a_qr_joiner_refuses_a_host_with_the_wrong_key() {
    let dir = tempfile::tempdir().unwrap();
    let host_dir = tempfile::tempdir().unwrap();
    let circle = Circle::create(host_dir.path(), "impostor").unwrap();
    let (l, addr) = listener().await;
    let cfg = tls::server_config(circle.key(), PeerRule::AnyForPairing, ALPN_PAIRING).unwrap();
    tokio::spawn(async move {
        let (tcp, peer) = l.accept().await.unwrap();
        let _ = tls::accept(&nets(), tcp, peer, cfg).await;
    });
    let secret = PairingSecret::new_qr().unwrap();
    let result = pairing::join(
        addr,
        &nets(),
        dir.path(),
        generate_key().unwrap(),
        &secret,
        HostPin::Key([9; 32]),
        info("joiner"),
        yes,
    )
    .await;
    assert!(matches!(result, Err(PairingError::Tls(_))));
}

#[tokio::test]
async fn a_qr_code_pairs_while_a_typed_code_is_also_open() {
    let host_dir = tempfile::tempdir().unwrap();
    let joiner_dir = tempfile::tempdir().unwrap();
    let mut circle = Circle::create(host_dir.path(), "host").unwrap();
    let host_id = circle.device_id();
    let (code_l, _) = listener().await;
    let (qr_l, qr_addr) = listener().await;
    let qr = PairingSecret::new_qr().unwrap();
    let scanned = PairingSecret::from_qr_bytes(qr.qr_bytes().unwrap());
    let path = joiner_dir.path().to_path_buf();
    let joining = tokio::spawn(async move {
        pairing::join(
            qr_addr,
            &nets(),
            &path,
            generate_key().unwrap(),
            &scanned,
            HostPin::Key(host_id),
            info("phone"),
            yes,
        )
        .await
    });
    let entries = vec![
        PairingListener {
            listener: code_l,
            secret: PairingSecret::new_code().unwrap(),
        },
        PairingListener {
            listener: qr_l,
            secret: qr,
        },
    ];
    let paired = pairing::host_any(entries, &nets(), &mut circle, info("host"), yes, || {})
        .await
        .unwrap();
    let joined = joining.await.unwrap().unwrap();
    assert_eq!(paired.device, joined.device_id());
}

#[tokio::test]
async fn a_spent_code_leaves_the_qr_code_working() {
    let host_dir = tempfile::tempdir().unwrap();
    let attacker_dir = tempfile::tempdir().unwrap();
    let joiner_dir = tempfile::tempdir().unwrap();
    let mut circle = Circle::create(host_dir.path(), "host").unwrap();
    let host_id = circle.device_id();
    let (code_l, code_addr) = listener().await;
    let (qr_l, qr_addr) = listener().await;
    let qr = PairingSecret::new_qr().unwrap();
    let scanned = PairingSecret::from_qr_bytes(qr.qr_bytes().unwrap());
    let (spent_tx, spent_rx) = tokio::sync::oneshot::channel::<()>();
    let attacker_path = attacker_dir.path().to_path_buf();
    let path = joiner_dir.path().to_path_buf();
    let joining = tokio::spawn(async move {
        let guess = pairing::join(
            code_addr,
            &nets(),
            &attacker_path,
            generate_key().unwrap(),
            &PairingSecret::from_typed("000000"),
            HostPin::None,
            info("attacker"),
            yes,
        )
        .await;
        assert!(guess.is_err());
        spent_rx.await.unwrap();
        // The spent code's listener is closed.
        assert!(TcpStream::connect(code_addr).await.is_err());
        pairing::join(
            qr_addr,
            &nets(),
            &path,
            generate_key().unwrap(),
            &scanned,
            HostPin::Key(host_id),
            info("phone"),
            yes,
        )
        .await
    });
    let entries = vec![
        PairingListener {
            listener: code_l,
            secret: PairingSecret::from_typed("123456"),
        },
        PairingListener {
            listener: qr_l,
            secret: qr,
        },
    ];
    let mut spent_tx = Some(spent_tx);
    let paired = pairing::host_any(entries, &nets(), &mut circle, info("host"), yes, || {
        if let Some(tx) = spent_tx.take() {
            tx.send(()).unwrap();
        }
    })
    .await
    .unwrap();
    let joined = joining.await.unwrap().unwrap();
    assert_eq!(paired.device, joined.device_id());
    assert_eq!(circle.ledger().membership().members.len(), 2);
}

#[tokio::test]
async fn a_spent_code_ends_pairing_when_nothing_else_is_open() {
    let host_dir = tempfile::tempdir().unwrap();
    let joiner_dir = tempfile::tempdir().unwrap();
    let mut circle = Circle::create(host_dir.path(), "host").unwrap();
    let (l, addr) = listener().await;
    let path = joiner_dir.path().to_path_buf();
    let joining = tokio::spawn(async move {
        pairing::join(
            addr,
            &nets(),
            &path,
            generate_key().unwrap(),
            &PairingSecret::from_typed("000000"),
            HostPin::None,
            info("attacker"),
            yes,
        )
        .await
    });
    let entries = vec![PairingListener {
        listener: l,
        secret: PairingSecret::from_typed("123456"),
    }];
    let calls = AtomicUsize::new(0);
    let result = pairing::host_any(entries, &nets(), &mut circle, info("host"), yes, || {
        calls.fetch_add(1, Ordering::SeqCst);
    })
    .await;
    assert!(result.is_err());
    assert!(joining.await.unwrap().is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_qr_joiner_moves_on_from_an_address_that_does_not_answer() {
    let host_dir = tempfile::tempdir().unwrap();
    let joiner_dir = tempfile::tempdir().unwrap();
    let mut circle = Circle::create(host_dir.path(), "host").unwrap();
    let host_id = circle.device_id();
    let (l, addr) = listener().await;
    // Bound and dropped, so nothing listens there any more.
    let (gone, dead) = listener().await;
    drop(gone);
    let qr = PairingSecret::new_qr().unwrap();
    let scanned = PairingSecret::from_qr_bytes(qr.qr_bytes().unwrap());
    let path = joiner_dir.path().to_path_buf();
    let joining = tokio::spawn(async move {
        pairing::join_sealed(
            &[dead, addr],
            &nets(),
            &path,
            generate_key().unwrap(),
            &scanned,
            HostPin::Key(host_id),
            info("phone"),
            &KeySealing::default(),
            yes,
        )
        .await
    });
    pairing::host(l, &nets(), &mut circle, &qr, info("host"), yes)
        .await
        .unwrap();
    assert!(joining.await.unwrap().is_ok());
}

#[tokio::test]
async fn a_qr_joiner_refuses_addresses_off_the_local_network() {
    let dir = tempfile::tempdir().unwrap();
    let result = pairing::join_sealed(
        &["8.8.8.8:53319".parse().unwrap()],
        &nets(),
        dir.path(),
        generate_key().unwrap(),
        &PairingSecret::new_qr().unwrap(),
        HostPin::Key([9; 32]),
        info("phone"),
        &KeySealing::default(),
        yes,
    )
    .await;
    assert!(matches!(result, Err(PairingError::Tls(TlsError::NotLocal))));
}

// ---- Transport ----

#[tokio::test]
async fn a_stranger_cannot_complete_a_handshake_with_a_device() {
    let pair = paired().await;
    let (l, addr) = listener().await;
    let target = pair.host.clone();
    let folder = tempfile::tempdir().unwrap();
    let (cfg, _) = config(folder.path(), ReceiveMode::Automatic, true);
    tokio::spawn(async move { target.serve(l, cfg).await });

    let stranger = generate_key().unwrap();
    let client = tls::client_config(&stranger, PeerRule::Exact(pair.host_id), ALPN_NORMAL).unwrap();
    let outcome = async {
        let stream = tls::connect(&nets(), addr, client).await?;
        let (mut rx, _tx) = tokio::io::split(stream);
        // TLS 1.3 clients finish first, so the refusal shows up on the first read.
        let r = wire::read_frame(&mut rx).await;
        assert!(r.is_err());
        Ok::<_, TlsError>(())
    }
    .await;
    if let Err(e) = outcome {
        assert!(matches!(e, TlsError::Io(_) | TlsError::Timeout), "{e:?}");
    }
}

#[tokio::test]
async fn a_client_refuses_a_member_that_is_not_the_intended_one() {
    let pair = paired().await;
    let (l, addr) = listener().await;
    let target = pair.host.clone();
    let folder = tempfile::tempdir().unwrap();
    let (cfg, _) = config(folder.path(), ReceiveMode::Automatic, true);
    tokio::spawn(async move { target.serve(l, cfg).await });

    let result = pair.joiner.sync_with(pair.joiner_id, addr).await;
    assert!(
        matches!(result, Err(NodeError::Tls(_))),
        "{:?}",
        result.err()
    );
}

#[tokio::test]
async fn connections_to_non_local_addresses_are_refused() {
    let key = generate_key().unwrap();
    let cfg = tls::client_config(&key, PeerRule::Exact([1; 32]), ALPN_NORMAL).unwrap();
    let no_loopback = LocalNetworks::from_subnets(Vec::new());
    let result = tls::connect(&no_loopback, "127.0.0.1:9".parse().unwrap(), cfg.clone()).await;
    assert!(matches!(result, Err(TlsError::NotLocal)));
    let result = tls::connect(&nets(), "8.8.8.8:443".parse().unwrap(), cfg).await;
    assert!(matches!(result, Err(TlsError::NotLocal)));
}

#[tokio::test]
async fn the_normal_listener_refuses_the_pairing_protocol() {
    let pair = paired().await;
    let (l, addr) = listener().await;
    let target = pair.host.clone();
    let folder = tempfile::tempdir().unwrap();
    let (cfg, _) = config(folder.path(), ReceiveMode::Automatic, true);
    tokio::spawn(async move { target.serve(l, cfg).await });

    let key = generate_key().unwrap();
    let client = tls::client_config(&key, PeerRule::AnyForPairing, ALPN_PAIRING).unwrap();
    let result = tls::connect(&nets(), addr, client).await;
    assert!(result.is_err(), "ALPN mismatch must fail the handshake");
}

// ---- Transfers ----

async fn send_and_serve(
    pair: &Pair,
    cfg: ReceiveConfig,
    paths: Vec<std::path::PathBuf>,
) -> (Result<transfer::SendReport, NodeError>, Vec<OfferOutcome>) {
    let (l, addr) = listener().await;
    let server = pair.host.clone();
    let serving = tokio::spawn(async move { server.serve_one(&l, &cfg).await });
    let plan = transfer::plan(&paths).unwrap();
    let report = pair.joiner.send(pair.host_id, addr, plan).await;
    let outcomes = serving.await.unwrap().unwrap_or_default();
    (report, outcomes)
}

#[tokio::test]
async fn sends_a_file_byte_for_byte() {
    let pair = paired().await;
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    let path = write_file(src.path(), "video.mov", &data);
    let (cfg, _) = config(dst.path(), ReceiveMode::AutomaticExceptPrograms, true);
    let (report, _) = send_and_serve(&pair, cfg, vec![path]).await;
    assert_eq!(report.unwrap().published, 1);
    assert_eq!(std::fs::read(dst.path().join("video.mov")).unwrap(), data);
}

#[tokio::test]
async fn sends_a_multi_segment_file() {
    let pair = paired().await;
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let size = SEGMENT_SIZE as usize * 2 + 12345;
    let data: Vec<u8> = (0..size).map(|i| (i % 253) as u8).collect();
    let path = write_file(src.path(), "big.bin", &data);
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    let (report, _) = send_and_serve(&pair, cfg, vec![path]).await;
    report.unwrap();
    assert_eq!(std::fs::read(dst.path().join("big.bin")).unwrap(), data);
}

#[tokio::test]
async fn sends_a_folder_with_nested_and_empty_directories() {
    let pair = paired().await;
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let root = src.path().join("album");
    std::fs::create_dir_all(root.join("2024/june")).unwrap();
    std::fs::create_dir_all(root.join("empty")).unwrap();
    std::fs::write(root.join("a.txt"), b"one").unwrap();
    std::fs::write(root.join("2024/june/b.txt"), b"two").unwrap();
    std::fs::write(root.join("zero.bin"), b"").unwrap();
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    let (report, _) = send_and_serve(&pair, cfg, vec![root]).await;
    assert_eq!(report.unwrap().published, 1);
    assert_eq!(
        std::fs::read(dst.path().join("album/a.txt")).unwrap(),
        b"one"
    );
    assert_eq!(
        std::fs::read(dst.path().join("album/2024/june/b.txt")).unwrap(),
        b"two"
    );
    assert_eq!(
        std::fs::read(dst.path().join("album/zero.bin")).unwrap(),
        b""
    );
    assert!(dst.path().join("album/empty").is_dir());
}

#[cfg(unix)]
#[tokio::test]
async fn does_not_follow_or_send_symlinks() {
    let pair = paired().await;
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let secret = tempfile::tempdir().unwrap();
    std::fs::write(secret.path().join("private.txt"), b"secret").unwrap();
    let root = src.path().join("folder");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("ok.txt"), b"fine").unwrap();
    std::os::unix::fs::symlink(secret.path().join("private.txt"), root.join("link.txt")).unwrap();
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    let (report, _) = send_and_serve(&pair, cfg, vec![root]).await;
    report.unwrap();
    assert!(dst.path().join("folder/ok.txt").exists());
    assert!(!dst.path().join("folder/link.txt").exists());
}

#[tokio::test]
async fn preserves_the_modification_time() {
    let pair = paired().await;
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let path = write_file(src.path(), "old.txt", b"data");
    let stamp = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(stamp)
        .unwrap();
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    send_and_serve(&pair, cfg, vec![path]).await.0.unwrap();
    let got = std::fs::metadata(dst.path().join("old.txt"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(got, stamp);
}

#[tokio::test]
async fn asks_before_accepting_a_program_by_default() {
    let pair = paired().await;
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let path = write_file(src.path(), "setup.EXE", b"binary");
    let (cfg, asked) = config(dst.path(), ReceiveMode::AutomaticExceptPrograms, false);
    let (report, _) = send_and_serve(&pair, cfg, vec![path]).await;
    assert!(matches!(
        report,
        Err(NodeError::Transfer(TransferError::Declined(code::DECLINED)))
    ));
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert!(!dst.path().join("setup.EXE").exists());
}

#[tokio::test]
async fn does_not_ask_for_an_ordinary_file_by_default() {
    let pair = paired().await;
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let path = write_file(src.path(), "photo.jpg", b"\xFF\xD8data");
    let (cfg, asked) = config(dst.path(), ReceiveMode::AutomaticExceptPrograms, false);
    send_and_serve(&pair, cfg, vec![path]).await.0.unwrap();
    assert_eq!(asked.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn holds_an_executable_disguised_as_a_photo() {
    let pair = paired().await;
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let path = write_file(src.path(), "cat.jpg", b"MZ\x90\0payload");
    let (cfg, asked) = config(dst.path(), ReceiveMode::AutomaticExceptPrograms, true);
    let report = send_and_serve(&pair, cfg, vec![path]).await.0.unwrap();
    assert_eq!(asked.load(Ordering::SeqCst), 0);
    assert_eq!(report.held, 1);
    assert_eq!(report.published, 0);
    assert!(!dst.path().join("cat.jpg").exists());
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn marks_received_files_as_downloaded() {
    let pair = paired().await;
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let path = write_file(src.path(), "doc.pdf", b"%PDF");
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    send_and_serve(&pair, cfg, vec![path]).await.0.unwrap();
    let out = std::process::Command::new("xattr")
        .arg("-p")
        .arg("com.apple.quarantine")
        .arg(dst.path().join("doc.pdf"))
        .output()
        .unwrap();
    assert!(out.status.success(), "quarantine attribute must be set");
}

// ---- A malicious member speaking the protocol directly ----

async fn malicious_session(
    pair: &Pair,
    cfg: ReceiveConfig,
) -> (
    Session<tls::ClientStream>,
    tokio::task::JoinHandle<Result<Vec<OfferOutcome>, NodeError>>,
) {
    let (l, addr) = listener().await;
    let server = pair.host.clone();
    let serving = tokio::spawn(async move { server.serve_one(&l, &cfg).await });
    let key = lock(pair.joiner.circle()).unwrap().key().clone();
    let client = tls::client_config(&key, PeerRule::Exact(pair.host_id), ALPN_NORMAL).unwrap();
    let stream = tls::connect(&nets(), addr, client).await.unwrap();
    let members = lock(pair.joiner.circle())
        .unwrap()
        .ledger()
        .membership()
        .members;
    let (_tx, rx) = watch::channel(members);
    std::mem::forget(_tx);
    let session = Session::establish(
        stream,
        pair.host_id,
        pair.joiner.circle(),
        &local("evil"),
        rx,
    )
    .await
    .unwrap();
    (session, serving)
}

fn raw_item(index: u32, path: &[&str], size: u64) -> Item {
    Item {
        index,
        path: path.iter().map(ToString::to_string).collect(),
        kind: ItemKind::File,
        size,
        mtime_ms: None,
        created_ms: None,
        group: None,
    }
}

async fn offer(session: &mut Session<tls::ClientStream>, items: Vec<Item>, total: u64) {
    let msg = Offer {
        transfer_id: [7; 16],
        item_count: u32::try_from(items.len()).unwrap(),
        total_bytes: total,
    };
    write_msg(&mut session.writer, FrameType::Offer, &msg)
        .await
        .unwrap();
    write_msg(
        &mut session.writer,
        FrameType::OfferItems,
        &OfferItems { items },
    )
    .await
    .unwrap();
    session.flush().await.unwrap();
}

async fn expect_declined(session: &mut Session<tls::ClientStream>, reason: u16) {
    let frame = session.next_frame().await.unwrap();
    assert_eq!(frame.kind, FrameType::Decision);
    let d: Decision = frame.decode().unwrap();
    assert!(!d.accept);
    assert_eq!(d.reason, Some(reason));
}

#[tokio::test]
async fn refuses_traversal_hidden_behind_an_invisible_character() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    let (mut s, _serving) = malicious_session(&pair, cfg).await;
    offer(
        &mut s,
        vec![raw_item(0, &["..\u{200E}", "..\u{200E}", "evil.txt"], 1)],
        1,
    )
    .await;
    expect_declined(&mut s, code::INVALID_PATH).await;
}

#[tokio::test]
async fn refuses_absolute_and_separator_paths() {
    for bad in ["/etc/passwd", "a/../../b", "..\\..\\windows"] {
        let pair = paired().await;
        let dst = tempfile::tempdir().unwrap();
        let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
        let (mut s, _serving) = malicious_session(&pair, cfg).await;
        offer(&mut s, vec![raw_item(0, &[bad], 1)], 1).await;
        expect_declined(&mut s, code::INVALID_PATH).await;
    }
}

#[tokio::test]
async fn refuses_a_manifest_that_lies_about_its_total() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let (cfg, asked) = config(dst.path(), ReceiveMode::Automatic, true);
    let (mut s, _serving) = malicious_session(&pair, cfg).await;
    offer(&mut s, vec![raw_item(0, &["big.bin"], 1 << 40)], 1).await;
    expect_declined(&mut s, code::INVALID_PATH).await;
    assert_eq!(asked.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn refuses_an_offer_larger_than_the_free_space() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    let (mut s, _serving) = malicious_session(&pair, cfg).await;
    let huge = 1u64 << 60;
    offer(&mut s, vec![raw_item(0, &["big.bin"], huge)], huge).await;
    expect_declined(&mut s, code::NO_SPACE).await;
}

#[tokio::test]
async fn closes_the_session_when_data_runs_past_the_declared_size() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    let (mut s, serving) = malicious_session(&pair, cfg).await;
    offer(&mut s, vec![raw_item(0, &["a.bin"], 2)], 2).await;
    let frame = s.next_frame().await.unwrap();
    assert!(frame.decode::<Decision>().unwrap().accept);
    let mut payload = wire::file_data_header(0, 0).to_vec();
    payload.extend_from_slice(&[1; 64]);
    wire::write_raw(&mut s.writer, FrameType::FileData, &payload)
        .await
        .unwrap();
    s.flush().await.unwrap();
    assert!(serving.await.unwrap().is_err());
    assert!(!dst.path().join("a.bin").exists());
}

#[tokio::test]
async fn refuses_a_tampered_file_and_publishes_nothing() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    let (mut s, serving) = malicious_session(&pair, cfg).await;
    offer(&mut s, vec![raw_item(0, &["a.bin"], 3)], 3).await;
    s.next_frame().await.unwrap();
    let mut payload = wire::file_data_header(0, 0).to_vec();
    payload.extend_from_slice(b"abc");
    wire::write_raw(&mut s.writer, FrameType::FileData, &payload)
        .await
        .unwrap();
    let seg = wire::SegmentEnd {
        file_index: 0,
        segment: 0,
        hash: [0; 32],
    };
    write_msg(&mut s.writer, FrameType::SegmentEnd, &seg)
        .await
        .unwrap();
    s.flush().await.unwrap();
    assert!(serving.await.unwrap().is_err());
    assert!(!dst.path().join("a.bin").exists());
}

#[tokio::test]
async fn rate_limits_a_flood_of_offers() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let (cfg, _) = config(dst.path(), ReceiveMode::AlwaysAsk, false);
    let (mut s, _serving) = malicious_session(&pair, cfg).await;
    for _ in 0..10 {
        offer(&mut s, vec![raw_item(0, &["a"], 0)], 0).await;
        expect_declined(&mut s, code::DECLINED).await;
    }
    offer(&mut s, vec![raw_item(0, &["a"], 0)], 0).await;
    expect_declined(&mut s, code::RATE_LIMITED).await;
}

#[tokio::test]
async fn closes_the_session_on_an_unknown_mandatory_frame() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    let (mut s, serving) = malicious_session(&pair, cfg).await;
    s.writer.write_all(&[0x7E, 0, 0, 0]).await.unwrap();
    s.flush().await.unwrap();
    assert!(serving.await.unwrap().is_err());
}

// ---- Resume ----

/// Forwards one connection and cuts it after `limit` bytes from the sender,
/// the way a dropped Wi-Fi link would.
async fn cutting_proxy(target: SocketAddr, limit: usize) -> SocketAddr {
    let (front, addr) = listener().await;
    tokio::spawn(async move {
        let (mut inbound, _) = front.accept().await.unwrap();
        let mut outbound = TcpStream::connect(target).await.unwrap();
        let (mut in_r, mut in_w) = inbound.split();
        let (mut out_r, mut out_w) = outbound.split();
        let forward = async {
            let mut buf = vec![0u8; 64 * 1024];
            let mut sent = 0;
            while sent < limit {
                let n = in_r.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                if out_w.write_all(&buf[..n]).await.is_err() {
                    break;
                }
                sent += n;
            }
        };
        let back = tokio::io::copy(&mut out_r, &mut in_w);
        tokio::select! {
            () = forward => {}
            _ = back => {}
        }
    });
    addr
}

struct Interrupted {
    id: [u8; 16],
    src: TempDir,
    dst: TempDir,
    cfg: ReceiveConfig,
    data: Vec<u8>,
    path: std::path::PathBuf,
}

const INTERRUPT_AFTER: usize = SEGMENT_SIZE as usize * 3 + 4096;

async fn interrupted_transfer(pair: &Pair, segments: usize) -> Interrupted {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let size = SEGMENT_SIZE as usize * segments + 777;
    let data: Vec<u8> = (0..size).map(|i| ((i ^ (i >> 9)) % 251) as u8).collect();
    let path = write_file(src.path(), "movie.bin", &data);
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);

    let (l, target) = listener().await;
    let server = pair.host.clone();
    let server_cfg = cfg.clone();
    let serving = tokio::spawn(async move { server.serve_one(&l, &server_cfg).await });
    let cut = cutting_proxy(target, INTERRUPT_AFTER).await;
    let plan = transfer::plan(std::slice::from_ref(&path)).unwrap();
    let id = plan.transfer_id;
    assert!(pair.joiner.send(pair.host_id, cut, plan).await.is_err());
    assert!(serving.await.unwrap().is_err());
    assert!(!dst.path().join("movie.bin").exists());
    Interrupted {
        id,
        src,
        dst,
        cfg,
        data,
        path,
    }
}

async fn resume(pair: &Pair, t: &Interrupted) -> Result<transfer::SendReport, NodeError> {
    let (l, addr) = listener().await;
    let server = pair.host.clone();
    let cfg = t.cfg.clone();
    let serving = tokio::spawn(async move { server.serve_one(&l, &cfg).await });
    let report = pair.joiner.resume(pair.host_id, addr, t.id).await;
    let _ = serving.await;
    report
}

#[tokio::test]
async fn continues_an_interrupted_transfer_and_ends_up_identical() {
    let pair = paired().await;
    let t = interrupted_transfer(&pair, 6).await;
    let report = resume(&pair, &t).await.unwrap();

    assert!(
        report.bytes_sent <= t.data.len() as u64 - 2 * SEGMENT_SIZE,
        "only {} of {} bytes should have been sent again",
        report.bytes_sent,
        t.data.len()
    );
    assert_eq!(
        std::fs::read(t.dst.path().join("movie.bin")).unwrap(),
        t.data
    );
    assert!(!t.dst.path().join(".zerohop-tmp").exists());
}

#[tokio::test]
async fn resumes_from_the_start_when_the_source_changed_without_its_timestamp() {
    let pair = paired().await;
    let t = interrupted_transfer(&pair, 6).await;
    let stamp = std::fs::metadata(&t.path).unwrap().modified().unwrap();
    let mut changed = t.data.clone();
    changed[10] ^= 0xFF;
    std::fs::write(&t.path, &changed).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&t.path)
        .unwrap()
        .set_modified(stamp)
        .unwrap();

    let report = resume(&pair, &t).await.unwrap();
    assert_eq!(report.bytes_sent, t.data.len() as u64);
    assert_eq!(
        std::fs::read(t.dst.path().join("movie.bin")).unwrap(),
        changed
    );
}

#[tokio::test]
async fn refuses_to_resume_when_the_source_visibly_changed() {
    let pair = paired().await;
    let t = interrupted_transfer(&pair, 6).await;
    std::fs::write(&t.path, b"something else").unwrap();

    let result = resume(&pair, &t).await;
    assert!(
        matches!(
            result,
            Err(NodeError::Transfer(TransferError::SourceChanged))
        ),
        "{:?}",
        result.err()
    );
    assert!(!t.dst.path().join("movie.bin").exists());
    assert!(!t.dst.path().join(".zerohop-tmp").exists());
    drop(t.src);
}

#[tokio::test]
async fn a_second_transfer_never_overwrites_the_first() {
    let pair = paired().await;
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    std::fs::write(dst.path().join("note.txt"), b"mine").unwrap();
    let path = write_file(src.path(), "note.txt", b"theirs");
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    send_and_serve(&pair, cfg, vec![path]).await.0.unwrap();
    assert_eq!(std::fs::read(dst.path().join("note.txt")).unwrap(), b"mine");
    assert_eq!(
        std::fs::read(dst.path().join("note (1).txt")).unwrap(),
        b"theirs"
    );
}

// ---- Revocation ----

#[tokio::test]
async fn a_revoked_device_is_told_and_cannot_send() {
    let pair = paired().await;
    lock(pair.host.circle())
        .unwrap()
        .revoke(pair.joiner_id)
        .unwrap();
    let (l, addr) = listener().await;
    let dst = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    let server = pair.host.clone();
    let serving = tokio::spawn(async move { server.serve_one(&l, &cfg).await });
    let path = write_file(src.path(), "x.txt", b"data");
    let plan = transfer::plan(&[path]).unwrap();
    let result = pair.joiner.send(pair.host_id, addr, plan).await;
    serving.await.unwrap().unwrap();
    assert!(
        matches!(result, Err(NodeError::Session(SessionError::Removed))),
        "{:?}",
        result.err()
    );
    assert!(
        !lock(pair.joiner.circle())
            .unwrap()
            .is_member(&pair.joiner_id)
    );
    assert!(!dst.path().join("x.txt").exists());
}

#[tokio::test]
async fn a_new_device_reaches_others_after_the_host_syncs() {
    let pair = paired().await;
    let third_dir = tempfile::tempdir().unwrap();
    let (l, addr) = listener().await;
    let secret = PairingSecret::new_code().unwrap();
    let typed = secret.clone();
    let path = third_dir.path().to_path_buf();
    let joining = tokio::spawn(async move {
        pairing::join(
            addr,
            &nets(),
            &path,
            generate_key().unwrap(),
            &typed,
            HostPin::None,
            info("third"),
            yes,
        )
        .await
    });
    let host = pair.host.clone();
    let hosting = tokio::task::spawn_blocking(move || {
        let handle = tokio::runtime::Handle::current();
        let mut circle = lock(host.circle()).unwrap();
        handle.block_on(pairing::host(
            l,
            &nets(),
            &mut *circle,
            &secret,
            info("host"),
            yes,
        ))
    });
    hosting.await.unwrap().unwrap();
    let third = joining.await.unwrap().unwrap();
    let third_id = third.device_id();

    let (l2, addr2) = listener().await;
    let dst = tempfile::tempdir().unwrap();
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    let server = pair.joiner.clone();
    let serving = tokio::spawn(async move { server.serve_one(&l2, &cfg).await });
    pair.host.sync_with(pair.joiner_id, addr2).await.unwrap();
    serving.await.unwrap().unwrap();
    assert!(lock(pair.joiner.circle()).unwrap().is_member(&third_id));
}

// ---- Found by independent review ----

async fn add_third(
    pair: &Pair,
    name: String,
) -> (
    Result<pairing::PairedDevice, PairingError>,
    Result<Circle, PairingError>,
    TempDir,
) {
    let third_dir = tempfile::tempdir().unwrap();
    let (l, addr) = listener().await;
    let secret = PairingSecret::new_code().unwrap();
    let typed = secret.clone();
    let path = third_dir.path().to_path_buf();
    let joining = tokio::spawn(async move {
        pairing::join(
            addr,
            &nets(),
            &path,
            generate_key().unwrap(),
            &typed,
            HostPin::None,
            info(&name),
            yes,
        )
        .await
    });
    let host = pair.host.clone();
    let hosting = tokio::task::spawn_blocking(move || {
        let handle = tokio::runtime::Handle::current();
        let mut circle = lock(host.circle()).unwrap();
        handle.block_on(pairing::host(
            l,
            &nets(),
            &mut *circle,
            &secret,
            info("host"),
            yes,
        ))
    });
    let hosted = hosting.await.unwrap();
    let joined = joining.await.unwrap();
    (hosted, joined, third_dir)
}

#[tokio::test]
async fn a_decoy_script_does_not_release_a_disguised_program() {
    let pair = paired().await;
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let root = src.path().join("pack");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.py"), b"pass").unwrap();
    std::fs::write(root.join("cat.jpg"), b"MZ\x90\0payload").unwrap();
    let (cfg, asked) = config(dst.path(), ReceiveMode::AutomaticExceptPrograms, true);
    let report = send_and_serve(&pair, cfg, vec![root]).await.0.unwrap();
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert_eq!(report.held, 1);
    assert!(!dst.path().join("pack").exists());
}

#[tokio::test]
async fn a_name_that_grows_when_cleaned_is_refused_and_leaves_the_host_working() {
    let pair = paired().await;
    let name = "\u{958}".repeat(21);
    assert_eq!(name.len(), 63);
    let (hosted, _joined, _dir) = add_third(&pair, name).await;
    assert!(hosted.is_err());
    let mut host = lock(pair.host.circle()).unwrap();
    host.revoke(pair.joiner_id).unwrap();
    assert!(!host.is_member(&pair.joiner_id));
}

#[tokio::test]
async fn a_removal_applied_by_a_failed_handshake_still_ends_live_sessions() {
    use zerohop_core::wire::{Hello, LedgerEvents, LedgerHave};
    let pair = paired().await;
    let (hosted, joined, _dir) = add_third(&pair, "third".to_string()).await;
    hosted.unwrap();
    let mut third = joined.unwrap();
    let third_id = third.device_id();

    let (l, addr) = listener().await;
    let folder = tempfile::tempdir().unwrap();
    let (cfg, _) = config(folder.path(), ReceiveMode::Automatic, true);
    let server = pair.host.clone();
    tokio::spawn(async move { server.serve(l, cfg).await });

    let key = lock(pair.joiner.circle()).unwrap().key().clone();
    let client = tls::client_config(&key, PeerRule::Exact(pair.host_id), ALPN_NORMAL).unwrap();
    let stream = tls::connect(&nets(), addr, client).await.unwrap();
    let members = lock(pair.joiner.circle())
        .unwrap()
        .ledger()
        .membership()
        .members;
    let (tx, rx) = watch::channel(members);
    std::mem::forget(tx);
    let mut live = Session::establish(stream, pair.host_id, pair.joiner.circle(), &local("j"), rx)
        .await
        .unwrap();

    // Another member delivers a removal, then its link drops during round two.
    third.revoke(pair.joiner_id).unwrap();
    let revoke = third
        .ledger()
        .all_events()
        .find(|e| e.body.author == third_id)
        .cloned()
        .unwrap();
    let client =
        tls::client_config(third.key(), PeerRule::Exact(pair.host_id), ALPN_NORMAL).unwrap();
    let mut s = tls::connect(&nets(), addr, client).await.unwrap();
    let hello = Hello {
        software: "x".into(),
        features: 0,
        name: "third".into(),
        platform: Platform::Other,
        circle_id: *third.ledger().circle_id(),
    };
    write_msg(&mut s, FrameType::Hello, &hello).await.unwrap();
    let heads = LedgerHave {
        authors: third.ledger().heads(),
    };
    write_msg(&mut s, FrameType::LedgerHave, &heads)
        .await
        .unwrap();
    let events = LedgerEvents {
        events: vec![revoke],
    };
    write_msg(&mut s, FrameType::LedgerEvents, &events)
        .await
        .unwrap();
    s.flush().await.unwrap();
    for _ in 0..3 {
        wire::read_frame(&mut s).await.unwrap();
    }
    for _ in 0..200 {
        if !lock(pair.host.circle()).unwrap().is_member(&pair.joiner_id) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    drop(s);

    let ended = tokio::time::timeout(std::time::Duration::from_secs(3), live.next_frame()).await;
    assert!(
        matches!(ended, Ok(Err(_))),
        "the removed device's session must end"
    );
}

#[tokio::test]
async fn a_peer_cannot_send_an_endless_sync() {
    use zerohop_core::ledger::AuthorHead;
    use zerohop_core::wire::{Hello, LedgerHave};
    let pair = paired().await;
    let (server_side, mut peer_side) = tokio::io::duplex(1 << 20);
    let circle = pair.host.circle().clone();
    let circle_id = *lock(&circle).unwrap().ledger().circle_id();
    let members = lock(&circle).unwrap().ledger().membership().members;
    let (tx, rx) = watch::channel(members);
    std::mem::forget(tx);
    let peer = pair.joiner_id;
    let task = tokio::spawn(async move {
        Session::establish(server_side, peer, &circle, &local("h"), rx)
            .await
            .map(|_| ())
    });
    let hello = Hello {
        software: "x".into(),
        features: 0,
        name: "e".into(),
        platform: Platform::Other,
        circle_id,
    };
    write_msg(&mut peer_side, FrameType::Hello, &hello)
        .await
        .unwrap();
    peer_side.flush().await.unwrap();
    wire::read_frame(&mut peer_side).await.unwrap();
    let head = AuthorHead {
        author: [9; 32],
        seq: 1,
        chain: [7; 32],
    };
    let msg = LedgerHave {
        authors: vec![head; 2000],
    };
    for _ in 0..7 {
        write_msg(&mut peer_side, FrameType::LedgerHave, &msg)
            .await
            .unwrap();
    }
    peer_side.flush().await.unwrap();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(3), task).await;
    assert!(
        matches!(ended, Ok(Ok(Err(_)))),
        "14000 heads is more than any honest peer has"
    );
}

// ---- Interception and impersonation ----

/// Forwards bytes both ways and keeps a copy of everything it saw.
async fn recording_proxy(target: SocketAddr) -> (SocketAddr, Arc<std::sync::Mutex<Vec<u8>>>) {
    let (front, addr) = listener().await;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        let (mut inbound, _) = front.accept().await.unwrap();
        let mut outbound = TcpStream::connect(target).await.unwrap();
        let (mut in_r, mut in_w) = inbound.split();
        let (mut out_r, mut out_w) = outbound.split();
        let up = async {
            let mut buf = vec![0u8; 64 * 1024];
            while let Ok(n) = in_r.read(&mut buf).await {
                if n == 0 || out_w.write_all(&buf[..n]).await.is_err() {
                    break;
                }
                log.lock().unwrap().extend_from_slice(&buf[..n]);
            }
            let _ = out_w.shutdown().await;
        };
        let down = async {
            let mut buf = vec![0u8; 64 * 1024];
            while let Ok(n) = out_r.read(&mut buf).await {
                if n == 0 || in_w.write_all(&buf[..n]).await.is_err() {
                    break;
                }
                log.lock().unwrap().extend_from_slice(&buf[..n]);
            }
            let _ = in_w.shutdown().await;
        };
        tokio::join!(up, down);
    });
    (addr, seen)
}

#[tokio::test]
async fn someone_recording_the_network_sees_neither_contents_nor_names() {
    let pair = paired().await;
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let marker = b"THE-SECRET-CONTENTS-OF-THIS-FILE".repeat(64);
    let name = "my-private-diary-entry.txt";
    let path = write_file(src.path(), name, &marker);
    let (cfg, _) = config(dst.path(), ReceiveMode::Automatic, true);
    let (l, target) = listener().await;
    let server = pair.host.clone();
    let serving = tokio::spawn(async move { server.serve_one(&l, &cfg).await });
    let (proxy, seen) = recording_proxy(target).await;
    let plan = transfer::plan(&[path]).unwrap();
    pair.joiner.send(pair.host_id, proxy, plan).await.unwrap();
    serving.await.unwrap().unwrap();
    assert_eq!(std::fs::read(dst.path().join(name)).unwrap(), marker);

    let seen = seen.lock().unwrap();
    assert!(
        seen.len() > marker.len(),
        "the proxy must have carried the transfer"
    );
    let found = |needle: &[u8]| seen.windows(needle.len()).any(|w| w == needle);
    assert!(
        !found(&marker[..32]),
        "file contents visible on the network"
    );
    assert!(!found(name.as_bytes()), "file name visible on the network");
    assert!(!found(b"host"), "device name visible on the network");
}

#[tokio::test]
async fn relaying_a_pairing_through_a_middleman_fails_on_both_sides() {
    let host_dir = tempfile::tempdir().unwrap();
    let joiner_dir = tempfile::tempdir().unwrap();
    let mut circle = Circle::create(host_dir.path(), "host").unwrap();
    let (host_listener, host_addr) = listener().await;
    let (fake_listener, fake_addr) = listener().await;
    let secret = PairingSecret::new_code().unwrap();
    let typed = secret.clone();

    // The middleman ends TLS on each side and copies what the two devices say.
    let mallory = generate_key().unwrap();
    let relay = tokio::spawn(async move {
        let inbound_cfg =
            tls::server_config(&mallory, PeerRule::AnyForPairing, ALPN_PAIRING).unwrap();
        let outbound_cfg =
            tls::client_config(&mallory, PeerRule::AnyForPairing, ALPN_PAIRING).unwrap();
        let (tcp, from) = fake_listener.accept().await.unwrap();
        let mut inbound = tls::accept(&nets(), tcp, from, inbound_cfg).await.unwrap();
        let mut outbound = tls::connect(&nets(), host_addr, outbound_cfg)
            .await
            .unwrap();
        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
    });

    let path = joiner_dir.path().to_path_buf();
    let joining = tokio::spawn(async move {
        pairing::join(
            fake_addr,
            &nets(),
            &path,
            generate_key().unwrap(),
            &typed,
            HostPin::None,
            info("joiner"),
            yes,
        )
        .await
    });
    let hosted = pairing::host(
        host_listener,
        &nets(),
        &mut circle,
        &secret,
        info("host"),
        yes,
    )
    .await;
    assert!(hosted.is_err(), "the host must not pair through a relay");
    assert!(
        joining.await.unwrap().is_err(),
        "the joiner must not pair through a relay"
    );
    assert_eq!(circle.ledger().membership().members.len(), 1);
    assert!(!Circle::exists(joiner_dir.path()));
    relay.abort();
}

#[tokio::test]
async fn a_middleman_who_knows_the_code_shows_different_comparison_numbers() {
    let host_dir = tempfile::tempdir().unwrap();
    let joiner_dir = tempfile::tempdir().unwrap();
    let mallory_dir = tempfile::tempdir().unwrap();
    let mallory_host_dir = tempfile::tempdir().unwrap();
    let mut circle = Circle::create(host_dir.path(), "host").unwrap();
    let mut mallory_circle = Circle::create(mallory_host_dir.path(), "host").unwrap();
    let (host_listener, host_addr) = listener().await;
    let (fake_listener, fake_addr) = listener().await;
    let secret = PairingSecret::from_typed("424242");

    // Each honest device talks to the middleman, which runs a full pairing with each.
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let record = |log: Arc<std::sync::Mutex<Vec<String>>>| {
        move |p: PairingPrompt| {
            log.lock().unwrap().push(p.comparison);
            async { false }
        }
    };

    let (s1, s2, s3) = (secret.clone(), secret.clone(), secret.clone());
    let (p1, p2) = (
        joiner_dir.path().to_path_buf(),
        mallory_dir.path().to_path_buf(),
    );
    let honest_joiner = tokio::spawn({
        let log = seen.clone();
        async move {
            pairing::join(
                fake_addr,
                &nets(),
                &p1,
                generate_key().unwrap(),
                &s1,
                HostPin::None,
                info("joiner"),
                record(log),
            )
            .await
        }
    });
    let mallory_joins = tokio::spawn(async move {
        pairing::join(
            host_addr,
            &nets(),
            &p2,
            generate_key().unwrap(),
            &s2,
            HostPin::None,
            info("joiner"),
            yes,
        )
        .await
    });
    let networks = nets();
    let (_, _) = tokio::join!(
        pairing::host(
            fake_listener,
            &networks,
            &mut mallory_circle,
            &s3,
            info("host"),
            yes
        ),
        pairing::host(
            host_listener,
            &networks,
            &mut circle,
            &secret,
            info("host"),
            record(seen.clone())
        ),
    );
    let _ = honest_joiner.await;
    let _ = mallory_joins.await;

    let numbers = seen.lock().unwrap().clone();
    assert_eq!(numbers.len(), 2, "each honest device must have been asked");
    assert_ne!(
        numbers[0], numbers[1],
        "an owner comparing the numbers would see the attack"
    );
    assert_eq!(circle.ledger().membership().members.len(), 1);
}

#[tokio::test]
async fn the_pairing_listener_refuses_the_normal_protocol() {
    let host_dir = tempfile::tempdir().unwrap();
    let mut circle = Circle::create(host_dir.path(), "host").unwrap();
    let (l, addr) = listener().await;
    let secret = PairingSecret::new_code().unwrap();
    let key = generate_key().unwrap();
    let client_cfg = tls::client_config(&key, PeerRule::AnyForPairing, ALPN_NORMAL).unwrap();
    let networks = nets();
    let hosting = pairing::host(l, &networks, &mut circle, &secret, info("host"), yes);
    let result = tokio::select! {
        r = async {
            let r = tls::connect(&nets(), addr, client_cfg).await;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            r
        } => r,
        _ = hosting => panic!("pairing must keep waiting"),
    };
    assert!(result.is_err(), "ALPN mismatch must fail the handshake");
    assert_eq!(circle.ledger().membership().members.len(), 1);
}

#[tokio::test]
async fn pairing_frames_in_the_wrong_order_add_nobody_and_do_not_spend_the_code() {
    use zerohop_core::wire::{Confirm, PakeA};
    let host_dir = tempfile::tempdir().unwrap();
    let joiner_dir = tempfile::tempdir().unwrap();
    let mut circle = Circle::create(host_dir.path(), "host").unwrap();
    let (l, addr) = listener().await;
    let secret = PairingSecret::new_code().unwrap();
    let typed = secret.clone();
    let path = joiner_dir.path().to_path_buf();
    let visitors = tokio::spawn(async move {
        for attempt in 0..2 {
            let key = generate_key().unwrap();
            let cfg = tls::client_config(&key, PeerRule::AnyForPairing, ALPN_PAIRING).unwrap();
            let mut s = tls::connect(&nets(), addr, cfg).await.unwrap();
            if attempt == 0 {
                write_msg(&mut s, FrameType::Confirm, &Confirm { tag: [1; 32] })
                    .await
                    .unwrap();
            } else {
                write_msg(&mut s, FrameType::Hello, &PakeA { msg_a: [2; 33] })
                    .await
                    .unwrap();
            }
            s.flush().await.unwrap();
            let _ = wire::read_frame(&mut s).await;
        }
        pairing::join(
            addr,
            &nets(),
            &path,
            generate_key().unwrap(),
            &typed,
            HostPin::None,
            info("joiner"),
            yes,
        )
        .await
    });
    let hosted = pairing::host(l, &nets(), &mut circle, &secret, info("host"), yes).await;
    assert!(hosted.is_ok(), "{:?}", hosted.err());
    assert!(visitors.await.unwrap().is_ok());
    assert_eq!(circle.ledger().membership().members.len(), 2);
}

#[tokio::test]
async fn a_member_cannot_forge_a_removal_in_another_members_name() {
    use zerohop_core::ledger::{EventKind, sign_event};
    let pair = paired().await;
    let mut host = lock(pair.host.circle()).unwrap();
    let joiner_key = lock(pair.joiner.circle()).unwrap().key().clone();
    let circle_id = *host.ledger().circle_id();
    let mut forged = sign_event(
        &joiner_key,
        &circle_id,
        5,
        EventKind::Revoke {
            device: pair.joiner_id,
            last_seen_seq: 0,
        },
    )
    .unwrap();
    forged.body.author = pair.host_id;
    assert!(host.apply(vec![forged]).is_err());
    assert!(host.is_member(&pair.joiner_id));
}

#[tokio::test]
async fn membership_updates_about_others_do_not_end_a_session() {
    let pair = paired().await;
    let (a, b) = tokio::io::duplex(1 << 20);
    let members = lock(pair.host.circle())
        .unwrap()
        .ledger()
        .membership()
        .members;
    let (tx, rx) = watch::channel(members.clone());
    let (_tx2, rx2) = watch::channel(members.clone());
    let (host_info, joiner_info) = (local("h"), local("j"));
    let (ours, _theirs) = tokio::join!(
        Session::establish(a, pair.joiner_id, pair.host.circle(), &host_info, rx),
        Session::establish(b, pair.host_id, pair.joiner.circle(), &joiner_info, rx2)
    );
    let mut ours = ours.unwrap();
    let waiting = tokio::spawn(async move { ours.next_frame().await.map(|_| ()) });
    for _ in 0..5 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        tx.send_replace(members.clone());
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!waiting.is_finished(), "the session must keep running");
    waiting.abort();
}

// ---- Things the interface relies on ----

/// Counts what the owner would be told about one transfer.
#[derive(Default)]
struct Watcher {
    started: Mutex<Vec<ReceiveStart>>,
    control: Mutex<Option<Control>>,
    finished: AtomicUsize,
    interrupted: Mutex<Vec<bool>>,
    asked: AtomicUsize,
    /// When set, the owner never answers.
    silent: bool,
}

impl Approver for Watcher {
    fn approve(&self, _: OfferSummary) -> Pin<Box<dyn Future<Output = bool> + Send>> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        if self.silent {
            Box::pin(std::future::pending())
        } else {
            Box::pin(async { true })
        }
    }

    fn receiving(&self, start: &ReceiveStart) -> Control {
        self.started.lock().unwrap().push(start.clone());
        let control = Control::new();
        *self.control.lock().unwrap() = Some(control.clone());
        control
    }

    fn finished(&self, _: &DeviceId, _: &[u8; 16], _: &OfferOutcome) {
        self.finished.fetch_add(1, Ordering::SeqCst);
    }

    fn interrupted(&self, _: &DeviceId, _: &[u8; 16], why: Option<&TransferError>) {
        let cancelled = matches!(why, Some(TransferError::Cancelled(_)));
        self.interrupted.lock().unwrap().push(cancelled);
    }
}

fn watched(folder: &Path, mode: ReceiveMode, watcher: Arc<Watcher>) -> ReceiveConfig {
    ReceiveConfig {
        settings: ReceiveSettings::new(ReceiveFolder::open(folder).unwrap(), mode),
        approver: watcher,
        auto_limit: DEFAULT_AUTO_LIMIT,
        budgets: Arc::new(Budgets::default()),
        clip: None,
    }
}

fn tmp_entries(folder: &Path) -> usize {
    std::fs::read_dir(folder.join(".zerohop-tmp")).map_or(0, |d| d.count())
}

async fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    for _ in 0..500 {
        if ready() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("timed out waiting: {what}");
}

#[tokio::test]
async fn a_session_exchanges_discovery_keys() {
    let pair = paired().await;
    let recv = tempfile::tempdir().unwrap();
    let (cfg, _) = config(recv.path(), ReceiveMode::Automatic, true);
    let (l, addr) = listener().await;
    let target = pair.host.clone();
    tokio::spawn(async move { target.serve(l, cfg).await });

    pair.joiner.sync_with(pair.host_id, addr).await.unwrap();

    let host_key = lock(pair.host.circle()).unwrap().own_discovery_key().key;
    let joiner_key = lock(pair.joiner.circle()).unwrap().own_discovery_key().key;
    let learned = lock(pair.joiner.circle()).unwrap().peer_discovery_keys();
    assert_eq!(learned.get(&pair.host_id), Some(&host_key));
    wait_until("the host to learn the key", || {
        lock(pair.host.circle())
            .unwrap()
            .peer_discovery_keys()
            .get(&pair.joiner_id)
            == Some(&joiner_key)
    })
    .await;
}

#[tokio::test]
async fn a_device_set_to_refuse_cannot_deliver_anything() {
    let pair = paired().await;
    let (src, recv) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let path = write_file(src.path(), "a.txt", b"hello");
    let watcher = Arc::new(Watcher::default());
    let cfg = watched(recv.path(), ReceiveMode::Automatic, watcher.clone());
    cfg.settings
        .set_device_mode(pair.joiner_id, Some(ReceiveMode::Refuse));
    let (l, addr) = listener().await;
    let target = pair.host.clone();
    tokio::spawn(async move { target.serve(l, cfg).await });

    let out = transfer::plan(&[path]).unwrap();
    let result = pair.joiner.send(pair.host_id, addr, out).await;

    assert!(matches!(
        result,
        Err(NodeError::Transfer(TransferError::Declined(code::DECLINED)))
    ));
    assert_eq!(watcher.asked.load(Ordering::SeqCst), 0);
    assert!(!recv.path().join("a.txt").exists());
}

#[tokio::test]
async fn the_mode_of_one_device_does_not_affect_another() {
    let pair = paired().await;
    let (src, recv) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let path = write_file(src.path(), "a.txt", b"hello");
    let watcher = Arc::new(Watcher::default());
    let cfg = watched(recv.path(), ReceiveMode::Automatic, watcher.clone());
    cfg.settings
        .set_device_mode([9; 32], Some(ReceiveMode::Refuse));
    let (l, addr) = listener().await;
    let target = pair.host.clone();
    tokio::spawn(async move { target.serve(l, cfg).await });

    let out = transfer::plan(&[path]).unwrap();
    pair.joiner.send(pair.host_id, addr, out).await.unwrap();
    assert!(recv.path().join("a.txt").exists());
}

#[tokio::test]
async fn progress_reaches_the_total_on_both_sides() {
    let pair = paired().await;
    let (src, recv) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let data = vec![7u8; 3 * 1024 * 1024];
    let path = write_file(src.path(), "a.bin", &data);
    let watcher = Arc::new(Watcher::default());
    let cfg = watched(recv.path(), ReceiveMode::Automatic, watcher.clone());
    let (l, addr) = listener().await;
    let target = pair.host.clone();
    tokio::spawn(async move { target.serve(l, cfg).await });

    let control = Control::new();
    let out = transfer::plan(&[path]).unwrap();
    pair.joiner
        .send_with(pair.host_id, addr, out, &control)
        .await
        .unwrap();

    assert_eq!(control.bytes_done(), data.len() as u64);
    wait_until("the receiver to finish", || {
        watcher.finished.load(Ordering::SeqCst) == 1
    })
    .await;
    let start = watcher.started.lock().unwrap()[0].clone();
    assert_eq!(start.total_bytes, data.len() as u64);
    assert_eq!((start.files, start.already), (1, 0));
    assert_eq!(start.names, vec!["a.bin".to_string()]);
    let seen = watcher.control.lock().unwrap().clone().unwrap();
    assert_eq!(seen.bytes_done(), data.len() as u64);
    assert!(watcher.interrupted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancelling_on_the_sender_leaves_nothing_on_either_side() {
    let pair = paired().await;
    let (src, recv) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let path = write_file(src.path(), "big.bin", &vec![1u8; 200 * 1024 * 1024]);
    let watcher = Arc::new(Watcher::default());
    let cfg = watched(recv.path(), ReceiveMode::Automatic, watcher.clone());
    let (l, addr) = listener().await;
    let target = pair.host.clone();
    tokio::spawn(async move { target.serve(l, cfg).await });

    let control = Control::new();
    let stopper = control.clone();
    tokio::spawn(async move {
        while stopper.bytes_done() == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        stopper.cancel();
    });
    let out = transfer::plan(&[path]).unwrap();
    let result = pair
        .joiner
        .send_with(pair.host_id, addr, out, &control)
        .await;

    assert!(matches!(
        result,
        Err(NodeError::Transfer(TransferError::Cancelled(
            code::CANCELLED
        )))
    ));
    wait_until("the receiver to clean up", || {
        !watcher.interrupted.lock().unwrap().is_empty()
    })
    .await;
    assert_eq!(watcher.interrupted.lock().unwrap().clone(), vec![true]);
    assert_eq!(tmp_entries(recv.path()), 0);
    assert!(!recv.path().join("big.bin").exists());
    assert!(
        transfer::unfinished(pair._joiner_dir.path())
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn cancelling_on_the_receiver_tells_the_sender() {
    let pair = paired().await;
    let (src, recv) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let path = write_file(src.path(), "big.bin", &vec![1u8; 200 * 1024 * 1024]);
    let watcher = Arc::new(Watcher::default());
    let cfg = watched(recv.path(), ReceiveMode::Automatic, watcher.clone());
    let (l, addr) = listener().await;
    let target = pair.host.clone();
    tokio::spawn(async move { target.serve(l, cfg).await });

    let stopper = watcher.clone();
    tokio::spawn(async move {
        loop {
            let control = stopper.control.lock().unwrap().clone();
            if let Some(control) = control
                && control.bytes_done() > 0
            {
                control.cancel();
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    });
    let out = transfer::plan(&[path]).unwrap();
    let result = pair.joiner.send(pair.host_id, addr, out).await;

    assert!(matches!(
        result,
        Err(NodeError::Transfer(TransferError::Cancelled(
            code::CANCELLED
        )))
    ));
    assert_eq!(tmp_entries(recv.path()), 0);
    assert!(
        transfer::unfinished(pair._joiner_dir.path())
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn withdrawing_an_offer_ends_the_question_without_breaking_the_protocol() {
    let pair = paired().await;
    let (src, recv) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let path = write_file(src.path(), "a.txt", b"hello");
    let watcher = Arc::new(Watcher {
        silent: true,
        ..Watcher::default()
    });
    let cfg = watched(recv.path(), ReceiveMode::AlwaysAsk, watcher.clone());
    let (l, addr) = listener().await;
    let target = pair.host.clone();
    let served = tokio::spawn(async move { target.serve_one(&l, &cfg).await });

    let control = Control::new();
    let stopper = control.clone();
    let asked = watcher.clone();
    tokio::spawn(async move {
        while asked.asked.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        stopper.cancel();
    });
    let out = transfer::plan(&[path]).unwrap();
    let result = pair
        .joiner
        .send_with(pair.host_id, addr, out, &control)
        .await;

    assert!(matches!(
        result,
        Err(NodeError::Transfer(TransferError::Cancelled(
            code::CANCELLED
        )))
    ));
    let outcomes = served.await.unwrap().unwrap();
    assert_eq!(outcomes, vec![OfferOutcome::Declined(code::CANCELLED)]);
    assert_eq!(watcher.finished.load(Ordering::SeqCst), 1);
    assert!(watcher.started.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_dropped_connection_is_reported_as_interrupted() {
    let pair = paired().await;
    let (src, recv) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let path = write_file(src.path(), "big.bin", &vec![1u8; 64 * 1024 * 1024]);
    let watcher = Arc::new(Watcher::default());
    let cfg = watched(recv.path(), ReceiveMode::Automatic, watcher.clone());
    let (l, addr) = listener().await;
    let target = pair.host.clone();
    tokio::spawn(async move { target.serve(l, cfg).await });
    let cut = cutting_proxy(addr, 4 * 1024 * 1024).await;

    let out = transfer::plan(&[path]).unwrap();
    assert!(pair.joiner.send(pair.host_id, cut, out).await.is_err());

    wait_until("the receiver to notice", || {
        !watcher.interrupted.lock().unwrap().is_empty()
    })
    .await;
    assert_eq!(watcher.interrupted.lock().unwrap().clone(), vec![false]);
}

/// Receives one program disguised as a photo, which is then held back.
async fn receive_held(recv: &Path) -> Pair {
    let pair = paired().await;
    let src = tempfile::tempdir().unwrap();
    let path = write_file(src.path(), "cat.jpg", b"MZ-this-is-a-program");
    let (cfg, _) = config(recv, ReceiveMode::AlwaysAsk, true);
    let (l, addr) = listener().await;
    let target = pair.host.clone();
    tokio::spawn(async move { target.serve(l, cfg).await });
    let out = transfer::plan(&[path]).unwrap();
    let report = pair.joiner.send(pair.host_id, addr, out).await.unwrap();
    assert_eq!(report.held, 1);
    pair
}

#[tokio::test]
async fn a_held_item_is_listed_and_can_be_let_in() {
    let recv = tempfile::tempdir().unwrap();
    let pair = receive_held(recv.path()).await;
    let folder = ReceiveFolder::open(recv.path()).unwrap();

    let held = zerohop_core::receive::list_held(&folder).unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].name, "cat.jpg");
    assert_eq!(held[0].peer, pair.joiner_id);
    assert_eq!(held[0].bytes, 20);
    assert!(!recv.path().join("cat.jpg").exists());

    zerohop_core::receive::release_held(&folder, &held[0].transfer_id, "cat.jpg").unwrap();

    assert_eq!(
        std::fs::read(recv.path().join("cat.jpg")).unwrap(),
        b"MZ-this-is-a-program"
    );
    #[cfg(target_os = "macos")]
    assert!(
        xattr::get(recv.path().join("cat.jpg"), "com.apple.quarantine")
            .unwrap()
            .is_some(),
        "an approved item still carries its download mark"
    );
    assert!(
        zerohop_core::receive::list_held(&folder)
            .unwrap()
            .is_empty()
    );
    assert_eq!(tmp_entries(recv.path()), 0);
}

#[tokio::test]
async fn a_held_item_can_be_thrown_away() {
    let recv = tempfile::tempdir().unwrap();
    let _pair = receive_held(recv.path()).await;
    let folder = ReceiveFolder::open(recv.path()).unwrap();
    let held = zerohop_core::receive::list_held(&folder).unwrap();

    zerohop_core::receive::discard_held(&folder, &held[0].transfer_id, "cat.jpg").unwrap();

    assert!(!recv.path().join("cat.jpg").exists());
    assert!(
        zerohop_core::receive::list_held(&folder)
            .unwrap()
            .is_empty()
    );
    assert_eq!(tmp_entries(recv.path()), 0);
}

#[tokio::test]
async fn a_held_item_that_changed_on_disk_is_not_let_in() {
    let recv = tempfile::tempdir().unwrap();
    let _pair = receive_held(recv.path()).await;
    let folder = ReceiveFolder::open(recv.path()).unwrap();
    let held = zerohop_core::receive::list_held(&folder).unwrap();
    let id = data_encoding::HEXLOWER.encode(&held[0].transfer_id);
    let stored = recv
        .path()
        .join(".zerohop-tmp")
        .join(id)
        .join("files/0/cat.jpg");
    std::fs::write(&stored, b"MZ-something-else!!!").unwrap();

    assert!(zerohop_core::receive::release_held(&folder, &held[0].transfer_id, "cat.jpg").is_err());
    assert!(!recv.path().join("cat.jpg").exists());
}

#[tokio::test]
async fn an_expired_held_item_is_swept_away_and_a_recent_one_is_not() {
    let recv = tempfile::tempdir().unwrap();
    let _pair = receive_held(recv.path()).await;
    let folder = ReceiveFolder::open(recv.path()).unwrap();

    let day = std::time::Duration::from_secs(24 * 60 * 60);
    assert_eq!(
        zerohop_core::receive::sweep_expired(&folder, day).unwrap(),
        0
    );
    assert_eq!(zerohop_core::receive::list_held(&folder).unwrap().len(), 1);

    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let moment = std::time::Duration::from_millis(1);
    assert_eq!(
        zerohop_core::receive::sweep_expired(&folder, moment).unwrap(),
        1
    );
    assert!(
        zerohop_core::receive::list_held(&folder)
            .unwrap()
            .is_empty()
    );
    assert_eq!(tmp_entries(recv.path()), 0);
}

#[tokio::test]
async fn pairing_works_while_the_device_keeps_serving() {
    let dirs = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let circle = Circle::create(dirs.0.path(), "host").unwrap();
    let node = Node::new(
        circle,
        nets(),
        LocalInfo {
            name: "host".into(),
            platform: Platform::Other,
        },
        dirs.0.path(),
    );
    let recv = tempfile::tempdir().unwrap();
    let (cfg, _) = config(recv.path(), ReceiveMode::Automatic, true);
    let (l, addr) = listener().await;
    let serving = node.clone();
    tokio::spawn(async move { serving.serve(l, cfg).await });

    let (pair_listener, pair_addr) = listener().await;
    let secret = PairingSecret::new_code().unwrap();
    let typed = PairingSecret::from_typed(secret.display_code().unwrap());
    let joiner_dir = dirs.1.path().to_path_buf();
    let joining = tokio::spawn(async move {
        pairing::join(
            pair_addr,
            &nets(),
            &joiner_dir,
            generate_key().unwrap(),
            &typed,
            HostPin::None,
            info("phone"),
            yes,
        )
        .await
    });
    let handle = node.circle().clone();
    let paired = pairing::host(pair_listener, &nets(), &handle, &secret, info("host"), yes)
        .await
        .unwrap();
    let joined = joining.await.unwrap().unwrap();
    assert!(lock(node.circle()).unwrap().is_member(&paired.device));

    let client = Node::new(
        joined,
        nets(),
        LocalInfo {
            name: "phone".into(),
            platform: Platform::Other,
        },
        dirs.1.path(),
    );
    let host_id = lock(node.circle()).unwrap().device_id();
    client.sync_with(host_id, addr).await.unwrap();
}

// ---- The shared clipboard ----

type TakenClip = (Vec<(ClipFormat, Vec<u8>)>, Duration);

#[derive(Default)]
struct ClipBox {
    refuse: Vec<ClipFormat>,
    got: Mutex<Vec<TakenClip>>,
}

impl ClipBox {
    fn clips(&self) -> Vec<TakenClip> {
        self.got.lock().unwrap().clone()
    }
}

impl ClipSink for ClipBox {
    fn accepts(&self, _: &DeviceId, format: ClipFormat, _: u64) -> bool {
        !self.refuse.contains(&format)
    }

    fn deliver(&self, _: &DeviceId, clip: ReceivedClip) {
        let parts = clip
            .parts
            .iter()
            .map(|p| (p.format, p.bytes.to_vec()))
            .collect();
        self.got.lock().unwrap().push((parts, clip.ttl));
    }
}

fn clip_config(folder: &Path, sink: Option<Arc<ClipBox>>) -> ReceiveConfig {
    let (mut cfg, _) = config(folder, ReceiveMode::Automatic, true);
    cfg.clip = sink.map(|s| s as Arc<dyn ClipSink>);
    cfg
}

fn outgoing(parts: Vec<(ClipFormat, Vec<u8>)>) -> OutgoingClip {
    let parts = parts
        .into_iter()
        .map(|(format, bytes)| ClipPart {
            format,
            bytes: Zeroizing::new(bytes),
        })
        .collect();
    OutgoingClip::new(parts, MAX_CLIP_TTL).unwrap()
}

/// A real sender and a real receiver; returns what the receiver's session reported.
async fn exchange_clip(pair: &Pair, cfg: ReceiveConfig, clip: &OutgoingClip) {
    let (l, addr) = listener().await;
    let server = pair.host.clone();
    let serving = tokio::spawn(async move { server.serve_one(&l, &cfg).await });
    pair.joiner
        .send_clip(pair.host_id, addr, clip)
        .await
        .unwrap();
    serving.await.unwrap().unwrap();
}

async fn send_offer(session: &mut Session<tls::ClientStream>, offer: &ClipOffer) {
    write_msg(&mut session.writer, FrameType::ClipOffer, offer)
        .await
        .unwrap();
    session.flush().await.unwrap();
}

fn one_item_offer(size: u64) -> ClipOffer {
    ClipOffer {
        clip_id: [5; 16],
        ttl_ms: 60_000,
        items: vec![ClipItem {
            format: ClipFormat::Png,
            size,
            inline: None,
        }],
    }
}

async fn expect_fetch(session: &mut Session<tls::ClientStream>, item: u32) {
    let frame = session.next_frame().await.unwrap();
    assert_eq!(frame.kind, FrameType::ClipFetch);
    let fetch: ClipFetch = frame.decode().unwrap();
    assert_eq!(fetch.item, item);
    assert_eq!(fetch.clip_id, [5; 16]);
}

async fn expect_violation(session: &mut Session<tls::ClientStream>) {
    assert!(matches!(
        session.next_frame().await,
        Err(SessionError::Remote(code::PROTOCOL_VIOLATION))
    ));
}

/// The session ended quietly: nothing was asked for and nothing was refused out loud.
async fn expect_left_behind(session: &mut Session<tls::ClientStream>) {
    let frame = session.next_frame().await.unwrap();
    assert_eq!(frame.kind, FrameType::Goodbye);
}

async fn write_data(session: &mut Session<tls::ClientStream>, msg: &ClipData) {
    write_msg(&mut session.writer, FrameType::ClipData, msg)
        .await
        .unwrap();
    session.flush().await.unwrap();
}

async fn write_end(session: &mut Session<tls::ClientStream>, msg: &ClipEnd) {
    write_msg(&mut session.writer, FrameType::ClipEnd, msg)
        .await
        .unwrap();
    session.flush().await.unwrap();
}

fn clip_data(offset: u64, bytes: &[u8]) -> ClipData {
    ClipData {
        clip_id: [5; 16],
        item: 0,
        offset,
        bytes: bytes.to_vec(),
    }
}

fn clip_end(bytes: &[u8]) -> ClipEnd {
    ClipEnd {
        clip_id: [5; 16],
        item: 0,
        hash: *blake3::hash(bytes).as_bytes(),
    }
}

#[tokio::test]
async fn a_small_clip_arrives_exactly_as_copied() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let sink = Arc::new(ClipBox::default());
    let cfg = clip_config(dst.path(), Some(sink.clone()));
    let text = "ссылка: https://example.com/?a=1".as_bytes().to_vec();
    let clip = outgoing(vec![(ClipFormat::Text, text.clone())]);
    exchange_clip(&pair, cfg, &clip).await;
    let clips = sink.clips();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].0, vec![(ClipFormat::Text, text)]);
    assert!(clips[0].1 <= MAX_CLIP_TTL);
    assert!(clips[0].1 > Duration::from_secs(100));
}

#[tokio::test]
async fn a_large_picture_is_fetched_and_arrives_intact() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let sink = Arc::new(ClipBox::default());
    let cfg = clip_config(dst.path(), Some(sink.clone()));
    let image: Vec<u8> = (0..3_000_000u32).map(|n| (n % 251) as u8).collect();
    let clip = outgoing(vec![(ClipFormat::Png, image.clone())]);
    exchange_clip(&pair, cfg, &clip).await;
    assert_eq!(sink.clips()[0].0, vec![(ClipFormat::Png, image)]);
}

#[tokio::test]
async fn a_device_without_clipboard_leaves_every_clip_behind() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let cfg = clip_config(dst.path(), None);
    let (mut s, _serving) = malicious_session(&pair, cfg).await;
    send_offer(&mut s, &one_item_offer(5)).await;
    expect_left_behind(&mut s).await;
}

#[tokio::test]
async fn a_refused_format_is_never_fetched() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let sink = Arc::new(ClipBox {
        refuse: vec![ClipFormat::Png],
        ..ClipBox::default()
    });
    let cfg = clip_config(dst.path(), Some(sink.clone()));
    let (mut s, _serving) = malicious_session(&pair, cfg).await;
    let mut offer = one_item_offer(500_000);
    offer.items.insert(
        0,
        ClipItem {
            format: ClipFormat::Text,
            size: 2,
            inline: Some(b"hi".to_vec()),
        },
    );
    send_offer(&mut s, &offer).await;
    expect_left_behind(&mut s).await;
    let clips = sink.clips();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].0, vec![(ClipFormat::Text, b"hi".to_vec())]);
}

#[tokio::test]
async fn nothing_refused_means_nothing_is_delivered() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let sink = Arc::new(ClipBox {
        refuse: vec![ClipFormat::Png],
        ..ClipBox::default()
    });
    let cfg = clip_config(dst.path(), Some(sink.clone()));
    let (mut s, _serving) = malicious_session(&pair, cfg).await;
    send_offer(&mut s, &one_item_offer(5)).await;
    expect_left_behind(&mut s).await;
    assert!(sink.clips().is_empty());
}

#[tokio::test]
async fn offers_outside_the_limits_end_the_session() {
    let item = |size, inline| ClipItem {
        format: ClipFormat::Text,
        size,
        inline,
    };
    let bad = [
        (0, vec![item(1, None)]),
        (120_001, vec![item(1, None)]),
        (1000, Vec::new()),
        (1000, vec![item(0, None)]),
        (1000, vec![item((64 << 20) + 1, None)]),
        (1000, (0..9).map(|_| item(1, None)).collect()),
        (1000, vec![item(3, Some(vec![0, 0]))]),
        (
            1000,
            vec![item(u64::MAX, None), item(u64::MAX, None), item(2, None)],
        ),
        (
            1000,
            vec![item((128 << 10) + 1, Some(vec![0; (128 << 10) + 1]))],
        ),
    ];
    for (ttl_ms, items) in bad {
        let pair = paired().await;
        let dst = tempfile::tempdir().unwrap();
        let sink = Arc::new(ClipBox::default());
        let cfg = clip_config(dst.path(), Some(sink.clone()));
        let (mut s, _serving) = malicious_session(&pair, cfg).await;
        send_offer(
            &mut s,
            &ClipOffer {
                clip_id: [5; 16],
                ttl_ms,
                items,
            },
        )
        .await;
        expect_violation(&mut s).await;
        assert!(sink.clips().is_empty());
    }
}

#[tokio::test]
async fn a_wrong_hash_is_not_delivered() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let sink = Arc::new(ClipBox::default());
    let cfg = clip_config(dst.path(), Some(sink.clone()));
    let (mut s, _serving) = malicious_session(&pair, cfg).await;
    send_offer(&mut s, &one_item_offer(5)).await;
    expect_fetch(&mut s, 0).await;
    write_data(&mut s, &clip_data(0, b"hello")).await;
    write_end(&mut s, &clip_end(b"HELLO")).await;
    expect_violation(&mut s).await;
    assert!(sink.clips().is_empty());
}

#[tokio::test]
async fn a_clip_cut_short_is_not_delivered() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let sink = Arc::new(ClipBox::default());
    let cfg = clip_config(dst.path(), Some(sink.clone()));
    let (mut s, _serving) = malicious_session(&pair, cfg).await;
    send_offer(&mut s, &one_item_offer(5)).await;
    expect_fetch(&mut s, 0).await;
    write_data(&mut s, &clip_data(0, b"hel")).await;
    write_end(&mut s, &clip_end(b"hel")).await;
    expect_violation(&mut s).await;
    assert!(sink.clips().is_empty());
}

#[tokio::test]
async fn chunks_that_lie_about_their_place_end_the_session() {
    let mut wrong_id = clip_data(0, b"he");
    wrong_id.clip_id = [9; 16];
    let mut wrong_item = clip_data(0, b"he");
    wrong_item.item = 7;
    let cases = [
        clip_data(1, b"he"),
        clip_data(u64::MAX, b"he"),
        clip_data(0, b"too long"),
        clip_data(0, b""),
        wrong_id,
        wrong_item,
    ];
    for case in cases {
        let pair = paired().await;
        let dst = tempfile::tempdir().unwrap();
        let sink = Arc::new(ClipBox::default());
        let cfg = clip_config(dst.path(), Some(sink.clone()));
        let (mut s, _serving) = malicious_session(&pair, cfg).await;
        send_offer(&mut s, &one_item_offer(5)).await;
        expect_fetch(&mut s, 0).await;
        write_data(&mut s, &case).await;
        expect_violation(&mut s).await;
        assert!(sink.clips().is_empty());
    }
}

#[tokio::test]
async fn an_end_for_another_clip_is_refused() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let sink = Arc::new(ClipBox::default());
    let cfg = clip_config(dst.path(), Some(sink.clone()));
    let (mut s, _serving) = malicious_session(&pair, cfg).await;
    send_offer(&mut s, &one_item_offer(5)).await;
    expect_fetch(&mut s, 0).await;
    write_data(&mut s, &clip_data(0, b"hello")).await;
    write_end(
        &mut s,
        &ClipEnd {
            clip_id: [8; 16],
            ..clip_end(b"hello")
        },
    )
    .await;
    expect_violation(&mut s).await;
    assert!(sink.clips().is_empty());
}

#[tokio::test]
async fn other_frames_during_a_fetch_end_the_session() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let sink = Arc::new(ClipBox::default());
    let cfg = clip_config(dst.path(), Some(sink.clone()));
    let (mut s, _serving) = malicious_session(&pair, cfg).await;
    send_offer(&mut s, &one_item_offer(5)).await;
    expect_fetch(&mut s, 0).await;
    offer(&mut s, vec![raw_item(0, &["a.txt"], 1)], 1).await;
    expect_violation(&mut s).await;
    assert!(sink.clips().is_empty());
}

#[tokio::test]
async fn a_clip_that_outlives_its_lifetime_is_dropped() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let sink = Arc::new(ClipBox::default());
    let cfg = clip_config(dst.path(), Some(sink.clone()));
    let (mut s, serving) = malicious_session(&pair, cfg).await;
    let mut offer = one_item_offer(5);
    offer.ttl_ms = 50;
    send_offer(&mut s, &offer).await;
    expect_fetch(&mut s, 0).await;
    let outcome = serving.await.unwrap();
    assert!(matches!(
        outcome,
        Err(NodeError::Transfer(TransferError::Session(
            SessionError::Timeout
        )))
    ));
    assert!(sink.clips().is_empty());
}

#[tokio::test]
async fn too_many_clips_in_a_minute_are_left_behind() {
    let pair = paired().await;
    let dst = tempfile::tempdir().unwrap();
    let sink = Arc::new(ClipBox::default());
    let cfg = clip_config(dst.path(), Some(sink.clone()));
    for n in 0u8..31 {
        let clip = outgoing(vec![(ClipFormat::Text, vec![n + 1])]);
        exchange_clip(&pair, cfg.clone(), &clip).await;
    }
    assert_eq!(sink.clips().len(), 30);
}

/// A dishonest receiver: answers the sender's offer with what `answer` builds.
async fn dishonest_receiver(pair: &Pair, answer: fn(&ClipOffer) -> ClipFetch) -> NodeError {
    let (l, addr) = listener().await;
    let host = pair.host.clone();
    let joiner_id = pair.joiner_id;
    let serving = tokio::spawn(async move {
        let key = lock(host.circle()).unwrap().key().clone();
        let rule = PeerRule::Exact(joiner_id);
        let config = tls::server_config(&key, rule, ALPN_NORMAL).unwrap();
        let (tcp, from) = l.accept().await.unwrap();
        let stream = tls::accept(&nets(), tcp, from, config).await.unwrap();
        let members = lock(host.circle()).unwrap().ledger().membership().members;
        let (tx, rx) = watch::channel(members);
        std::mem::forget(tx);
        let mut session = Session::establish(stream, joiner_id, host.circle(), &local("evil"), rx)
            .await
            .unwrap();
        let frame = session.next_frame().await.unwrap();
        assert_eq!(frame.kind, FrameType::ClipOffer);
        let offered: ClipOffer = frame.decode().unwrap();
        write_msg(&mut session.writer, FrameType::ClipFetch, &answer(&offered))
            .await
            .unwrap();
        session.flush().await.unwrap();
        // Stay until the sender has reacted.
        while session.next_frame().await.is_ok() {}
    });
    let clip = outgoing(vec![
        (ClipFormat::Text, b"small".to_vec()),
        (ClipFormat::Png, vec![1; 300_000]),
    ]);
    let outcome = pair.joiner.send_clip(pair.host_id, addr, &clip).await;
    serving.abort();
    outcome.expect_err("the sender must refuse")
}

#[tokio::test]
async fn the_sender_refuses_an_item_it_never_offered() {
    let pair = paired().await;
    let error = dishonest_receiver(&pair, |offer| ClipFetch {
        clip_id: offer.clip_id,
        item: 9,
    })
    .await;
    assert!(matches!(
        error,
        NodeError::Transfer(TransferError::Session(SessionError::Protocol))
    ));
}

#[tokio::test]
async fn the_sender_refuses_another_clips_id() {
    let pair = paired().await;
    let error = dishonest_receiver(&pair, |_| ClipFetch {
        clip_id: [0; 16],
        item: 1,
    })
    .await;
    assert!(matches!(
        error,
        NodeError::Transfer(TransferError::Session(SessionError::Protocol))
    ));
}

#[tokio::test]
async fn the_sender_refuses_to_fetch_what_was_already_inside_the_offer() {
    let pair = paired().await;
    let error = dishonest_receiver(&pair, |offer| ClipFetch {
        clip_id: offer.clip_id,
        item: 0,
    })
    .await;
    assert!(matches!(
        error,
        NodeError::Transfer(TransferError::Session(SessionError::Protocol))
    ));
}
