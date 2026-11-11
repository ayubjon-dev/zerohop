# Performance measurements

These numbers show what the code does on one machine. They are not a claim about real networks.

## Setup

- Date: 2026-11-09
- A Mac with an Apple M4, 16 GB of memory, internal SSD
- `cargo build --release`, with sender and receiver as separate processes of the command-line tool
- Connection: loopback (TCP over 127.0.0.1, full TLS 1.3 with the hybrid key exchange), enabled only by the `dev-loopback` feature
- Data: random bytes, so nothing is compressible
- Sender and receiver share the same CPU cores and the same disk, which makes this slower than two real devices would be for CPU and disk, and faster for the network

## Results

| Test | Result |
|---|---|
| One 2 GiB file, three runs | 628, 835 and 847 MiB/s |
| Plain `cp` of that file on the same disk | about 1.2 GiB/s |
| 5000 files of 4 KiB in one folder | 0.59 s, about 8500 files per second |
| Sender CPU for 2 GiB | about 2.0 s (1.35 s user, 0.65 s system) |
| Receiver CPU for 2 GiB | about 2.3 s |
| Sender peak memory | 20 to 25 MiB |
| Receiver peak memory | 31 MiB (34 MiB with 5000 files) |
| Receiver memory just after starting | 4.1 MiB |
| Receiver memory after those transfers, idle | 29 to 34 MiB |

Every transfer was compared byte for byte with its source afterwards.

The table comes from runs with the machine otherwise quiet. A later round of security fixes did not change the code that moves data. Repeating the measurements afterwards, while other applications kept the machine busy (the CPU time for the same work roughly doubled), gave 215 to 598 MiB/s for the 2 GiB file over nine runs and 0.9 to 4.1 s for the 5000 files over three. Expect the speed to depend on what else the computer is doing.

The queues between the socket, the encryption and the disk were reduced from 64 and 128 chunks to 16 and 12 after measuring. Throughput stayed within the variation between runs, and the receiver's peak memory fell from about 120 MiB to 31 MiB.

## Interrupted transfers

With a 1.5 GiB file, each side was killed with `SIGKILL` part way through, then the transfer was continued with `zerohop resume`. How much is resent depends on when the kill lands:

| Killed | Resent | Partial file visible in the folder | Result |
|---|---|---|---|
| Sender | 1008 MiB | No | Identical to the source |
| Receiver | 976 MiB | No | Identical to the source |

## Compared with LocalSend on a real phone

- Date: 2026-11-09
- Sender: the Mac above, on Wi-Fi. Receiver: Samsung SM-S928B, on the same Wi-Fi network. Direction: Mac to phone.
- LocalSend 1.18.2 with its encryption on and Quick Save on, so that no one had to accept anything. Zerohop 0.1.0 in its default mode.
- Each app in turn, alternating, so that changes in the Wi-Fi affect both alike. The data was random, so nothing could be compressed.
- Time runs from the first sustained traffic on the Mac's Wi-Fi interface. For Zerohop it ends when the sender is told that the phone has finished. For LocalSend it ends with the last traffic above 1 MB/s, because it does not report a finish time.
- CPU and peak memory are for the app on the Mac only.

| Test | LocalSend | Zerohop |
|---|---|---|
| One 512 MiB file, seconds per run | 54.2, 45.9 | 38.8, 57.8, 54.2 |
| 5000 files of 64 KiB (320 MiB), seconds per run | 127.5, 102.2 | 58.2, 54.5 |
| Peak memory on the Mac | 121 to 136 MB | 100 to 125 MB |
| CPU for the 512 MiB file | 9.6 and 12.5 s | 1.9 s |
| CPU for the 5000 files | 43.9 and 55.0 s | 16.6 and 22.8 s, before the fix below |

With many small files Zerohop took about half the time. With one large file the two were level: the difference between runs of the same app was larger than the difference between the apps, and the Wi-Fi speed of about 10 to 14 MB/s was probably the limit.

The two large-file runs in the first round showed Zerohop using 16 to 24 s of CPU. Nearly all of that was the interface rebuilding its theme on every progress update. After fixing it, the third large-file run used 1.9 s. The 5000-file test was not repeated after the fix.

This is one Mac, one phone and one network, with two or three runs each. It does not show what happens on other devices or in the other direction.

## Not measured yet

- What the network itself can carry, so the goal of reaching at least 90% of it cannot be confirmed.
- Sending from the phone to the Mac, for speed.
- Windows, iPhone and older computers.

## Working in the background

Measured on 2026-11-09 with the shared clipboard on and nothing happening:

| Where | Memory | CPU |
|---|---|---|
| Mac, window closed | 112 MB resident | 0.0 to 0.1% |
| Android 16 emulator, started by the service after a restart, no screen ever opened | 58 MB proportional (170 MB resident) | 0.0% |
| Android 16 emulator, after the interface had been open | 80 MB proportional (211 MB resident) | not measured |

These are from a Mac and an emulator. Battery use on a real phone has not been measured.

Measured again on 2026-11-11 on the same Mac, now that closing the window lets the window and the interface go while the Flutter engine, Dart and the core keep running:

| Mac | Memory (footprint) | Resident | CPU |
|---|---|---|---|
| Window open, after the first half minute | 88 MB | 77 MB | not measured |
| Window closed after it had been open | 62 MB | 67 MB | 0.0% |
| Started without a window, as at login (simulated by a test build) | 36 MB | not measured | not measured |

The footprint is what Activity Monitor shows as Memory. For the first half minute with the window open, drawing takes up to about 280 MB, mostly graphics memory, which the renderer gives back afterwards. After the window has been open once, the engine keeps about 15 MB of graphics memory. Windows cannot let the interface go without stopping the engine, because its Flutter embedding offers no other way, so it is unchanged. On Android the service already runs without the interface; it now passes the system's requests to free memory on to Flutter while no screen is open, which has not been measured.

All of these are above the 30 MB target for an idle service, because a Flutter engine stays loaded. Reaching it needs a background mode without Flutter.
