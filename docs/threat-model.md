# Zerohop Threat Model

**Status:** draft, written before implementation. It describes what the design intends. Zerohop has not been independently audited, and nothing here is a claim that the software is free of vulnerabilities.

This document goes with the [protocol specification](protocol.md). Each threat lists the defense, what risk remains, and how the defense will be tested.

## 1. What needs protecting

| Asset | Why it matters |
|---|---|
| File contents in transit | Private by default |
| Clipboard contents | Often holds passwords, codes and tokens |
| Device private keys | Whoever holds one can act as that device |
| Circle membership | Decides who can send to and receive from the owner's devices |
| The receiving device | A malicious transfer must not be able to run code or damage files |
| Device metadata | Names, models and presence reveal who owns what |

## 2. Who we defend against

| ID | Attacker | Capabilities |
|---|---|---|
| A1 | Passive observer on the network | Sees all local traffic |
| A2 | Active attacker on the network | Spoofs mDNS, ARP and DNS, intercepts, alters and injects traffic, and connects to any open port |
| A3 | Malicious or compromised member | Holds a valid device key and sends deliberately hostile data |
| A4 | Stolen device | Physical possession of a device, possibly unlocked |
| A5 | Revoked device | Still holds its old key and ledger |
| A6 | Malicious file | Reaches the owner through some other route and is then transferred; has a hostile name or content |
| A7 | Bystander during pairing | Can see the screen, or tricks the owner into pairing the wrong device |
| A8 | Future decryption | Records traffic today to attack it later, including with a quantum computer |
| A9 | Supply chain | Compromised dependency, build system or release artifact |

## 3. Out of scope

Being honest about these matters as much as the defenses:

- **A compromised endpoint.** Malware running as the same user on a device can read received files and, on desktop systems, likely the device key too. Zerohop cannot defend a device from itself.
- **Traffic analysis.** An observer can see that two devices are talking, and when, and roughly how much data moved.
- **Hiding from an active attacker on the network.** Anyone on the network can connect and learn that a device runs Zerohop. They can also learn its public key, because a TLS 1.3 server presents its key before the client authenticates. The operating system's mDNS host record may also reveal the machine's hostname.
- **Denial of service on the local network.** Someone on the same network can always disrupt it, including burning typed pairing codes.
- **Data already received.** Revoking a device stops future access. It cannot erase what that device already has.
- **Files at rest.** Received files are ordinary files, protected only by the operating system.
- **Hostile content the owner chooses to open.** Zerohop warns and marks files; it cannot decide whether a document is safe.

## 4. Security invariants

These must hold in every build. The verification plan in section 7 ties each to a test, and the specification compliance review checks them against the code.

| ID | Invariant |
|---|---|
| INV-1 | Outside pairing, no application frame from a peer is parsed until mutual TLS has succeeded with the key of a current member. |
| INV-2 | The pairing listener exists only after a direct user action, uses its own socket and TLS configuration, closes within its window, and a typed code is burned on the first attempt. |
| INV-3 | Nothing learned from discovery ever grants trust. |
| INV-4 | A client connecting to a member accepts only that exact member's key. |
| INV-5 | There is no code path for plaintext, TLS below 1.3, or key exchange other than X25519MLKEM768. |
| INV-6 | Only the receiver decides where data is written. Every component passes name handling in the specified order, the path given to the OS never contains `.` or `..`, writes stay inside the transfer's private tree, and nothing is overwritten or merged. |
| INV-7 | Every length is checked against its limit before allocation. Every budget in the protocol is enforced. |
| INV-8 | Received files are never executed, opened, decompressed or parsed by Zerohop. |
| INV-9 | Clipboard sync is off until the owner enables it, skips content marked secret, marks what it applies so that it does not spread wherever the platform offers such a marker, and never touches disk or logs. |
| INV-10 | Membership is a pure function of the event set, and revocation is monotonic: no later event can bring a revoked device back. A revoked key cannot exchange data, and existing sessions with it are closed. |
| INV-11 | Private keys never leave the device and never appear in logs. A `seq` is never signed twice. |
| INV-12 | The transfer core only communicates with on-link addresses on eligible interfaces, in both directions. There are no accounts, telemetry or background calls. |
| INV-13 | File names, paths and clipboard contents do not appear in logs by default. |
| INV-14 | A new device joins only after the owner has compared the code and approved on both devices. |

