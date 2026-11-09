//! The TLS 1.3 profile: raw Ed25519 keys and hybrid post-quantum key exchange
//! (protocol sections 3 and 7).
//!
//! There is deliberately one key exchange group and no plaintext path, so
//! nothing exists to downgrade to.

use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{AlwaysResolvesClientRawPublicKeys, Resumption};
use rustls::crypto::{
    CryptoProvider, WebPkiSupportedAlgorithms, aws_lc_rs, verify_tls13_signature_with_raw_key,
};
use rustls::pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, SubjectPublicKeyInfoDer,
    UnixTime,
};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::{AlwaysResolvesServerRawPublicKeys, NoServerSessionStorage};
use rustls::sign::CertifiedKey;
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, DistinguishedName, Error, ServerConfig,
    SignatureScheme,
};
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};
use zeroize::Zeroizing;

use crate::address::LocalNetworks;
use crate::ledger::DeviceId;

/// ALPN for sessions between members.
pub const ALPN_NORMAL: &[u8] = b"zerohop/1";
/// ALPN for pairing.
pub const ALPN_PAIRING: &[u8] = b"zerohop-pair/1";
/// How long a TCP connection or TLS handshake may take.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Most concurrent connections that have not finished authenticating.
pub const MAX_UNAUTHENTICATED: usize = 16;

const SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2A, 0x30, 0x05, 0x06, 0x03, 0x2B, 0x65, 0x70, 0x03, 0x21, 0x00,
];
const PKCS8_PREFIX: [u8; 16] = [
    0x30, 0x2E, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2B, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];
// The name is never sent, because SNI is disabled, but the API requires one.
const SERVER_NAME: &str = "zerohop.invalid";

/// Errors from establishing a connection.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    /// The address is not on the local network.
    #[error("address is not on the local network")]
    NotLocal,
    /// The handshake or connection failed.
    #[error("connection failed: {0}")]
    Io(#[from] std::io::Error),
    /// The TLS configuration could not be built.
    #[error("TLS configuration failed: {0}")]
    Config(#[from] Error),
    /// The connection took too long.
    #[error("connection timed out")]
    Timeout,
    /// The peer did not present a usable key.
    #[error("the device at this address is not the one expected")]
    BadPeerKey,
}

/// DER-encoded `SubjectPublicKeyInfo` for a device ID.
#[must_use]
pub fn spki_of(id: &DeviceId) -> Vec<u8> {
    [SPKI_PREFIX.as_slice(), id.as_slice()].concat()
}

/// Extracts a device ID from a DER-encoded `SubjectPublicKeyInfo`.
#[must_use]
pub fn id_from_spki(spki: &[u8]) -> Option<DeviceId> {
    spki.strip_prefix(SPKI_PREFIX.as_slice())?.try_into().ok()
}

/// Which peers a connection will accept.
#[derive(Clone)]
pub enum PeerRule {
    /// Exactly this device. Used by clients.
    Exact(DeviceId),
    /// Any device this predicate accepts. Used by the normal server.
    Matching(Arc<dyn Fn(&DeviceId) -> bool + Send + Sync>),
    /// Any key at all. Used only during pairing, which authenticates afterwards.
    AnyForPairing,
}

impl fmt::Debug for PeerRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Exact(_) => "Exact",
            Self::Matching(_) => "Matching",
            Self::AnyForPairing => "AnyForPairing",
        })
    }
}

fn provider() -> Arc<CryptoProvider> {
    let mut p = aws_lc_rs::default_provider();
    p.kx_groups = vec![aws_lc_rs::kx_group::X25519MLKEM768];
    p.cipher_suites = vec![
        aws_lc_rs::cipher_suite::TLS13_AES_128_GCM_SHA256,
        aws_lc_rs::cipher_suite::TLS13_CHACHA20_POLY1305_SHA256,
    ];
    Arc::new(p)
}

fn certified_key(key: &SigningKey, p: &CryptoProvider) -> Result<Arc<CertifiedKey>, TlsError> {
    let der = Zeroizing::new([PKCS8_PREFIX.as_slice(), key.to_bytes().as_slice()].concat());
    let signer =
        p.key_provider
            .load_private_key(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                der.as_slice().to_vec(),
            )))?;
    let spki = spki_of(&key.verifying_key().to_bytes());
    Ok(Arc::new(CertifiedKey::new(
        vec![CertificateDer::from(spki)],
        signer,
    )))
}

