# Fuzzing

Coverage-guided fuzz targets for everything that handles data from another device. They need the nightly toolchain and `cargo-fuzz`.

```bash
cargo +nightly fuzz run --sanitizer none session -- -max_len=4096 -len_control=0 -timeout=30
```

`--sanitizer none` is used because the project's own crates forbid `unsafe`, and it roughly doubles the speed.

| Target | What it exercises | What must hold |
|---|---|---|
| `frames` | Frame reading and every message decoder | No panic, limits respected, decoded values re-encode to the same values |
| `names` | Name handling for Windows, macOS and other systems | Results are safe to give to the file system and stable when handled twice |
| `links` | Pairing links | No panic, valid links round-trip |
| `manifest` | Offers built from awkward names | No false totals, duplicates, or names that collide |
| `ledger` | Validly signed events in any order | A removal is never undone; the result does not depend on order; two sync rounds converge |
| `writer` | The receive path in any order, with interruptions | Nothing outside the folder, nothing overwritten or merged, no disguised program published, only verified bytes published |
| `session` | A whole session against a device that sends anything | The same, plus no panic or hang |

Under the fuzzer, `SEGMENT_SIZE` is 32 bytes so that short inputs can span several segments.

## Last run

Three minutes per target on the code of 2026-11-09 that includes the additions for the app (discovery keys, cancelling, per-device modes, held items), with no crashes:

| Target | Inputs tried |
|---|---|
| `frames` | 6.2 million |
| `names` | 1.2 million |
| `links` | 9.0 million |
| `manifest` | 4.5 million |
| `ledger` | 77 thousand |
| `writer` | 79 thousand |
| `session` | 122 thousand |

The code before those additions had ten minutes per target, also with no crashes.

After the shared clipboard was added, `session` ran for three minutes (98 thousand inputs, with structured clipboard steps) and `frames` for two (2.9 million, with the clipboard messages), with no crashes. The other targets do not touch that code and were not rerun.

Earlier runs found two real bugs, recorded in `docs/threat-model.md`. Runs of this length are a short campaign, and a clean run shows only that these inputs found nothing.

Pairing is covered through its message decoders and the attack tests. A hostile peer deep inside the pairing exchange is not fuzzed, because reaching it needs the shared secret.
