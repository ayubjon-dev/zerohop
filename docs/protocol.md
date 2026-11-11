# Zerohop Protocol, Version 1

**Status:** draft. Not implemented yet. Not independently audited.

Zerohop moves files and clipboard contents between devices owned by one person, directly over the local network. This document defines what goes on the wire and how a receiver must treat what it gets. The companion [threat model](threat-model.md) explains what this design defends against and what it does not.

The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are used as described in RFC 2119.

## 1. Design goals

1. Only devices the owner has paired can exchange anything. Everything else is rejected before any application data is parsed.
2. No servers, accounts, or internet access. Version 1 only talks to devices on the local network, in both directions.
3. There is exactly one way to connect: authenticated and encrypted. No option turns that off.
4. Throughput is limited by the network or the disk, not by the protocol.
5. A receiver never trusts a sender about where a file goes or what it is.

Non-goals for version 1: transfer over the internet, folder synchronization, hiding from an active attacker on the same network that a Zerohop device exists there, and protecting data on a device that is already compromised.

## 2. Terminology

| Term | Meaning |
|---|---|
| Device key | An Ed25519 key pair that identifies one device within one circle. |
| Device ID | The 32-byte Ed25519 public key. |
| Circle | The set of devices one person has paired together. |
| Ledger | The signed set of membership events that defines a circle. |
| Member | A device that the ledger currently says belongs to the circle. |
| Host | During pairing, the device already in the circle (or about to create it). |
| Joiner | During pairing, the device being added. |
| Session | One authenticated TLS connection between two members. |

## 3. Cryptography

| Purpose | Algorithm |
|---|---|
| Channel | TLS 1.3 only (RFC 8446) |
| Key exchange | `X25519MLKEM768` only |
| Cipher suites | `TLS_AES_128_GCM_SHA256`, `TLS_CHACHA20_POLY1305_SHA256` |
| Device authentication | Ed25519 raw public keys (RFC 7250, RFC 8032), required in both directions |
| Pairing | SPAKE2 over edwards25519, plus a short comparison code |
| Key derivation and MAC | HKDF-SHA-256, HMAC-SHA-256 |
| File integrity | BLAKE3 |
| Ledger signatures | Ed25519, strictly verified, with domain separation |
| Randomness | The operating system's CSPRNG |

Implementations MUST NOT negotiate anything outside this table. TLS session tickets, session resumption and 0-RTT data MUST be disabled, and SNI MUST NOT be sent. There is one key exchange group, so there is nothing to downgrade to.

Every signature, hash and derivation that needs domain separation uses one of these exact prefixes:

| Prefix | Used for |
|---|---|
| `zerohop-v1/ledger\0` | Ledger event signatures |
| `zerohop-v1/circle\0` | Circle ID derivation |
| `zerohop-v1/chain\0` | Per-author ledger hash chain |
| `zerohop-v1/tag\0` | Discovery tags |
| `zerohop-v1/pair-confirm` | Pairing key confirmation (HKDF info) |
| `zerohop-v1/pair-sas` | Pairing comparison code (HKDF info) |
| `zerohop-v1/sas-commit\0` | Pairing comparison code commitment |
| `zerohop-v1/file\0` | File root hash |
| `EXPORTER-zerohop-pair-v1` | TLS exporter label for pairing |

**Rationale.** TLS 1.3 rather than a custom or Noise-based channel, because it has the deepest review history of any secure transport. Raw public keys rather than certificates, because identity is a key the owner already pinned, and removing X.509 removes a whole parser from the attack surface. A hybrid post-quantum exchange, because a recording of today's traffic should stay unreadable later. AES-128-GCM is an accepted choice for post-quantum use and is hardware-accelerated on current devices; ChaCha20 covers devices without AES instructions.

A feasibility prototype on 2026-11-09 (rustls 0.23.46 with aws-lc-rs) confirmed this exact profile negotiates, rejects unknown keys in both directions, completes a handshake in under 1 ms, and encrypts at about 4 GiB/s over loopback on an Apple M4. That is roughly 40 times a fast Wi-Fi link, so encryption is not expected to be a bottleneck. Phones still need to be measured.

## 4. Device identity

A device generates a fresh key pair each time it creates or joins a circle. Device keys are per circle: leaving a circle deletes the key, and joining a new one starts with a new key. The private key MUST stay on the device, MUST NOT appear in logs or backups, and MUST be stored using the platform's protected storage:

| Platform | Storage |
|---|---|
| iOS | Encrypted with a secret kept in the keychain as this-device-only, so it is never synced and cannot be restored onto another device |
| macOS | Encrypted with a secret kept in the login keychain, which is not synced |
| Android | Encrypted under a key held in Android Keystore, excluded from backup |
| Windows | Protected with DPAPI for the current user, in the local profile rather than the roaming one |

Keychain items can survive an app being uninstalled. On first launch with no app data, an implementation MUST delete any leftover Zerohop key instead of using it.

A device MUST durably record a signed ledger event and the next `seq` in one atomic write before that event leaves the device. A crash then cannot cause a number anyone has seen to be used twice, and cannot leave a gap, which would make all of that device's later events unusable. If it receives a validly signed event by its own key with a `seq` at or above the one it has recorded, its state was lost: it MUST stop signing and ask the owner to pair it again. An unsigned claim, such as a sequence number in `LedgerHave`, MUST NOT trigger this.

Only one process may write a device's state at a time, enforced by an exclusive operating-system lock. Two writers could sign the same `seq`, which the protocol treats as a compromised device, and a stale writer could undo a removal. A process that cannot get the lock may read a snapshot of the state to send files, but it MUST NOT sign or save anything. A command that changes membership MUST fail rather than proceed while another process owns the state, so a removal can never appear to succeed without reaching a running receiver.

## 5. Circles and the ledger

Pairing a new device with any one member adds it to the circle, so owning four devices means three pairings rather than six. Membership is defined by a set of signed events that members exchange when they connect.

### 5.1 Events

