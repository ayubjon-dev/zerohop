# Zerohop

Private file transfer between your own devices, directly over your local network. No account, no cloud, no server.

[Русская версия](README.ru.md)

> **Zerohop has not been independently audited.** It was built with security as the first goal and is heavily tested, but no outside party has reviewed it yet. Do not rely on it where a flaw could cause serious harm.

## What it does

- **Sends files and folders** between devices you have paired, byte for byte. Interrupted transfers continue where they stopped.
- **Pairing happens once.** You type a six-digit code and compare a number on both screens. After that, every device in your circle recognizes the others.
- **Shared clipboard**, off by default: copy text or a picture on one device and paste it on another.
- **Works in the background** while the clipboard is on, including after a restart.
- **You decide what arrives**, per device: always ask, accept automatically, accept automatically except programs, or refuse.

## Platforms

| Platform | State |
|---|---|
| macOS | Works |
| Android | Works. Sending the clipboard takes one tap, because Android does not let apps read it in the background |
| Windows | File transfer works. The shared clipboard, background mode and an installer are still to come |
| iPhone and iPad | In progress. Sending and receiving files works in the iOS Simulator; not yet tried on a real iPhone. Files arrive only while the app is open, because iOS does not let it listen in the background. No shared clipboard yet |

There are no published releases yet. To try it, build it from source.

## Security in brief

- Only paired devices can connect. Devices are identified by pinned keys, and a stranger is refused during the handshake.
- All traffic uses TLS 1.3 with a post-quantum hybrid key exchange. There is no unencrypted mode and no web page.
- Names from the network are validated and files can only land inside your receive folder. Programs can be held back for your approval.
- Each segment of a file is verified with BLAKE3 before it is accepted.
- The device key is encrypted on disk with a secret held by the macOS or iOS keychain, the Android Keystore or Windows data protection (DPAPI).
- Zerohop keeps clipboard contents in memory only, removes them after two minutes, and never writes them to disk or to the transfer history. Content marked secret by password managers is skipped.

### Known limits

- The device key has to be in memory while the app runs, because the secure chips in Macs and phones do not offer Ed25519.
- Someone holding your unlocked device can use it as you, and can remove your other devices.
- On Android, copied text can stay on the clipboard past two minutes if Zerohop is in the background when it expires.
- Other apps on the receiving device can read what arrives on its clipboard.
- With the optional “To Zerohop” text entry turned on, an app on the phone's screen can send its own text to your clipboard. It is off by default.

The full list of threats and what is done about each one is in the [threat model](docs/threat-model.md).

## Building from source

You need Rust (stable), Flutter 3.47, and for each target:

- **macOS:** Xcode
- **Android:** JDK 21 and the Android SDK with NDK 28.2
- **Windows:** Visual Studio 2022 with the “Desktop development with C++” workload
- **iPhone and iPad:** Xcode with the iOS platform, and the Rust targets `aarch64-apple-ios` and `aarch64-apple-ios-sim`. `flutter build ios --simulator` also needs `x86_64-apple-ios`

```bash
cd app
flutter build macos --release
flutter build apk --release --target-platform android-arm,android-arm64
flutter build windows --release
flutter run -d "iPhone 17 Pro"   # in the iOS Simulator
```

There is also a command-line tool:

```bash
cargo run -p zerohop-cli -- --help
```

## Running the tests

```bash
cargo test --workspace --features zerohop-core/dev-loopback,zerohop-app/dev-loopback
cargo clippy --workspace --all-targets --all-features -- -D warnings
cd app && flutter test
```

The fuzz targets are described in [fuzz/README.md](fuzz/README.md).

## How the code is organized

| Path | What is there |
|---|---|
| `crates/zerohop-core` | Protocol, pairing, encrypted sessions, transfers, receive rules |
| `crates/zerohop-app` | The application layer shared by every interface |
| `crates/zerohop-ffi` | The only crate allowed to use `unsafe`: four functions at the boundary with the app, and on Windows the calls that protect the device key's secret |
| `crates/zerohop-cli` | The command-line tool |
| `app` | The Flutter interface and the native code for each platform |
| `fuzz` | Coverage-guided fuzz targets |
| `docs` | [Protocol](docs/protocol.md), [threat model](docs/threat-model.md), [measurements](docs/benchmarks.md) |

## Reporting a vulnerability

Please do not open a public issue. See [SECURITY.md](SECURITY.md).

## License

[Apache License 2.0](LICENSE).