Desktop builds may include an updater. It is separate from the transfer core, talks only to the release feed, verifies signatures, and can be turned off.

## 5. Threats and defenses

### 5.1 Network attackers (A1, A2)

| ID | Threat | Defense | Remaining risk | Tested by |
|---|---|---|---|---|
| T-01 | Reading files in transit | TLS 1.3 on every connection, with no plaintext mode | Sizes and timing are visible | INV-5 test, packet capture check |
| T-02 | Impersonating a device through forged discovery | Discovery only supplies addresses; the TLS handshake pins the exact key | Connection attempts can be wasted, limited per intended member | Forged mDNS integration test |
| T-03 | Man in the middle between two members | Mutual authentication against pinned keys | None known beyond library flaws | Interception test harness |
| T-04 | Reaching parser bugs without credentials | Normal mode has no pre-authentication parser beyond the TLS library, and a separate pairing listener stops a stranger reaching the normal parser | A flaw in TLS or its handshake | Fuzzing of all parsers, unknown-key test, cross-ALPN test |
| T-05 | Replaying captured sessions | TLS with fresh keys; resumption and 0-RTT disabled | None known | Configuration test |
| T-06 | Downgrading the connection | One TLS version and one key exchange group | None known | Handshake rejection tests |
| T-07 | Exhausting connections or CPU | 16 unauthenticated connections, 5 second handshake limit, inbound address filter | Local denial of service stays possible | Connection flood test |
| T-08 | Tracking a device across networks through its advertisements | No names or keys in instance names or TXT records; rotating instance names and tags; a random room label during pairing | An active attacker can connect and read the public key, which is per circle; the OS mDNS host record may reveal a hostname | Advertisement content test |
| T-09 | Being reached from, or sent to, a remote host | On-link addresses on non-VPN interfaces only, checked for outgoing and incoming connections, after normalizing mapped addresses | Other hosts on the same network | Address filter property test |
| T-10 | Recording traffic for later decryption | Hybrid X25519MLKEM768 and forward secrecy | Authentication is classical, which matters only at the time of connection | Negotiated group test |

### 5.2 Pairing (A2, A7)

| ID | Threat | Defense | Remaining risk | Tested by |
|---|---|---|---|---|
| T-11 | Guessing a typed code | PAKE allows one online guess, and the code is burned on the first `PakeA`, even if the attacker disconnects without a verdict | 1 in 1,000,000 per code the owner shows | Burn-on-first-message test |
| T-12 | Brute-forcing a code offline from captured traffic | SPAKE2 reveals nothing that can be checked offline, and it runs inside TLS | None known, subject to the implementation | PAKE test vectors |
| T-13 | Relaying between two devices during pairing | Key confirmation binds the TLS exporter and both device IDs | None known | Relay attack harness |
| T-14 | A bystander who saw the code runs a fake host or joiner | The comparison code must match on both devices and be approved on both. Its commitment step stops an attacker from choosing values to make two connections match | 1 in 1,000,000 per attempt, and the owner has to actually compare | Comparison code tests, fake host and joiner tests |
| T-15 | Blocking pairing by burning codes or occupying the single pairing slot | Accepted denial of service. The QR code and the typed code have separate listeners, and a burned typed code closes only its own, so burning codes does not block QR pairing. A stranger can still hold the slot for short periods | Pairing can be delayed or blocked on a hostile network | Attack test: a spent code leaves the QR code working |
| T-16 | Talking the owner into pairing a stranger | Approval on both devices with a comparison code | Owner error | Usability review |
| T-17 | A link starting pairing | The URI scheme is not registered with the system. Only the scanner inside Zerohop reads a pairing code, and it ignores every other code | None known | Manifest and Info.plist check |
| T-18 | A flaw in the SPAKE2 implementation | The crate has no independent audit. It sits behind one interface and runs only inside TLS, QR pairing additionally pins the host's key, and the comparison code is independent of whether SPAKE2 held | A flaw could weaken typed-code pairing until the comparison step | Test vectors; review before release |
| T-19 | A half-finished pairing leaving a phantom member | A pending `Add` is not synced. Without a `WelcomeAck` the host revokes the new device, even after a restart. Joiner keys are fresh per attempt | None known | Dropped-connection and restart tests |
| T-50 | A crafted QR code that sends the joiner elsewhere or makes it try many addresses | The joiner pins the host key from the code, refuses a code with a repeated key or secret or with more than 8 addresses, connects only to local addresses (protocol section 6.1), and tries the next address only before pairing has begun | A code from someone else's screen pairs the phone with that device, which the comparison step and the other device's name reveal (T-16) | Unit tests for QR decoding; attack tests for a wrong host key, a remote address and an address that does not answer |