```text
SignedEvent {
    body: EventBody,
    signature: [u8; 64],
}

EventBody {
    author: [u8; 32],   // device ID of the signer
    seq: u32,           // per author, starts at 1, no gaps
    kind: EventKind,
}

EventKind =
    0: Genesis { nonce: [u8; 32], name: String }
    1: Add     { device: [u8; 32], name: String, platform: Platform }
    2: Revoke  { device: [u8; 32], last_seen_seq: u32 }

Platform = 0: Windows, 1: MacOS, 2: IOS, 3: Android, 4: Linux, 5: Other
```

The signature covers `zerohop-v1/ledger\0 || circle_id || encode(body)`, and verification MUST reject non-canonical signatures.

`circle_id` is `SHA-256(zerohop-v1/circle\0 || creator_device_id || nonce)`. A circle has exactly one valid genesis event: `seq` 1, whose author and nonce produce the circle ID.

`last_seen_seq` in a `Revoke` is the highest `seq` from the target device that the revoking device had seen.

### 5.2 Computing membership

Membership MUST be computed as a pure function of the set of events held. It is recomputed whenever that set changes. Events with a bad signature or the wrong circle are never added to the set.

1. **Usable events.** For each author, an event is usable if every lower `seq` from that author is present and its `seq` is at most 128.
2. **Equivocation.** Two usable events with the same `(author, seq)` and different bodies mean the author equivocated. Its `eq_cut` is the smallest such `seq` minus 1.
3. **Adds that count.** Walk each author's `seq` values in order. A `seq` holding one event, a usable `Add`, takes one of 32 places, and the walk stops when a 33rd would be needed. A `seq` holding two variants takes no place, and any usable `Add` among them counts, so a fork can only ever make more `Add` events count, never fewer.
4. **Ever-members.** Start with the genesis author. Repeatedly add the target of any counted `Add` whose author is in the set. No cutoffs apply here, and forked events are included. This set only ever grows as events arrive.
5. **Revokes.** Every usable `Revoke` whose author and target are both ever-members counts, permanently.
6. **Revoked set.** The targets of those `Revoke` events, plus every ever-member that equivocated.
7. **Cutoffs.** `cutoff(d)` is the smallest of `eq_cut(d)` and the value of every counted `Revoke` targeting `d`, or unlimited if there are none. The value is `last_seen_seq`, except when a device revokes itself, where it is that `Revoke` event's own `seq` minus 1 regardless of the field.
8. **Live set.** Start with the genesis author. Working breadth-first, in order of (depth, author ID, `seq`), add the target of each counted `Add` whose author is in the live set and whose `seq` is at most `cutoff(author)`. Revoked devices are traversed but do not count toward the limit: once 64 non-revoked devices are in the set, further non-revoked targets are skipped.
9. **Members** are the live set minus the revoked set.

**Revocation is permanent and monotonic.** Steps 4 to 6 use no cutoffs and depend on nothing that can shrink, so no combination of later events can bring a revoked device back. A device outside the ever-member set has no authority at all.

The cost of this rule is deliberate. A revoked device can still sign a `Revoke`, and it counts if the device can deliver it to a member that has not yet learned of its own revocation. Such a late revocation removes other devices but cannot bring the revoked device back. No rule without a central authority can reliably tell a late revocation from a concurrent one, so Zerohop chooses the failure that loses availability rather than the one that loses security. It requires being on the same network as a member that has not heard of the revocation, and for each member that window closes once it learns.

A `Revoke` drops any device the target added that the revoker had not yet seen. Those devices are by definition unknown to the revoker, so an implementation SHOULD sync before revoking when possible, MUST tell the owner when it last synced with the target, and MUST notify the owner on any device that later computes such a drop.

### 5.3 Rules

- A device MAY revoke itself, which is how it leaves a circle.
- On platforms that offer it, issuing a `Revoke` MUST require device owner authentication (biometric or passcode). The automatic cleanup in section 8.1 step 14 is the only exception.
- A device that learns it is no longer a member MUST delete its device key and tell the owner.
- `last_seen_seq` is self-reported, so colluding devices can overstate it. Any member whose `Add` was signed by a revoked device MUST be labeled as such and offered for removal. The owner MUST be able to revoke a device again from the list of removed devices, which applies a new cutoff. A device SHOULD warn when `last_seen_seq` exceeds what it holds for the target.
- A device belongs to at most one circle. To join another it must first leave the current one.
- Display names from the ledger or `PeerInfo` go through the character removal in N2 (section 11.1) and are shown with a short identifier derived from the device ID.

### 5.4 Syncing

After `Hello`, each side sends `LedgerHave`:

```text
LedgerHave   { authors: Vec<AuthorHead> }
AuthorHead   { author: [u8; 32], seq: u32, chain: [u8; 32] }
LedgerEvents { events: Vec<SignedEvent> }
```

For each author, `chain` is a hash chain over its events: `chain_0` is 32 zero bytes, and `chain_n = SHA-256(zerohop-v1/chain\0 || chain_(n-1) || SHA-256(encode(body_n)))`. A device holding an equivocation by an author reports `chain` as 32 bytes of `0xFF`. `seq` is the author's highest contiguous sequence number. `LedgerHave` may span several frames.

Lists are split across frames of at most 2000 authors or 1000 events. The first frame holding fewer than that ends the list, so a list that is an exact multiple ends with an empty frame. Each session runs exactly two sync rounds, each consisting of `LedgerHave` then `LedgerEvents` in both directions, because the second round carries what the first revealed. A peer cannot extend the exchange.

Each side sends whatever the peer lacks. If two heads have the same `seq` but different `chain`, or if either head is all `0xFF`, both sides send all events by that author, so that forks surface as equivocation instead of staying hidden. A device stores at most two variants of an `(author, seq)`, the first it receives, because two are enough to prove a fork. A further variant is refused, never swapped in: replacing a stored event, by any rule, would let whoever holds a key undo the effects it has already had. The one exception is a valid genesis event, which is always stored. Devices can therefore end up holding different variants from an author that forked three or more ways, which affects only that author's own events. Events with a `seq` above 128 can never be usable and are not stored.

Hard limits on what is held: 4096 non-`Revoke` events and 1024 `Revoke` events. When a limit is reached, events from current members are still stored, up to 128 per member and 13,312 events in all, so that a flood cannot stop the owner from removing the device causing it. Events from authors that were never added give way to events from devices that were. Anything else is refused. If the reserve itself runs out, the owner is told to start a new circle (section 5.6). These limits can only be reached through abuse by a member.

