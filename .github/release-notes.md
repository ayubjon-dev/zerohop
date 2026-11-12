Draft made by the release workflow from `{tag}`. Check the files and edit these notes before publishing.

## Installing

**Windows 10 or later:** run `Zerohop-{version}-windows-x64-setup.exe`. It installs for you alone with no administrator rights; its first window offers all users instead. Until the installer is signed, Windows SmartScreen warns first: choose More info, then Run anyway.

**macOS 12 or later:** open `Zerohop-{version}-macos.dmg` and drag Zerohop to Applications. The app is not notarized by Apple, so macOS refuses to open it the first time: open System Settings → Privacy & Security and choose Open Anyway. On its first start macOS asks for your login keychain password, which unlocks Zerohop's device key; it asks again after each update until the app has a stable signature. To paste files copied on other computers, which needs macOS 13 or later, switch Zerohop on under File Providers when the app asks.

**Android 7 or later:** open `Zerohop-{version}-android.apk` on the phone and allow your browser or file manager to install apps. If you had a test build of Zerohop installed, remove it first: this one is signed with the project's release key, whose certificate has the SHA-256 fingerprint `{android_cert}`.

iPhone and iPad are not part of this release yet.

## Checking the files

Their SHA-256 checksums are in `SHA256SUMS`. Releases are built only by GitHub Actions, from the tag above.

[Code signing policy](https://github.com/ayubjon-dev/zerohop/blob/main/docs/code-signing.md)