### 5.3 Hostile or compromised members (A3, A4, A5)

| ID | Threat | Defense | Remaining risk | Tested by |
|---|---|---|---|---|
| T-20 | Writing outside the receive folder | Path components only, name handling in a fixed order so that stripping cannot create `..`, handle-relative writes | None known | Fuzzing and property tests, including `..` with invisible characters |
| T-21 | Overwriting or planting into existing locations | No-replace publishing with no plain-rename fallback; existing directories are never merged | None known | Collision tests, including case and normalization |
| T-22 | Symlink or special-file tricks | Never sent or created; handle-relative opens | None known | Race and symlink tests |
| T-23 | Disguised file names, such as right-to-left overrides | Bidirectional controls are stripped and reserved names are rewritten | Lookalike Unicode letters are still possible | Name normalization tests |
| T-24 | Delivering an executable automatically | Default policy asks for programs, every path component and magic bytes are checked, directories and files are marked before publishing, and nothing is launched | A hostile document in an allowed format. FAT volumes, Android and iOS have no download marks, so programs always ask there | Policy tests, bundle tests |
| T-25 | Filling the disk or exhausting memory | Computed totals, enforced item sizes, free-space checks, rolling per-peer byte and file budgets, one session per peer, bounded manifests and journals | A trusted device can use space up to its budget | Oversized and inconsistent input tests |
| T-26 | Flooding the owner with requests | Rate limits on offers, resumes and clipboard offers | Mild annoyance | Rate limit tests |
| T-27 | A member impersonating another member | Clients pin the exact key, ledger events are signed by their author, and names are shown with a key-derived identifier | Names are self-reported | Member impersonation test |
| T-28 | A member adding a rogue device | A compromised member can add devices, but the new device gains no capability the member lacked. All members are visible, a `Revoke` cuts off later additions, a device cannot inflate its own cutoff, and members added by a revoked device are labeled | Colluding devices can overstate a cutoff. Rogue devices stay until removed, and the owner can revoke again | Ledger property tests and simulation |
| T-29 | A revoked device rejoining | Revocation is monotonic and cannot be undone by any later event. `Add` events after the cutoff are ignored | A member that has not heard of the revocation accepts the device until they sync | Ledger convergence and attack simulation |
| T-30 | A revoked device still interfering | A late `Revoke` it signs can still remove other devices if delivered to a member that has not heard of its revocation. This needs the same network as a member that has not heard of the revocation, and the window closes for each member once it learns | The circle can be damaged, and the owner may need to start a new one. Confidentiality still holds | Documented limitation, simulation |
| T-31 | Forking ledger history by signing conflicting events | Per-author hash chains expose forks during sync, any known fork forces a full exchange of that author's events, and two different events with the same number revoke the author | Members disagree until they have synced with each other | Equivocation tests |
| T-32 | Exhausting ledger limits to block revocation | `Revoke` has its own limit, events past `seq` 128 and third variants are not stored, unknown authors give way, and room is reserved for current members | A member that creates a long chain of new devices can still use up the reserve and force a new circle | Flood tests |
| T-33 | A thief revoking the owner's other devices | Removing a device, turning away one added through another device, and leaving the circle first ask the system to confirm the owner: Touch ID or the login password on a Mac, Face ID, Touch ID or the passcode on iPhone and iPad, a strong biometric or the screen lock on Android, Windows Hello on Windows. A cancelled check, or one that fails to run, revokes nothing. Recovery is a new circle (protocol section 5.6) | A device with no screen lock or Windows Hello asks nothing. The check lives in the app's interface, so code running as the app, and the command-line tool, can still revoke. An unlocked stolen device without a lock can break the circle. Confidentiality still holds | Unit tests: the check comes first, and a cancelled, broken or unknown answer revokes nothing. In the iOS Simulator the passcode prompt named the device being removed, and leaving it unanswered removed nothing |
| T-34 | Reading the private key from a stolen device or a copy of its files | The key is encrypted on disk with a secret held by the system: the login keychain on macOS; on Android a secret wrapped by a key that never leaves the Keystore (StrongBox when the phone has it); on Windows a secret protected by DPAPI for the signed-in account and kept in the local profile, so a roaming profile never carries it to another computer; on iPhone and iPad a keychain item for this device only, which is never synced and cannot be restored onto another device. A secret left in the iPhone keychain by an earlier install is deleted when the app is installed again. A copy of the app's files alone cannot open it | The key has to be in memory to sign, because neither the Secure Enclave nor StrongBox offers Ed25519. Code running as the app, or a device unlocked by an attacker, can still use it. On Windows any program running as the same account can ask DPAPI to open the secret. On Android no screen lock is demanded, so that receiving works in a pocket. On iPhone the secret can be read from the first unlock after the phone starts, also while it is locked again, so that iOS can start the app ahead of time. The command line tool keeps a file only its owner can read | Unit and engine tests: sealed format, wrong secret, tampering, moving an existing key without changing the device. On Windows, unit tests in CI: the secret comes back after a restart, is not stored in the clear, and a damaged or foreign one is reported and left in place. Checked by hand on a Mac, an Android emulator and a Windows computer. In the iOS Simulator the secret came back from the keychain after a restart; not yet on a real iPhone |
| T-35 | Resuming onto different content | The manifest hash is bound, and the source prefix is re-hashed rather than trusting timestamps | None known | Same-size edit resume test |
| T-44 | A removed device still recognizing the others on the network | Each device replaces its discovery key whenever a member leaves, and hands the new one only to devices known to be members | It still sees that some Zerohop device is announcing, as anyone on the network can | Unit tests for rotation and for refusing keys from non-members |
| T-45 | Changing a held item on disk before the owner approves it | Approval re-hashes the item against its journal and refuses on any difference, and the download mark stays | A process already running as the owner can do far worse than this | Attack test that alters a held file |
| T-46 | A device added through another device sending files straight away | Optional setting: such a device is refused on this device until the owner approves it here | Off by default, as the owner chose | Engine test |