#[derive(Debug)]
struct Verifier {
    rule: PeerRule,
    algs: WebPkiSupportedAlgorithms,
}

impl Verifier {
    fn check(&self, key: &CertificateDer<'_>) -> Result<(), Error> {
        let rejected =
            || Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure);
        let id = id_from_spki(key.as_ref()).ok_or_else(rejected)?;
        let ok = match &self.rule {
            PeerRule::Exact(want) => *want == id,
            PeerRule::Matching(f) => f(&id),
            PeerRule::AnyForPairing => true,
        };
        if ok { Ok(()) } else { Err(rejected()) }
    }

    fn signature(
        &self,
        msg: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature_with_raw_key(
            msg,
            &SubjectPublicKeyInfoDer::from(cert.as_ref()),
            dss,
            &self.algs,
        )
    }
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        self.check(end_entity)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TLS 1.2 is not supported".into()))
    }

    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.signature(m, c, d)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

impl ClientCertVerifier for Verifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        self.check(end_entity)?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TLS 1.2 is not supported".into()))
    }

    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.signature(m, c, d)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

/// Builds a client configuration for one purpose.
///
/// # Errors
/// Returns an error if the configuration cannot be built.
pub fn client_config(
    key: &SigningKey,
    rule: PeerRule,
    alpn: &[u8],
) -> Result<Arc<ClientConfig>, TlsError> {
    client_config_with(provider(), key, rule, alpn)
}

fn client_config_with(
    p: Arc<CryptoProvider>,
    key: &SigningKey,
    rule: PeerRule,
    alpn: &[u8],
) -> Result<Arc<ClientConfig>, TlsError> {
    let verifier = Arc::new(Verifier {
        rule,
        algs: p.signature_verification_algorithms,
    });
    let certified = certified_key(key, &p)?;
    let mut cfg = ClientConfig::builder_with_provider(p)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_cert_resolver(Arc::new(AlwaysResolvesClientRawPublicKeys::new(certified)));
    cfg.alpn_protocols = vec![alpn.to_vec()];
    cfg.enable_sni = false;
    cfg.resumption = Resumption::disabled();
    cfg.enable_early_data = false;
    Ok(Arc::new(cfg))
}

/// Builds a server configuration for one purpose.
///
/// # Errors
/// Returns an error if the configuration cannot be built.
pub fn server_config(
    key: &SigningKey,
    rule: PeerRule,
    alpn: &[u8],
) -> Result<Arc<ServerConfig>, TlsError> {
    server_config_with(provider(), key, rule, alpn)
}

fn server_config_with(
    p: Arc<CryptoProvider>,
    key: &SigningKey,
    rule: PeerRule,
    alpn: &[u8],
) -> Result<Arc<ServerConfig>, TlsError> {
    let verifier = Arc::new(Verifier {
        rule,
        algs: p.signature_verification_algorithms,
    });
    let certified = certified_key(key, &p)?;
    let mut cfg = ServerConfig::builder_with_provider(p)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_cert_resolver(Arc::new(AlwaysResolvesServerRawPublicKeys::new(certified)));
    cfg.alpn_protocols = vec![alpn.to_vec()];
    cfg.session_storage = Arc::new(NoServerSessionStorage {});
    cfg.send_tls13_tickets = 0;
    cfg.max_early_data_size = 0;
    Ok(Arc::new(cfg))
}

/// A client-side TLS connection.
pub type ClientStream = tokio_rustls::client::TlsStream<TcpStream>;
/// A server-side TLS connection.
pub type ServerStream = tokio_rustls::server::TlsStream<TcpStream>;

