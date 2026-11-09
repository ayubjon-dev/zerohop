//! Core protocol and transfer engine for Zerohop.
//!
//! The design is specified in `docs/protocol.md`, and `docs/threat-model.md`
//! describes what it defends against. Zerohop has not been independently audited.

pub mod address;
pub mod circle;
pub mod clip;
pub mod discovery;
pub mod ledger;
pub mod names;
pub mod node;
pub mod pairing;
pub mod receive;
pub mod session;
pub mod tls;
pub mod transfer;
pub mod wire;