### 5.4 Clipboard

| ID | Threat | Defense | Remaining risk | Tested by |
|---|---|---|---|---|
| T-36 | Passwords spreading between devices | Off by default, and content marked secret by password managers is skipped on macOS, Android, Windows, iPhone and iPad. On Windows a marker that is present but cannot be read counts as secret | Secrets copied without any marker, or marked in a way the platform does not expose | Checked by hand on macOS with concealed content and on a Windows computer; no automated test per platform yet, and the Android side has not yet been exercised on a device |
| T-37 | Applied content spreading further | macOS: current-host-only and transient markers, so it stays out of Universal Clipboard and clipboard managers. Android: marked sensitive, so the system preview hides it. Windows: kept out of clipboard history and the cloud clipboard, and marked for clipboard tools to ignore. iPhone and iPad: local only, so it stays out of Universal Clipboard, and marked transient | Other apps on the receiving device can still read the system clipboard. Android has no way to stop other apps or keyboards keeping what they see | Checked by hand on macOS, by looking at the markers on the clipboard, and on Windows, where applied content stays out of clipboard history |
| T-38 | Clipboard data left behind | Memory only, 2 minute lifetime. macOS clears the system clipboard at expiry and on quit if the change counter is unchanged, and Windows does the same with its sequence number, also when Windows shuts down. Native text buffers on Windows are zeroed after use. Android clears it only with the focus and only if its own label is still there. iOS removes it by itself at expiry, through the expiration date given with it. Buffers in the core, the engine and the native boundary are wiped when dropped | The copy in the system clipboard until expiry. On Android, text can stay past 2 minutes if the app is in the background at expiry, until it is next opened. Strings inside Dart cannot be wiped and last until the garbage collector reuses them | Disk audit test over the whole state and download folders; content never in snapshots or history; `Debug` output test |
| T-39 | A member pushing content for the owner to paste, such as a malicious command | Off by default, receive switch per device, per-type switches, 30 offers a minute, paired devices only, a device waiting for approval is left out. Copied files arrive only as a list and move only when the owner pastes them | Pasting without looking at the content | Attack tests for limits and refused types; the remaining risk is a documented limitation |
| T-51 | Copied files serving something else at the paste, or reaching the disk unasked | A pull happens only when the owner pastes, and only for the clip this device was offered. The transfer that answers it must carry exactly the manifest the list named, or it is declined. It lands in a folder of its own, never the receive folder, with every receive rule and download marks, and Explorer copies it on with the marks. A device offers its files only to the devices it sent the list to, only for the clip's two minutes, and a newer copy of anything ends that. On a Mac the list is shown as placeholders by a sandboxed File Provider extension in a hidden place. It holds the list only in memory, rechecks every name, and is reached through a restricted service that only the app managing the place can open; the app hands each file over already open, only for the clip it showed. Each placeholder carries the download mark, which the extension asks the system to keep, so Finder copies it on. The placeholders go when the clip ends, and the app removes any that an earlier run of the extension left | A member can list hostile files that the owner then pastes; programs are not held, because the owner asked for them, but they keep their marks. During the two minutes, a member that got the list can fetch the files without the owner pasting there. Pasted files stay in the app's folder for up to an hour, so a paste still copying is not cut off. On a Mac any program that reads a placeholder, not only a paste, makes the files come, and the system's File Provider database keeps the names of the copied files until the clip ends | Attack tests: a paste fetches exactly what was copied, a clip never offered gives nothing, another manifest is refused, an offer without a pull follows the ordinary rules. Engine tests: nothing reaches the disk before the paste, pasting twice fetches once, pastes asked at once fetch once, the list names each file where the paste puts it, a newer copy ends the offer, the per-device switch keeps files home. Tried by the owner from a Mac to a Windows computer: the files came only with Ctrl+V in Explorer |
| T-47 | Another app on the phone pushing content to the owner's devices through Zerohop | The window that reads the clipboard is not exported. Files shared to Zerohop are only shown in the app, and nothing is sent until the owner taps Send. On iPhone and iPad the Share extension runs only when the owner picks Zerohop in the Share menu; it copies the files into space only Zerohop and the extension reach, and the app shows them the next time it is opened. Shares never opened are removed after a day, and those not sent are dropped on the next start. The entry in the text selection menu, and text from the share sheet, are optional and stay off until the owner turns them on, and every send shows a message. Everything arriving is treated as hostile: only other apps' content addresses are opened, never a `file:` address or one of Zerohop's own, file names are reduced to a single safe name inside their own folder, copying stops when free space runs low, and a malformed request is ignored | With the text entry on, an app on screen can put its own text on the clipboard of the owner's devices, or swap what the owner selected. Android does not say whether the owner tapped. An app can also put something on the clipboard that the owner then sends with the button, or present a misleading file for the owner to send | Checked by hand on an Android emulator: entry absent while off, `file:` and own addresses refused, shared file byte-identical and not sent before the tap. In the iOS Simulator: a photo and a video shared from Photos kept their names, waited until the app was opened, were not sent, and Cancel removed the copies |
| T-48 | The app listening for longer | While the clipboard is on, the listener stays up in the background and after a restart. Nothing is read from a connection before TLS 1.3 with a pinned key, and the receive limits are the same as with the window open. The owner turns it off with one switch | A flaw in the TLS stack would be reachable from the local network for more of the time | Checked by hand on an Android emulator, a Mac and a Windows computer |
| T-49 | Another program on the computer handing files to Zerohop | Files from Explorer's Send to menu, or from a program that starts Zerohop with paths, open a window that names them and lists the owner's devices. Choosing a device sends nothing: only the Send button does, and Cancel drops the files. Only paths that exist are taken, at most 1000 at a time, and a running copy accepts them only in its own tagged message | A program running as the owner can add misleading files for the owner to send, or keep adding them | Unit and widget tests for the choice of device. Send to checked by hand on a Windows computer |

