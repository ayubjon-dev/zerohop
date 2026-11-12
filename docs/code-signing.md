# Code signing policy

This page says what is signed in a Zerohop release, by whom, and how.

Every release is built by the [release workflow](../.github/workflows/release.yml) on GitHub-hosted runners, from a version tag in this repository. Nothing built anywhere else is signed or published, and the build scripts and CI configuration are part of the source here.

## What is signed

**Windows.** Nothing is signed yet, so Windows warns before running the installer.
- The plan is to publish Zerohop in the Microsoft Store as well, which signs what it distributes.
- The release workflow also has steps that sign `zerohop.exe` and the installer through [SignPath Foundation](https://signpath.org)'s programme for open-source projects. They stay idle unless that is set up, as described at the end of this page.
- The program and the installer already carry the product name Zerohop and the same product version, as signed files must.

**Android.** The APK is signed with the project's release key, which only the release workflow uses, from the repository's secrets. The workflow fails a build signed with any other key. The key's certificate has this SHA-256 fingerprint:

`DB:A4:D0:C6:D9:55:C2:98:86:90:82:11:AB:5A:5E:AD:03:3A:C9:7D:31:16:6F:5B:AE:78:D8:E5:81:4E:4E:FE`

Android installs an update only when it is signed with the same key, so a copy of Zerohop signed with anything else cannot replace an installed release.

**macOS.** The app is signed ad hoc and is not notarized: signing it with a Developer ID needs Apple's paid developer programme. macOS therefore refuses to open it until the owner allows it in System Settings. Each update asks once more for the login keychain password that unlocks Zerohop's device key.

## Team roles

| Role | Who | Trusted to |
|---|---|---|
| Authors | [ayubjon-dev](https://github.com/ayubjon-dev) | Change the source without further review |
| Reviewers | [ayubjon-dev](https://github.com/ayubjon-dev) | Review every change proposed by anyone else |
| Approvers | [ayubjon-dev](https://github.com/ayubjon-dev) | Approve each signing request, and decide which releases are signed |

Everyone in these roles uses multi-factor authentication for GitHub.

## How a release is signed

1. A version tag starts the release workflow. The tag has to match the version in `app/pubspec.yaml`.
2. For Windows, the workflow builds the installer. It is not signed today. With SignPath set up, it would first send `zerohop.exe` and then the installer to SignPath, each request approved by hand.
3. For Android, the workflow signs the APK with the release key and checks that the signature is from that key.
4. For macOS, the workflow puts the app in a disk image.
5. The workflow drafts a GitHub release with the Windows installer, the disk image, the APK and their SHA-256 checksums. An author checks the draft and publishes it.

## Privacy

Zerohop has no servers, accounts, analytics, telemetry or update checks.

- **What it sends, and where.** It sends data only to devices you paired with it, over encrypted connections, and only what you send. The shared clipboard is off until you turn it on, and each device has its own switches.
- **How your devices find it.** It announces itself on the local network with DNS-SD over mDNS. The announcement holds a random name and a rotating tag, never your device's name. Turn off "Visible on the network" in Settings to stop it.
- **Components.** None of the libraries it ships with sends anything anywhere else.

## What it changes on your computer

- **The installer** puts Zerohop in your programs folder, adds it to the Start menu, and adds a desktop shortcut if you ask for one.
- **The program** adds itself to Explorer's Send to menu. With background mode on, it also starts when you sign in to Windows.
- **Uninstalling** from Settings → Apps removes all of that. It asks before removing this computer's Zerohop key, the list of your devices and the settings, and keeps them unless you say otherwise. Files you received are never removed.

## Setting up signing

The maintainer does this once. The repository already holds everything else.

**Android.** Store the release key in the repository's secrets: `ANDROID_KEYSTORE`, the key file in base64, and `ANDROID_KEYSTORE_PASSWORD`. The key's alias is `zerohop`, and the key and its file share one password. Keep a copy of the key and the password offline: if they are lost, installed copies can never be updated again.

**Windows, through SignPath (not in use).** Should the project use SignPath Foundation after all:

1. Publish a first release, unsigned: SignPath Foundation signs only projects that are already released.
2. Apply on [SignPath's page for open-source projects](https://about.signpath.io/product/open-source), naming this repository.
3. Turn on multi-factor authentication on GitHub and in SignPath, for everyone in the roles above.
4. Once the project is accepted, set it up in SignPath:
   - two artifact configurations, `windows-program` and `windows-installer`, from the files in [`.signpath/artifact-configurations`](../.signpath/artifact-configurations);
   - in the signing policy: manual approval of every request, and the product name `Zerohop` required in file metadata;
   - GitHub as the trusted build system for this repository.
5. In this repository's settings, add:
   - the secret `SIGNPATH_API_TOKEN`;
   - the variables `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG` and `SIGNPATH_SIGNING_POLICY_SLUG`.
6. Add SignPath's attribution line to this page and to the release notes, as its terms ask.

From then on, a version tag builds a signed release, and it waits for an approver at each of the two signing steps.
