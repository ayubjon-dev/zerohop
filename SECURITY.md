# Security policy

Zerohop has not been independently audited. What it defends against, and what it does not, is written down in the [threat model](docs/threat-model.md).

## Reporting a vulnerability

Please report vulnerabilities privately, through GitHub: open the **Security** tab of this repository and choose **Report a vulnerability**. Do not open a public issue.

Include what you found, how to reproduce it, and which version or commit you tested. Reports in English or Russian are welcome.

## What to expect

Zerohop is maintained by one person, so these are realistic commitments rather than a team's service levels:

- An acknowledgment within 7 days.
- A fix or a mitigation for critical issues within 14 days of confirming them.
- Credit in the release notes, if you want it.

Please give a fix time to reach people before publishing details. If you have not had a reply in 7 days, it is fine to send a reminder.

## Supported versions

Zerohop has no stable release yet. Only the latest code on the main branch receives fixes.

## In scope

- Any way for a device that was not paired to connect, read, or change anything.
- A paired device writing outside the receive folder, or getting around the owner's receive rules.
- Interception or tampering during pairing or transfers.
- Reading the device key or clipboard contents where the threat model says they are protected.
- Memory safety problems in the Rust code.

## Out of scope

- Problems that need an attacker who already has your unlocked device, or code running inside the app. The threat model explains these limits.
- Flaws in the operating system or in an upstream dependency, unless Zerohop uses it incorrectly. Please report those upstream as well.