### 5.5 Supply chain (A9)

| ID | Threat | Defense | Remaining risk |
|---|---|---|---|
| T-40 | A malicious or vulnerable dependency | Few dependencies, committed lockfiles, automated advisory and license checks | A flaw not yet known |
| T-41 | Tampering in the build | Releases built only by CI, workflows pinned by commit hash with minimal permissions | A compromise of the CI platform itself |
| T-42 | A substituted download | Signed applications, checksums and build provenance | A user skipping verification on an unofficial copy |
| T-43 | A secret leaking from the repository | Secret scanning; signing keys never in the repository | Human error |

## 6. How this design answers known LocalSend issues

| LocalSend issue | How Zerohop avoids that class of problem |
|---|---|
| [CVE-2025-54792](https://github.com/localsend/localsend/security/advisories/GHSA-424h-5f6m-x63f), discovery spoofing leading to interception | Discovery grants no trust. Only pre-paired keys are accepted, and strangers are rejected during the handshake. |
| [CVE-2025-27142](https://github.com/localsend/localsend/security/advisories/GHSA-f7jp-p6j4-3522), path traversal to arbitrary file write | Paths are lists of validated components, and the receiver alone decides the location. |
| [CVE-2026-25154](https://github.com/localsend/localsend/security/advisories/GHSA-34v6-52hh-x4r4), XSS in the web share page | Zerohop has no web page. |
| Encryption can be turned off, and link sharing uses HTTP | No plaintext mode exists. |

These are design-level comparisons. LocalSend solves a broader problem, accepting transfers from strangers, which Zerohop deliberately does not.

## 7. Verification plan

| Method | Covers |
|---|---|
| Coverage-guided fuzzing with `cargo-fuzz` | Frame decoder, ledger decoder, pairing messages, manifest and path handling |
| Property-based tests with `proptest` | Ledger order independence and monotonic revocation, name handling, address filter, encode and decode round trips |
| Integration attack harness | Forged discovery, interception, relay, unknown keys, cross-ALPN access, downgrade attempts, member impersonation, burned codes |
| Filesystem tests on each OS | Traversal, collisions, merging, symlinks, reserved names, case folding, marking before publishing |
| Known-answer vectors | SPAKE2, key confirmation, comparison code, tags, file hashes, ledger signatures and chains |
| Wycheproof-style checks where applicable | Primitive use through the selected libraries |
| Specification compliance review | Code against this document and the protocol |
| Rust security review and dependency audit | Before each release |
| Independent external audit | Planned. Until it happens, the README must say the project has not been audited. |

## 8. Design review history

| Date | Review | Result |
|---|---|---|
| 2026-11-09 | Automated adversarial design review of the first draft | 27 findings, 2 critical (revocation reinstatement, name handling order). Addressed in this revision. |
| 2026-11-09 | Second adversarial review of the revision | Both critical findings confirmed closed. 2 new high findings and several smaller ones, addressed in this revision. The final edits have not had a further review. |
| 2026-11-09 | [Simulation of the ledger rules](verification/ledger_sim.py) | 20 of 20 attack scenarios held, including order independence over 2000 shuffles and monotonic revocation across all tested event subsets. |
| 2026-11-09 | Implementation testing with a hostile raw client and the real command-line binary | Found and fixed: the program hold could be bypassed by splitting a file's first bytes across frames or by interrupting and resuming; resuming a file with all its segments present could seek past its end; and a removal made by a separate process did not reach a running receiver. Each has a regression test or a scripted check. |
| 2026-11-09 | Automated adversarial review of the implementation, with reproducing tests | 12 findings, 5 high and none critical: folders differing only by case could release a held item, unbounded `SegmentEnd` and sync lists, sequence gaps that silently voided later events, a flood from unknown authors blocking revocation, overlapping writers on one transfer, a crash corrupting a published file, a decoy script disabling the hold, a removal not reaching live sessions after a failed handshake, and four smaller ones. All addressed with regression tests, except that answering pings during a re-hash has no automated test. This round of fixes has not had a further review. |
| 2026-11-09 | Coverage-guided fuzzing of frames, names, pairing links, manifests, the ledger, the file writer and whole hostile sessions, with `cargo-fuzz` | Found that a C1 control character passed name handling, and that signing further variants of an event could evict a stored one and undo its effects, including a removal. Both fixed. Later runs came up clean; counts are in `fuzz/README.md`. |
| 2026-11-09 | Attack tests for interception, relaying a pairing, a middleman who knows the code, TLS 1.2 and classical-only key exchange in both directions | All refused as designed. |
| 2026-11-09 | Second automated review of the implementation, targeting the previous fixes | Found 2 fixes incomplete and 4 further problems: case pairs the Unicode tables do not know could still merge folders, a member could fill the ledger and block its own removal, a fork could push out an `Add` and shrink the ever-member set, unrelated membership updates could end sessions, partial copies on file systems without links, and a folder named like a program excusing its contents. All addressed with regression tests, and a parent-directory sync added to state saves. This round of fixes has not had a further review. |
| 2026-11-09 | Two complete app builds exchanging files over the real network on one Mac, inside the macOS sandbox | Found and fixed: writing the download mark failed under the sandbox, which already sets it, and stopped every transfer; the message explaining a stop could be destroyed by a connection reset; loopback addresses reported by Bonjour were tried first. Automated tests had not shown any of these. |
| 2026-11-09 | The Android build on an Android 16 emulator, receiving from the command-line tool into the shared Download folder | Found and fixed: that storage has no file locks, so every transfer failed, and the sender was given no reason; received files were invisible to other apps until the system was told about them. Discovery cannot be tested on an emulator. |
| 2026-11-09 | `cargo audit` against the RustSec database | No known vulnerabilities in 181 dependencies at that date. |
| 2026-11-11 | The iOS build in the iOS Simulator (iOS 26.1) on a Mac, paired over the real network with a separate copy of the Mac app, with files sent both ways | Found and fixed: the core did not build for iOS, because it used a part of the standard library that exists only on macOS; download marks, which iOS does not have, were attempted on every Apple system; copies made by the iOS file picker were never removed. Discovery, pairing, the keychain secret across a restart, and byte-identical files in both directions worked. The local network permission prompt, keeping the screen on during a transfer, and a real iPhone were not checked. |
| 2026-11-11 | Sending from the iOS photo library in the same setup, through the system photo picker in its `current` mode | A HEIC photo, an HEVC video, a PNG screenshot and one JPEG arrived byte for byte, with no question about photo access. Four sample JPEGs whose stored time zone in the library differed from their EXIF arrived re-encoded: the picker wrote the library's date into them and compressed the image again, and offered no other form of them. Recorded under "Not implemented yet". Copies in the temporary folder were removed after sending. An original held only in iCloud and a real iPhone were not checked. |
| 2026-11-11 | QR pairing: the Android build on the owner's phone reading the code shown by the iOS build in the Simulator, and the iOS build without a camera | The phone's camera read the code and the app checked it, then refused because the phone was already in a circle, as designed. In the Simulator the camera prompt, the message for a missing camera and the host screen with its address worked. A complete pairing by camera between two real devices has not been tried yet. |
| 2026-11-11 | The shared clipboard and the Share menu on iPhone, with two iOS builds in two iOS 26.1 Simulators paired over the Mac's network | Sending first showed iOS's own question about pasting. After refusing, nothing was sent; after allowing, the text arrived on the other iPhone's clipboard and was gone two minutes later. Sending that clip back said it had come from another device, without iOS asking again. A photo and a video shared from Photos waited in the shared space, appeared in the app with their names when it was opened, and Cancel removed the copies. Pictures on the clipboard, content marked secret, an iPad beside another app and a real iPhone were not checked. |
| 2026-11-12 | Copied files fetched only when pasted: attack tests over real TLS between two devices and engine tests with two engines | A paste fetched exactly the files copied and nothing reached the disk before it; a pull naming another clip got nothing, a transfer with another manifest was declined, and an offer straight after connecting stayed under the ordinary rules. Then tried by the owner on their Mac and Windows computer: files copied in Finder arrived only when pasted with Ctrl+V in Explorer, so the Mac app could still read Finder's copies at that moment. Download marks after Explorer's copy, folders and large files were not looked at. |
| 2026-11-12 | Pasting files on a Mac: a probe extension first, then Zerohop's own File Provider extension in a test build on the owner's Mac (macOS 27), with files copied in Explorer on their Windows computer | Finder's ⌘V was the first read of a placeholder, and only then did the files come. A file of 37 MB and a folder of 12 files arrived whole, unlocked, each with the download mark. Found and fixed on the way: read-only placeholders pasted as locked files, a sandboxed app's placeholders never reached the clipboard without security-scoped access, System Settings does not list a hidden place so it could never be switched on, the system dropped the download mark until the extension declared it, and placeholders shown before the extension was restarted stayed behind. Text from Windows still pasted as before. Not looked at: macOS 13 to 26, a paste after the extension was stopped mid-copy, very large folders. |

These are checks performed during development, not an independent audit.

### Not implemented yet

The design above describes more than the first version of the app does today:

- The clipboard (section 5.4) exists on macOS, Android, Windows, iPhone and iPad, for text and pictures. Copied files go from macOS and Windows and can be pasted on Windows and on macOS 13 or later, where the owner first switches on Zerohop's File Provider extension. Phones neither offer nor paste them.
- Pairing QR codes list IPv4 addresses only, and are scanned only on phones; computers show the code and the six digits.
- On Android, sending a copy needs a tap: Android lets only the app in front or the keyboard read the clipboard.
- On iPhone and iPad, files and the clipboard arrive only while Zerohop is open, because iOS does not let it keep listening in the background. Sending the clipboard needs a tap in the app, and iOS asks the owner each time unless they allowed pasting in Settings. Files shared to Zerohop from other apps wait until the owner opens it, because iOS does not let a Share extension open its app; text and links cannot be shared to it, but can be copied and sent as the clipboard. Folders cannot be picked. Photos and videos from the photo library travel as the library holds them, with two exceptions: a Live Photo goes as its still image only, without the moving part, and when the library keeps a different date or time zone for a photo than the file itself says, iOS hands over a re-encoded copy with the library's values written in. Without access to the whole library the app cannot tell such a copy apart, so the owner is not told (protocol section 10.5).
- Journals are limited by age only, not yet to 64 MiB in total.
- Only the Android release is signed with the project's own key, and its certificate fingerprint is published (T-42). The Windows installer and the Mac app run only after the owner gets past the system's warning about an unknown publisher. Publishing the Windows app through the Microsoft Store, which signs what it distributes, is planned. Signing and notarizing the Mac app needs Apple's paid developer programme. See the [code signing policy](code-signing.md). An installation for one user sits in that user's own folder, where anything running as the user could change it, as with other programs installed that way.

## 9. Reporting vulnerabilities

Once the repository is public, security reports go through GitHub private vulnerability reporting, as described in `SECURITY.md`.
