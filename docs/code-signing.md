# Code signing policy

Zerohop's Windows builds are being set up for signing through [SignPath Foundation](https://signpath.org)'s free programme for open-source projects. Until SignPath has accepted the project, the program and its installer are not signed, and Windows warns before running them. This page says what will be signed, by whom, and how.

## What is signed

- `zerohop.exe` and the Windows installer.
- Both are built by the [release workflow](../.github/workflows/release.yml) on GitHub-hosted runners, from a version tag in this repository. Nothing built anywhere else is signed, and the build scripts and CI configuration are part of the source here.
- The libraries shipped with the program are not signed separately: the Flutter engine, the plugins and Zerohop's core library. The Microsoft C++ runtime keeps Microsoft's own signature.
- Every signed file carries the product name Zerohop and the same product version as the rest of its release.

## Team roles

| Role | Who | Trusted to |
|---|---|---|
| Authors | [ayubjon-dev](https://github.com/ayubjon-dev) | Change the source without further review |
| Reviewers | [ayubjon-dev](https://github.com/ayubjon-dev) | Review every change proposed by anyone else |
| Approvers | [ayubjon-dev](https://github.com/ayubjon-dev) | Approve each signing request, and decide which releases are signed |

Everyone in these roles uses multi-factor authentication for GitHub and SignPath.

## How a release is signed

1. A version tag starts the release workflow.
2. The workflow builds Zerohop and sends `zerohop.exe` to SignPath.
3. An approver checks the request and approves it by hand. Nothing is signed without that.
4. The workflow builds the installer around the signed program and sends the installer to SignPath. An approver approves that request too.
5. The workflow drafts a GitHub release with the installer and its SHA-256 checksum. An author checks the draft and publishes it.

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