/// Connects to an address, refusing anything that is not local.
///
/// # Errors
/// Returns an error if the address is not local or the handshake fails.
pub async fn connect(
    nets: &LocalNetworks,
    addr: SocketAddr,
    cfg: Arc<ClientConfig>,
) -> Result<ClientStream, TlsError> {
    if !nets.is_local(addr.ip()) {
        return Err(TlsError::NotLocal);
    }
    tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let tcp = TcpStream::connect(addr).await?;
        tcp.set_nodelay(true)?;
        let name = ServerName::try_from(SERVER_NAME)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        TlsConnector::from(cfg)
            .connect(name, tcp)
            .await
            .map_err(classify_client_error)
    })
    .await
    .map_err(|_| TlsError::Timeout)?
}

fn classify_client_error(error: std::io::Error) -> TlsError {
    let rejected = error
        .get_ref()
        .and_then(|e| e.downcast_ref::<Error>())
        .is_some_and(|e| matches!(e, Error::InvalidCertificate(_)));
    if rejected {
        TlsError::BadPeerKey
    } else {
        TlsError::Io(error)
    }
}

/// Completes the server side of a handshake, refusing non-local peers.
///
/// # Errors
/// Returns an error if the peer is not local or the handshake fails.
pub async fn accept(
    nets: &LocalNetworks,
    tcp: TcpStream,
    peer: SocketAddr,
    cfg: Arc<ServerConfig>,
) -> Result<ServerStream, TlsError> {
    if !nets.is_local(peer.ip()) {
        return Err(TlsError::NotLocal);
    }
    tcp.set_nodelay(true)?;
    tokio::time::timeout(HANDSHAKE_TIMEOUT, TlsAcceptor::from(cfg).accept(tcp))
        .await
        .map_err(|_| TlsError::Timeout)?
        .map_err(TlsError::Io)
}

/// The device ID the peer authenticated with.
///
/// # Errors
/// Returns [`TlsError::BadPeerKey`] if no valid key was presented.
pub fn peer_id(certs: Option<&[CertificateDer<'_>]>) -> Result<DeviceId, TlsError> {
    certs
        .and_then(<[_]>::first)
        .and_then(|c| id_from_spki(c.as_ref()))
        .ok_or(TlsError::BadPeerKey)
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::indexing_slicing,
    clippy::unwrap_used
)]
mod tests {
    use super::*;

    #[cfg(feature = "dev-loopback")]
    mod downgrade {
        use super::*;
        use crate::address::LocalNetworks;
        use crate::circle::generate_key;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        fn nets() -> LocalNetworks {
            LocalNetworks::from_subnets(Vec::new()).with_loopback()
        }

        fn classic_only() -> Arc<CryptoProvider> {
            let mut p = aws_lc_rs::default_provider();
            p.kx_groups = vec![aws_lc_rs::kx_group::X25519];
            p.cipher_suites = vec![aws_lc_rs::cipher_suite::TLS13_AES_128_GCM_SHA256];
            Arc::new(p)
        }

        fn without_the_required_suites() -> Arc<CryptoProvider> {
            let mut p = aws_lc_rs::default_provider();
            p.kx_groups = vec![aws_lc_rs::kx_group::X25519MLKEM768];
            p.cipher_suites = vec![aws_lc_rs::cipher_suite::TLS13_AES_256_GCM_SHA384];
            Arc::new(p)
        }

        async fn handshake(
            client: Arc<CryptoProvider>,
            server: Arc<CryptoProvider>,
        ) -> (bool, bool) {
            let (c_key, s_key) = (generate_key().unwrap(), generate_key().unwrap());
            let c_cfg = client_config_with(
                client,
                &c_key,
                PeerRule::Exact(s_key.verifying_key().to_bytes()),
                ALPN_NORMAL,
            )
            .unwrap();
            let s_cfg =
                server_config_with(server, &s_key, PeerRule::AnyForPairing, ALPN_NORMAL).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let serving = tokio::spawn(async move {
                let (tcp, from) = listener.accept().await.unwrap();
                accept(&nets(), tcp, from, s_cfg).await.is_ok()
            });
            let client_ok = connect(&nets(), addr, c_cfg).await.is_ok();
            (client_ok, serving.await.unwrap())
        }

        #[tokio::test]
        async fn the_matching_profile_connects() {
            assert_eq!(handshake(provider(), provider()).await, (true, true));
        }

        #[tokio::test]
        async fn a_server_refuses_a_client_without_post_quantum_key_exchange() {
            assert!(!handshake(classic_only(), provider()).await.1);
        }