The pending set of events from unknown authors is a cache: dropping from it never changes the result, because sync will offer them again. When a limit is reached, events from authors that were never added give way to events from devices that were, and an event that cannot be stored is skipped rather than ending the session, so a flood cannot stop a `Revoke` later in the same batch.

### 5.5 Applying changes

When membership changes:

- A device that issues or learns of a `Revoke` MUST deliver the relevant events to the affected peer, then close that session with `NOT_MEMBER`.
- A server MAY complete the handshake with the key of any ever-member that is no longer a member, solely to inform it. It sends `Hello`, then `LedgerEvents` with everything needed to verify the removal (the `Revoke` events, their authors' earlier events, and their `Add` paths back to genesis), then `Error { NOT_MEMBER }`, and closes. It MUST NOT read or parse any frame from such a peer.
- Each device that learns of a removal MUST rotate its discovery key (section 6).
- After pairing, the host MUST promptly sync with every member it can reach. Until a member has synced, it will refuse connections from the new device. This is a known limitation of having no central server.

### 5.6 Starting over

Revocation state belongs to one circle. If a circle is destroyed by an attacker, or fills up, any device can leave and create a new circle, and the owner pairs the other devices into it with new keys. A stolen device is simply not paired again. No data is lost, since circles hold no files.

## 6. Discovery

Devices find each other with DNS-SD over mDNS, using the platform's own service: Bonjour on Apple platforms, `NsdManager` on Android, the DNS-SD API on Windows. The application MUST NOT implement its own multicast parser.

Service type: `_zerohop._tcp`.

- The instance name is 16 random hex characters. It MUST change whenever the tag epoch or the network changes.
- The TXT record contains only `v=1` and `t=<tag>`.
- `tag` is the hex encoding of the first 8 bytes of `HMAC-SHA-256(discovery_key, zerohop-v1/tag\0 || u64be(epoch))`, where `epoch = floor(unix_seconds / 600)`.

Each device owns one 32-byte **discovery key** and shares it directly with each member over a session, using `DiscoveryKey { generation: u32, key: [u8; 32] }`. A device accepts a discovery key only from the device it belongs to, with a higher generation than before, and never forwards another device's key. A device rotates its key when a member is removed.

To recognize a peer, a device computes the expected tag for each member's key for epochs `e-1`, `e` and `e+1`. Device names, models and keys MUST NOT appear in instance names or TXT records. The mDNS host record is managed by the operating system and may contain the machine's hostname; see the threat model.

**Discovery results are hints.** They choose which address to try and nothing more. Authorization happens only in the TLS handshake (section 7). A forged announcement can at worst waste a connection attempt.

A connection attempt always has one intended member, and that member's key is pinned. A device tries tagged instances and the member's last known addresses first. If those fail, for example because of clock skew or a discovery key it has not yet learned, it MAY try untagged instances with the same pin, up to 16 attempts per intended member per scan.

A device that the owner has made invisible MUST stop advertising. On phones, browsing SHOULD run only while the app is in the foreground or a transfer is pending.

### 6.1 Address rules

An address is **local** when, after converting IPv4-mapped IPv6 to IPv4, it is on-link on an active interface that is neither a VPN nor cellular: either in the same subnet as one of that interface's addresses, or IPv6 link-local on it. Zone-less link-local addresses from a QR code are tried on each eligible interface.

Version 1 MUST refuse any address that is not local, for outgoing connections and, before TLS starts, for incoming ones. This applies equally to addresses from mDNS, QR codes and manual entry, and there is no override.

## 7. Transport

Each session runs over one TCP connection protected by TLS as specified in section 3. `TCP_NODELAY` MUST be enabled because the sender already coalesces data into frames.

| ALPN | Purpose |
|---|---|
| `zerohop/1` | Normal sessions between members |
| `zerohop-pair/1` | Pairing only |

The major version lives in ALPN. There is no in-band version negotiation.

**Normal and pairing listeners MUST be separate,** each with its own socket and TLS configuration. The pairing verifier accepts any key, so sharing a configuration would let a stranger reach the normal parser during pairing.

**Verifying the peer in a normal session.** Both directions use raw public keys.

- The server MUST accept a client only if its key belongs to a current member, except for the proof delivery in section 5.5.
- The client MUST accept the server only if its key is exactly the member it intended to reach. Accepting "any member" is not sufficient, because one member must not be able to impersonate another.
- A failed check closes the connection during the handshake. No application data is read.

In TLS 1.3 a client finishes the handshake before the server has checked its key, so a client MUST NOT treat a session as established until it has received the server's `Hello`.

**Before authentication,** a device MUST limit itself to 16 concurrent unauthenticated connections and allow at most 5 seconds for a handshake.

**One session per peer.** If two members connect to each other at the same time, the session whose client has the lexicographically lower device ID survives, and the other closes with `DUPLICATE_SESSION`. A new connection from a client that already has a session replaces the old one, which covers half-open connections.

**Liveness.** Each side sends `Ping` after 15 seconds of silence and closes the session after 45 seconds without receiving anything. While a side's writes are blocked by backpressure, or its disk is stalled, that limit is 120 seconds.

## 8. Pairing

There is one pairing protocol. A QR code and a typed code differ only in how the shared secret gets from one screen to the other.

| Method | Secret | SPAKE2 password | Extra check |
|---|---|---|---|
| QR code | 16 random bytes | the 16 raw bytes | Joiner also pins the host's key from the QR code |
| Typed code | 6 uniformly random decimal digits | the 6 ASCII digits | Burned on first use |

### 8.1 Flow

1. The owner opens pairing on the host. If the host has no circle yet, it creates one with a genesis event. Pairing mode MUST start only from a direct user action. The joiner generates a new device key.
2. The host generates the secret and a random 4-hex-character **room label**, shows both, and opens a dedicated pairing listener. It advertises `_zerohop-pair._tcp` with TXT `v=1` and `r=<room label>`. A joiner seeing several rooms lets the owner pick by label. A host MAY offer a QR code and a typed code at once, each with its own secret and its own listener; only the typed code's listener is advertised, and the QR code carries the other's addresses.
3. Pairing mode ends after 120 seconds or when pairing finishes. An attempt already past step 6 may complete. Outside pairing mode the listener MUST NOT exist. Only one pairing attempt runs at a time.
4. The joiner connects. Both sides present raw public keys. Neither checks the other's key against a list, except that a QR joiner MUST verify the host's key equals the key in the QR code.
5. Both sides compute `E = TLS-Exporter("EXPORTER-zerohop-pair-v1", "", 32)`.
6. **For a typed code, the host MUST burn the secret when it receives the first `PakeA`, before computing `PakeB`.** Any later attempt needs a new code from a new user action, whether or not the first attempt succeeded. A 128-bit QR secret cannot be guessed and is not burned this way, so an attacker burning typed codes cannot block QR pairing. When both are offered, burning the typed code closes only its listener, and pairing mode continues with the QR code.
7. They run SPAKE2 with the joiner as side A and the host as side B, using the raw device IDs as identities and the encoding above as the password. The result is `K`.
8. Both sides derive 64 bytes: `HKDF-SHA-256(ikm=K, info="zerohop-v1/pair-confirm" || E || joiner_id || host_id || msg_a || msg_b)`. The first 32 bytes are `K_host`, the last 32 are `K_joiner`.
9. Key confirmation, comparison code and approval use these frames in order. Tags MUST be compared in constant time. A wrong tag, or a commitment that does not match, ends pairing with `PAIRING_FAILED`.

| # | From | Frame | Contents |
|---|---|---|---|
| 1 | Joiner | `PakeA` | `msg_a: [u8; 33]` |
| 2 | Host | `PakeB` | `msg_b: [u8; 33]`, `tag = HMAC(K_host, "host")` |
| 3 | Joiner | `Confirm` | `tag = HMAC(K_joiner, "joiner")` |
| 4 | Host | `SasCommit` | `commit = SHA-256(zerohop-v1/sas-commit\0 \|\| N_h)` |
| 5 | Joiner | `SasNonce` | `N_j`, 32 random bytes |
| 6 | Host | `SasReveal` | `N_h`, 32 random bytes |
| 7 | Both | `PeerInfo` | `name`, `platform` |
| 8 | Both | `Approve` | Sent after the owner confirms on that device |
| 9 | Host | `LedgerEvents`, then `WelcomeEnd` | The ledger, then `WelcomeEnd { event_count: u32 }` |
| 10 | Joiner | `WelcomeAck` | Empty |

10. **Comparison code.** After frame 6, the joiner checks `commit`. Both sides compute `HKDF-SHA-256(ikm=K, info="zerohop-v1/pair-sas" || E || joiner_id || host_id || N_h || N_j, length=4)`, read it as a big-endian `u32`, reduce it modulo 1,000,000, and display it as six digits. The commitment stops a man in the middle from choosing values that make two different connections show the same number.
11. **Approval on both devices.** Each device shows the comparison code and the other device's name and platform, and asks the owner to confirm that the numbers match. Declining sends `PAIRING_DECLINED`. The host MUST have both its own owner's confirmation and the joiner's `Approve` before it signs anything.
12. The host persists a signed `Add` for the joiner, authored by `host_id`, together with a pending flag that keeps it out of sync. It then sends the ledger in one or more `LedgerEvents` frames. Discovery keys are exchanged in the first normal session, which also covers a device added through another device.
13. The joiner verifies the ledger: the genesis event produces the circle ID, and the computed members include both the host and itself. It stores the circle and sends `WelcomeAck`.
14. On `WelcomeAck` the host clears the pending flag. If none arrives within 30 seconds, or the host restarts with the flag still set, it signs a `Revoke` for that device with `last_seen_seq` 0, so no phantom member remains. The joiner discards its key and can try again.

**Timeouts.** Each protocol step has 10 seconds. Owner approval has 60 seconds. A joiner MUST NOT retry automatically after sending `PakeA`; each retry needs an owner action.

**Before frame 3 is verified,** only the frames for steps 1 to 3 are legal, each with a payload of at most 128 bytes.

Because `E` differs across two separately terminated TLS connections, an attacker relaying between the two devices cannot complete key confirmation without knowing the secret. An attacker who does know the secret, such as someone who saw the screen, must still get past the comparison code on both devices.

### 8.2 QR payload

```text
zerohop://pair/1?k=<host device ID, base64url>&s=<secret, base64url>&a=<ip:port>[&a=<ip:port>...]
```

Applications MUST NOT register this URI scheme with the operating system. Only the scanner inside Zerohop reads it, so a link on a web page can never start pairing. Addresses in the payload are subject to section 6.1.

A joiner MUST refuse a payload with an unknown parameter, a repeated `k` or `s`, or more than 8 addresses. It tries the addresses in order until one completes the TLS handshake with the pinned key, and MUST NOT move on to another address once it has sent `PakeA`.

## 9. Framing

After the TLS handshake, both directions carry frames:

```text
+----------+-------------+---------------------+
| type: u8 | length: u24 | payload             |
|          | big-endian  | (length bytes)      |
+----------+-------------+---------------------+
```

Control payloads are encoded with [postcard](https://postcard.jamesmunns.com/wire-format). Enum variants use the numbers listed in this document, and when no numbers are listed, the order in which variants are written. `FileData` uses a fixed layout because it is the hot path.

Limits are checked **before** any allocation:

- `FileData` and `ClipData` payloads: at most 1 MiB.
- Every other payload: at most 256 KiB.

A frame over its limit, a payload with trailing bytes, or a frame invalid in the current state is fatal: the session closes with an `Error` frame.

Frame types below `0x80` are mandatory, and an unknown one is fatal. Types `0x80` to `0xFF` are optional extensions, and an unknown one is ignored.

| Type | Frame | Direction |
|---|---|---|
| `0x01` | `Hello` | both |
| `0x02` | `Goodbye` | both |
| `0x03` | `Error` | both |
| `0x04` | `Ping` | both |
| `0x05` | `Pong` | both |
| `0x10` | `LedgerHave` | both |
| `0x11` | `LedgerEvents` | both |
| `0x12` | `DiscoveryKey` | both |
| `0x20` | `Offer` | sender |
| `0x21` | `OfferItems` | sender |
| `0x22` | `Decision` | receiver |
| `0x23` | `FileData` | sender |
| `0x24` | `SegmentEnd` | sender |
| `0x25` | `FileEnd` | sender |
| `0x26` | `Progress` | receiver |
| `0x27` | `Cancel` | both |
| `0x28` | `Complete` | sender |
| `0x29` | `Done` | receiver |
| `0x2A` | `ResumeRequest` | sender |
| `0x2B` | `ResumeState` | receiver |
| `0x30` | `ClipOffer` | both |
| `0x31` | `ClipFetch` | both |
| `0x32` | `ClipData` | both |
| `0x33` | `ClipEnd` | both |
| `0x34` | `ClipPull` | receiver of a clip |
| `0x40` | `PakeA` | joiner |
| `0x41` | `PakeB` | host |
| `0x42` | `Confirm` | joiner |
| `0x43` | `SasCommit` | host |
| `0x44` | `SasNonce` | joiner |
| `0x45` | `SasReveal` | host |
| `0x46` | `PeerInfo` | both |
| `0x47` | `Approve` | both |
| `0x48` | `WelcomeEnd` | host |
| `0x49` | `WelcomeAck` | joiner |

Frames `0x40` to `0x49`, `Error`, `LedgerEvents` and `DiscoveryKey` are the only frames legal on a pairing connection. Frames `0x40` to `0x49` are illegal on a normal session.

### 9.1 Session start

Each side sends `Hello` first:

```text
Hello {
    software: String,      // at most 64 bytes
    features: u64,         // bit set; no bits are defined in version 1
    name: String,          // display name, at most 63 bytes
    platform: Platform,
    circle_id: [u8; 32],
}
```

A device MUST NOT send any other frame before it has received the peer's `Hello`. Different `circle_id` values are fatal. Each side then sends `LedgerHave` and syncs as in section 5.4. Once both are known to be members, each side sends its `DiscoveryKey`, exactly once per session, and a session in which the frame does not arrive at that point is invalid.

## 10. File transfer

### 10.1 Offer and decision

At most one transfer runs per direction per session. Additional transfers queue on the sender.

```text
Offer { transfer_id: [u8; 16], item_count: u32, total_bytes: u64 }

OfferItems { items: Vec<Item> }

Item {
    index: u32,          // starts at 0, no gaps
    path: Vec<String>,   // path components, never a joined string
    kind: ItemKind,      // 0: File, 1: Directory
    size: u64,           // 0 for a directory
    mtime_ms: Option<i64>,
    created_ms: Option<i64>,
    group: Option<u32>,  // keeps related files together, e.g. Live Photos
}

Decision { transfer_id: [u8; 16], accept: bool, reason: Option<u16> }
```

`OfferItems` frames stream the manifest, and the receiver validates items as they arrive under section 11. A receiver MUST NOT hold more than 32 MiB of decoded manifest in memory; a larger one is spilled to disk or declined.

The receiver MUST compute the total size itself with overflow-checked addition and decline with `INVALID_PATH` if it differs from `total_bytes`. Duplicate paths, and a path that is both a file and the parent of another item, compared after name handling and case folding, also fail the offer, as do directories with a nonzero size.

The decision covers the whole offer: version 1 has no partial acceptance.

Receive policy is local to the receiver and set per paired device:

| Policy | Behavior |
|---|---|
| Always ask | The owner confirms every offer. |
| Automatic | Offers are accepted up to the limits below. |
| Automatic except programs (default) | As automatic, but an offer containing a program or script asks first. |

An optional setting, "Confirm devices added elsewhere", is off by default and not implemented yet. When it is on, a device whose `Add` was signed by another device is treated as "Always ask" on this device until the owner confirms it here. It narrows what a stolen or compromised member can do by adding a device of its own.

Automatic acceptance has per-peer limits: 10 GiB per offer (configurable) and a rolling 24-hour budget of 100 GiB and 500,000 files, which includes partial data left behind. Anything above asks. A device accepts at most 10 offers per minute from one peer and declines the rest with `RATE_LIMITED`.

### 10.2 Sending data

The sender walks accepted files in index order, one at a time, with strictly sequential offsets:

```text
FileData payload (fixed layout):
    file_index: u32 little-endian
    offset:     u64 little-endian
    bytes:      at most 1 MiB - 12
```

Each file is divided into 16 MiB segments, with at least one segment even for an empty file.

```text
SegmentEnd { file_index: u32, segment: u32, hash: [u8; 32] }   // BLAKE3 of that segment
FileEnd    { file_index: u32, root: [u8; 32] }
Progress   { transfer_id: [u8; 16], file_index: u32, segment: u32 }
Complete   { transfer_id: [u8; 16], files: u32, bytes: u64 }
Done       { transfer_id: [u8; 16], status: u16, published: u32, held: u32 }
```

`root` is `BLAKE3(zerohop-v1/file\0 || u64le(size) || hash_0 || hash_1 || ...)`.

Data that would run past an item's declared size, or an offset other than the expected next one, is a fatal `PROTOCOL_VIOLATION`.

Both sides hash as they stream, so there is no scan before sending and no extra read afterwards. The receiver compares each `SegmentEnd`, so a corrupted stream is caught within 16 MiB. These hashes cover the bytes between the sender's read and the receiver's hash. They do not prove the disk stored them correctly, which a resume re-checks.

If the size or modification time of a source file changes while it is being read, the sender MUST stop that file and report `SOURCE_CHANGED`.

Many small files add only a few frames each. There is no per-file round trip.

`Progress` reports segments verified so far and is used for display and the sender's journal.

**Flow control** relies on TCP backpressure. A receiver stops reading the socket when its write queue is full. In-flight memory is bounded to 64 MiB on the receiver and 32 MiB of read-ahead on the sender, per session, and there is one session per peer.

**Durability.** Zerohop copies and never deletes the source, so `Done` means received and hash-verified, not guaranteed to survive an immediate power loss. Receivers need not `fsync` every file. A resume always re-verifies what is on disk, so a lost write is detected rather than silently accepted.

### 10.3 Resuming

Both sides keep a small journal per transfer for 24 hours, holding segment hashes and the manifest hash. Journals are limited to 64 MiB per device in total, with the oldest evicted first.

1. After reconnecting, the sender sends `ResumeRequest { transfer_id, manifest: [u8; 32] }`, where `manifest` is the BLAKE3 of the encoded items. Each request counts against the same per-peer limit as offers, 10 per minute, because answering one means re-hashing what is on disk. Both sides keep answering pings while re-hashing. Only one writer may hold a transfer at a time, so a resume waits briefly for a replaced connection's writes to drain. Where the file system has no locks, as Android's shared storage does not, the claim is held inside the one process that writes there.
2. The receiver checks that `manifest` matches the one the owner approved, re-hashes what it has on disk, and replies `ResumeState { transfer_id, next_file: u32, next_segment: u32, prefix: [u8; 32] }`, where `prefix` is the BLAKE3 of the current file's completed segment hashes. `next_segment` is always below the file's segment count: when every segment is present but the file was not closed, the last one is sent again so that the root hash is still checked. It replies with `UNKNOWN_TRANSFER` if the state is gone.
3. The sender confirms the source's size and modification time against the manifest. If either differs, the approved manifest no longer describes the file, so it sends `Cancel { SOURCE_CHANGED }`.
4. Otherwise the sender **re-hashes the source prefix** and compares it with both the journal and `prefix`. Modification times can be coarse or preserved, so they are not sufficient. On a mismatch it restarts that file by sending from offset 0, which tells the receiver to discard what it kept of that file.

A transfer can only be resumed by the same two devices.

### 10.4 Cancelling

Either side can send `Cancel { transfer_id, reason }`, including while the owner of the receiving device is still being asked, which withdraws the offer. The receiver deletes unpublished data for that transfer, and the sender forgets its journal. Top-level items already published stay in place.

After sending `Cancel`, a device waits briefly for the other side to close the connection. Closing while data is still arriving would reset the connection, and a reset can destroy the `Cancel` on its way. The same applies to a device delivering proof of removal.

### 10.5 Fidelity

Zerohop transfers the exact bytes of a file. It MUST NOT compress, transcode, resize or otherwise alter file contents, on either side. Metadata stored inside a file, such as EXIF, stays intact because it is part of those bytes.

Phones do not always hand over the original unless asked, so an implementation MUST request it:

- On iOS, photos and videos are requested in their current form (for example `PHPickerConfiguration.AssetRepresentationMode.current`), never an automatically converted copy. Originals held only in iCloud are downloaded first.
- On Android, media is read using `ACCESS_MEDIA_LOCATION` and `MediaStore.setRequireOriginal`, because otherwise the system removes location data from EXIF.

If the original cannot be obtained, the owner MUST be told rather than silently sent a substitute.

File system metadata is a narrower promise. Modification time is preserved, and creation time where the receiving system allows it. Names change only as section 11 requires. Permission bits, extended attributes such as Finder tags, NTFS alternate streams and symlinks are not transferred.

## 11. Receiving files safely

These rules apply on every platform. Android and iOS adapt R3 and R4 to their sandboxes, for example writing into app-private storage first and handing off through `MediaStore`.

### 11.1 Name handling

The sender's path is a list of components and is never joined and re-parsed. Each component goes through these steps **in this exact order**:

- **N1.** Must be valid UTF-8.
- **N2.** Remove U+061C, U+200E, U+200F, U+202A to U+202E, and U+2066 to U+2069, so that a name cannot visually fake its extension.
- **N3.** Normalize to NFC.
- **N4.** Reject the offer if the component is empty, is `.` or `..`, contains `/`, `\`, DEL or any C0 or C1 control character, is over 255 UTF-8 bytes, or equals `.zerohop-tmp` ignoring case. Comparisons with `.`, `..` and `.zerohop-tmp`, and the test for an empty name, ignore Unicode default-ignorable code points, because some file systems skip them when comparing names. Those code points are otherwise left in place, since joiners are part of correct spelling in several languages.
- **N5.** Make it safe for this system. Characters a platform forbids become `_` (on Windows `< > : " | ? *`, and `:` on macOS). Trailing dots and spaces become `_`. A Windows reserved stem (`CON`, `CONIN$`, `CONOUT$`, `PRN`, `AUX`, `NUL`, `COM0`-`COM9`, `LPT0`-`LPT9`, and the superscript forms `COM¹`-`COM³`, `LPT¹`-`LPT³`), tested case-insensitively after trimming trailing spaces from the stem, gets `_` added to the stem, so `CON.txt` becomes `CON_.txt`. The result is shortened to fit 255 bytes, keeping the extension.
- **N6.** Apply N4 again to the final name.

Reject the offer as well if a path has more than 64 components or over 1024 bytes in total. The path given to the operating system MUST NOT contain `.` or `..`.

### 11.2 Writing

- **R1. The receiver picks the location.** Everything lands inside the configured receive folder, and the sender has no influence over the base.
- **R2. Fresh private tree.** Data goes to `<receive folder>/.zerohop-tmp/<transfer_id>/`, using handle-relative opens and exclusive creation, without following symlinks.
- **R2a. One folder per item.** Inside the private tree each top-level item has its own numbered folder. Two items can then never share a folder on disk, whatever the file system treats as the same name, including case pairs newer than the receiver's Unicode tables.
- **R3. Never overwrite.** Top-level items are moved into the receive folder with a no-replace rename. If the platform or filesystem does not support that, use an exclusive-create or link-based publish. A plain rename MUST NOT be used. Where links are not available, as in Android's shared storage, the target name is claimed by exclusive creation and journaled first, and the file is then moved over that claim, or copied if moving fails, so that a resume can remove anything partial.
- **R4. Never merge.** A top-level name that collides with anything in the receive folder gets a ` (1)` style suffix, and a directory is renamed as one whole tree. Names are compared after NFC normalization and case folding.
- **R5. No special files.** Symlinks, hard links, devices and FIFOs are never sent or created. Permission bits, setuid bits and executable bits from the sender are not applied.
- **R6. Publish only verified data.** A top-level item appears only after every file in it has matched its root hash.
- **R7. Check space.** Accepting requires free space of at least the offer size plus 1 GiB. Writing stops with `NO_SPACE` if free space falls below 1 GiB.
- **R8. Mark as downloaded on creation.** Every file and directory gets a `Zone.Identifier` stream with `ZoneId=3` on Windows, or the `com.apple.quarantine` attribute on macOS, as soon as it is created in the private tree, so that the operating system can apply its own checks. Where the system has already marked the file, as macOS does for a sandboxed app, that mark is kept. Nothing held or published is ever unmarked, even after a crash.
- **R9. Never open or parse.** Zerohop never launches, opens or decompresses a received file, and the core never parses file contents to make thumbnails or previews.

### 11.3 Recognizing programs

Under "automatic except programs", an item is a program if the extension of **any component of its path** is on the maintained list, after name handling and Unicode case folding:

- Windows: `exe`, `msi`, `msix`, `appx`, `bat`, `cmd`, `com`, `scr`, `ps1`, `vbs`, `vbe`, `js`, `jse`, `wsf`, `hta`, `lnk`, `reg`, `dll`, `cpl`, `jar`
- macOS: `app`, `command`, `pkg`, `mpkg`, `dmg`, `terminal`, `scpt`, `workflow`
- Android: `apk`, `apks`, `xapk`, `aab`
- Cross-platform: `sh`, `py`, `pl`, `rb`

A receiver also checks each file's first bytes for `MZ`, ELF, Mach-O and `#!`. The check reads the completed file from disk rather than the incoming frames, because a sender controls how bytes are split across frames and can resume past the start of a file. The result is journaled before the file is recorded as complete, so it survives an interrupted transfer. The check is skipped only for a file whose own name is on the program list, or that sits inside an application bundle (`.app`, `.pkg`, `.mpkg`, `.workflow`), because that is what the owner was told about. A program elsewhere in the same offer, or a folder merely named like one, does not excuse it. A file that matches, whatever its name, puts its top-level item on **hold**: it stays in the private tree until the owner approves it, and is deleted after 24 hours otherwise. Approving re-hashes what is on disk first and keeps the download mark, so an item that changed while held is refused. `Done.held` counts held items.

Where download marks cannot be stored, such as on FAT volumes, and on Android and iOS, which have no equivalent, "Automatic" behaves as "Automatic except programs".

This is a safety net rather than a guarantee.

## 12. Clipboard

Clipboard sync is **off by default**. The owner controls it with a master switch, per device, per direction, and per content type (text, images, files). A receiver enforces its own settings and silently drops anything not allowed.

```text
ClipOffer {
    clip_id: [u8; 16],
    ttl_ms: u32,                 // at most 120000
    items: Vec<ClipItem>,        // at most 8
}

ClipItem {
    format: ClipFormat,          // 0: Text, 1: Html, 2: Rtf, 3: Png, 4: Jpeg, 5: Files
    size: u64,                   // at most 64 MiB
    inline: Option<Vec<u8>>,     // total inline across items at most 128 KiB
}

ClipFetch { clip_id: [u8; 16], item: u32 }
ClipData  { clip_id: [u8; 16], item: u32, offset: u64, bytes: Vec<u8> }   // offset + len at most size
ClipEnd   { clip_id: [u8; 16], item: u32, hash: [u8; 32] }                // BLAKE3
ClipPull  { clip_id: [u8; 16], item: u32 }                                  // in a session of its own

ClipFiles { items: Vec<Item>, total_bytes: u64 }                            // what a Files item holds
```

- An item that does not fit inside the offer is fetched right after the offer, once per item. A device holds at most 64 MiB of clipboard data in memory in total, across all peers; an offer that would go over that is left behind.
- A session carries one clip. The receiver sends `Goodbye` when it has what it wants, and straight away if it wants nothing, so the sender cannot tell a clip that was taken from one that was left behind.
- An item's offset must be exactly the number of bytes received so far, and `ClipEnd` must match both the declared size and the hash. Any deviation, or any other frame during a fetch, is a protocol violation and nothing from that clip is delivered. So is an offer outside the limits above, an item of size 0, or inline data whose length differs from `size`.
- `ttl_ms` limits the fetch as well: a clip that has not fully arrived when its lifetime ends is dropped.
- A newer `ClipOffer` replaces the previous one. A device accepts at most 30 offers per minute from one peer.
- Clipboard contents MUST NOT be written to disk or to transfer history, MUST be wiped from memory at expiry on a best-effort basis, and MUST NOT appear in logs.
- **Copied files move only when the owner pastes them.** A `Files` item holds a `ClipFiles`: the manifest of section 10.1 and the total size, and nothing of the files themselves. When the owner pastes on the receiving device, it opens a session of its own and sends `ClipPull` naming the clip and the item. The sender answers in that session with an ordinary transfer of exactly those items, under sections 10 and 11. The receiver declines it unless its manifest hashes to the hash of the listed items, so nothing else can be served in their place.
- A sender serves a pull only for its current clip, during the clip's lifetime, and only to a device it offered that clip to; otherwise it answers `Goodbye`. A pull counts against the same per-peer rate as an offer. A newer clip of any kind ends the offer of files.
- The receiver accepts a pulled transfer without asking, because the owner asked for it by pasting. It goes into a folder of its own outside the receive folder, and every other receive rule applies, including names, limits and download marks. The receiver then hands the files to the system's paste, which copies them on. Only a device whose system can paste files that have not arrived yet takes a `Files` item, and so far that is Windows and macOS; any other leaves it behind, which costs only the list.
- A clip carries one format: `Text`, `Png`, `Jpeg` or `Files`. `Html` and `Rtf` are not sent.
- To prevent loops, a device remembers the hash of the last clip it applied and does not re-offer the same content.

**Content marked secret MUST NOT be sent:** `org.nspasteboard.ConcealedType`, `org.nspasteboard.TransientType` and `org.nspasteboard.AutoGeneratedType` on macOS; `ExcludeClipboardContentFromMonitorProcessing`, `Clipboard Viewer Ignore`, `CanIncludeInClipboardHistory=0` and `CanUploadToCloudClipboard=0` on Windows; `ClipDescription.EXTRA_IS_SENSITIVE` on Android; the same `org.nspasteboard` types on iOS, where password managers do not always set them.

**Applied content MUST be marked so it does not spread further:** `CanIncludeInClipboardHistory=0` and `CanUploadToCloudClipboard=0` on Windows, together with `ExcludeClipboardContentFromMonitorProcessing` so that clipboard tools ignore it, the current-host-only option on macOS together with `org.nspasteboard.TransientType` so that clipboard managers ignore it, and the local-only option on iOS, which keeps it out of Universal Clipboard. Android has no equivalent; `EXTRA_IS_SENSITIVE` SHOULD be set so that the system's clipboard preview hides the content.

**At expiry,** a device clears the system clipboard only if the clip it applied is still there, using the platform's change counter (`changeCount` on macOS, `GetClipboardSequenceNumber` on Windows). iOS removes the clip by itself at expiry, through the expiration date given when it is applied, and leaves anything copied since. Android has no counter and shows the clipboard only to the app in front, so the Android app labels each clip it applies and clears it only when it has the focus and that label is still there. If it is in the background at expiry it waits until it is next opened, because clearing blindly could destroy something copied since. Pictures on Android are served from memory and become unreadable at expiry either way.

**Platform behavior:**

| Platform | Sending a copy | Receiving a copy |
|---|---|---|
| Windows | Automatic. Keeps working with the window closed while the clipboard is on, from an icon next to the clock, and starts at sign-in | Automatic |
| macOS | Automatic once the owner allows it in System Settings. macOS asks the first time, and after that the app does not read until allowed. Keeps working with the window closed while the clipboard is on | Automatic |
| Android | One user action, in the notification, in quick settings, in the app, or through the optional entry in the text selection menu and the share sheet, because reads in the background are blocked. Only the app in front or the keyboard may read | Automatic, with nothing open, while the background service runs. The service runs while the clipboard is on and comes back after a restart |
| iOS | A tap inside the app. iOS asks the owner before the app may read what another app copied, unless they allowed it in Settings | Automatic while the app is open |

Copied files: Windows and macOS offer the list of what was copied in Explorer or Finder. Windows pastes it with Ctrl+V in Explorer, which fetches the files right then and copies them on with their download marks. Finder does not paste file promises, so macOS 13 and later paste through Zerohop's File Provider extension: the list becomes placeholders in a hidden place, the clipboard holds them, and the first read of one, which is the paste, makes the app fetch the files and hand each one to the extension already open. macOS adds the extension switched off, and the owner switches it on once in System Settings; until then the app says so. Phones neither offer nor paste files.

## 13. Limits

| Item | Limit |
|---|---|
| `FileData` or `ClipData` payload | 1 MiB |
| Any other frame payload | 256 KiB |
| Items per offer | 200,000 |
| Manifest size per offer, encoded | 64 MiB |
| Manifest in memory, decoded | 32 MiB |
| Path components | 64 |
| Path component | 255 UTF-8 bytes |
| Whole path | 1024 UTF-8 bytes |
| Members per circle | 64 |
| Ledger events per author | 128, of which 32 `Add` count |
| Ledger events held | 4096, plus 1024 `Revoke` |
| Display name | 63 UTF-8 bytes |
| Unauthenticated connections | 16 |
| TLS handshake time | 5 seconds |
| Pairing window | 120 seconds; a typed code is single use |
| Pairing step / owner approval | 10 seconds / 60 seconds |
| Offers per peer | 10 per minute |
| Automatic acceptance per peer | 10 GiB per offer; 100 GiB and 500,000 files per 24 hours |
| Resume requests | 1 per transfer per minute |
| Journals per device | 64 MiB |
| Clipboard offers per peer | 30 per minute |
| Inline clipboard data per offer | 128 KiB |
| Clipboard item | 64 MiB |
| Clipboard data held in memory | 64 MiB |
| Clipboard lifetime | 120 seconds |

## 14. Errors

```text
Error { code: u16, message: Option<String> }   // message at most 256 bytes, never shown as trusted text
```

| Code | Name | Meaning | Fatal to session |
|---|---|---|---|
| 1 | `PROTOCOL_VIOLATION` | Invalid frame or state | Yes |
| 2 | `FRAME_TOO_LARGE` | A frame exceeded its limit | Yes |
| 3 | `DUPLICATE_SESSION` | Lost the duplicate session tie-break | Yes |
| 4 | `NOT_MEMBER` | Peer is no longer a member | Yes |
| 10 | `PAIRING_FAILED` | Key confirmation failed | Yes |
| 11 | `PAIRING_DECLINED` | The owner declined | Yes |
| 12 | `PAIRING_UNAVAILABLE` | The ledger is full or another attempt is running | Yes |
| 20 | `DECLINED` | The owner declined | No |
| 21 | `INVALID_PATH` | A path failed section 11, or the manifest is inconsistent | No |
| 22 | `NO_SPACE` | Not enough disk space | No |
| 23 | `RATE_LIMITED` | Too many offers | No |
| 24 | `TOO_LARGE` | Over the size limit | No |
| 25 | `INTEGRITY_FAILED` | A hash did not match | No |
| 26 | `SOURCE_CHANGED` | The source changed during sending | No |
| 27 | `UNKNOWN_TRANSFER` | No state for this transfer | No |
| 28 | `CANCELLED` | The transfer was cancelled | No |

## 15. Versioning

A breaking change gets a new ALPN value (`zerohop/2`). Compatible additions use `Hello.features` and optional frame types. A device supporting several versions lists them in ALPN and accepts only versions it considers safe. There is no fallback to weaker behavior.

## 16. Open items

These need measurements or prototypes on real devices before the protocol is final:

1. Whether more than one parallel TCP connection ever helps on Wi-Fi. Offset-addressed `FileData` allows adding this later, but version 1 stays with one connection until a benchmark justifies more.
2. The best `FileData` chunk size and the 16 MiB segment size on phones.
3. Pasting files from another device into Explorer and Finder.
4. Writing to the clipboard from an Android background service on current releases, and how Android's foreground service time limits affect it.
5. Whether macOS pasteboard privacy prompts for background reads on shipping versions.
6. Hardware-backed device keys. Secure Enclave and StrongBox do not offer Ed25519. The app seals the key at rest with a secret held in the system's key store, and the key is in memory while the app runs.
7. The wire format of SPAKE2 messages. The `spake2` crate follows an older draft rather than RFC 9382, and this needs either pinning with test vectors or replacing.
8. How eligible interfaces are detected on each platform for section 6.1, including VPNs and hotspots.
9. Whether a device's mDNS host record can be kept from revealing its hostname on each platform.