        #[tokio::test]
        async fn a_client_refuses_a_server_without_post_quantum_key_exchange() {
            assert!(!handshake(provider(), classic_only()).await.0);
        }

        #[tokio::test]
        async fn a_server_refuses_a_cipher_outside_the_profile() {
            assert!(!handshake(without_the_required_suites(), provider()).await.1);
        }

        #[tokio::test]
        async fn a_client_refuses_a_server_using_a_cipher_outside_the_profile() {
            assert!(!handshake(provider(), without_the_required_suites()).await.0);
        }

        /// A hand-built TLS 1.2 `ClientHello`, since this build cannot speak 1.2 at all.
        fn tls12_client_hello() -> Vec<u8> {
            let mut hello = vec![0x03, 0x03];
            hello.extend_from_slice(&[7; 32]);
            hello.push(0);
            hello.extend_from_slice(&[0, 2, 0xC0, 0x2F]);
            hello.extend_from_slice(&[1, 0]);
            hello.extend_from_slice(&[0, 0]);
            let len = u32::try_from(hello.len()).unwrap().to_be_bytes();
            let mut handshake = vec![1, len[1], len[2], len[3]];
            handshake.extend_from_slice(&hello);
            let outer = u16::try_from(handshake.len()).unwrap().to_be_bytes();
            let mut record = vec![0x16, 0x03, 0x03, outer[0], outer[1]];
            record.extend_from_slice(&handshake);
            record
        }

        #[tokio::test]
        async fn a_server_never_answers_tls_1_2_with_a_handshake() {
            let key = generate_key().unwrap();
            let cfg = server_config(&key, PeerRule::AnyForPairing, ALPN_NORMAL).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let serving = tokio::spawn(async move {
                let (tcp, from) = listener.accept().await.unwrap();
                accept(&nets(), tcp, from, cfg).await.is_ok()
            });
            let mut raw = TcpStream::connect(addr).await.unwrap();
            raw.write_all(&tls12_client_hello()).await.unwrap();
            let mut reply = Vec::new();
            let _ = raw.read_to_end(&mut reply).await;
            assert!(!serving.await.unwrap(), "the handshake must fail");
            assert_ne!(
                reply.first(),
                Some(&0x16),
                "the server must not continue the handshake"
            );
        }

        #[tokio::test]
        async fn a_plaintext_client_gets_nothing() {
            let key = generate_key().unwrap();
            let cfg = server_config(&key, PeerRule::AnyForPairing, ALPN_NORMAL).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let serving = tokio::spawn(async move {
                let (tcp, from) = listener.accept().await.unwrap();
                accept(&nets(), tcp, from, cfg).await.is_ok()
            });
            let mut raw = TcpStream::connect(addr).await.unwrap();
            raw.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
            let mut reply = Vec::new();
            let _ = raw.read_to_end(&mut reply).await;
            assert!(!serving.await.unwrap());
            assert!(!reply.windows(4).any(|w| w == b"HTTP"));
        }
    }

    #[test]
    fn spki_round_trips() {
        let id = [9u8; 32];
        assert_eq!(id_from_spki(&spki_of(&id)), Some(id));
    }

    #[test]
    fn rejects_a_non_ed25519_spki() {
        let mut spki = spki_of(&[1; 32]);
        spki[8] = 0x71;
        assert_eq!(id_from_spki(&spki), None);
        assert_eq!(id_from_spki(&spki_of(&[1; 32])[..43]), None);
    }

    #[test]
    fn provider_has_only_the_hybrid_group_and_tls13_suites() {
        let p = provider();
        assert_eq!(p.kx_groups.len(), 1);
        assert_eq!(p.cipher_suites.len(), 2);
        assert!(p.cipher_suites.iter().all(|s| s.tls13().is_some()));
    }

    #[test]
    fn loads_an_ed25519_identity() {
        let key = SigningKey::from_bytes(&[5; 32]);
        client_config(&key, PeerRule::Exact([1; 32]), ALPN_NORMAL).unwrap();
        server_config(&key, PeerRule::AnyForPairing, ALPN_PAIRING).unwrap();
    }
}
