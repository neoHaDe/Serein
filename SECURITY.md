# Security Policy

## Reporting a vulnerability

Email **hadegamingon@gmail.com** with `SEREIN SECURITY` in the subject.

- **First response: within 5 working days.** If you do not hear back, resend — do not assume
  the report was received and ignored.
- Please do not open a public issue for a vulnerability. There is no bug bounty; there is a
  credit in the release notes if you want one.
- If you send a proof of concept, say so explicitly and describe what it does before I run it.

Supported versions: **the latest release only.** This is a one-person project, and pretending
to backport fixes to older lines would be a promise nobody can keep.

## What Serein protects, and from whom

Serein is a desktop SSH/SFTP client. It stores the credentials you give it and connects to
servers you choose. The threat model follows from that, and stating it plainly is more useful
than a longer list of things nobody checks.

### In scope

| Threat | What is done about it |
|---|---|
| Another user of the same machine reads your saved credentials | Secrets never sit in plain text: Windows uses DPAPI, Linux the Secret Service via `keyring`, and a master password (scrypt + AES-256-GCM) covers both. The profile directory is `0700` and its files `0600` on Unix; on Windows a generated private key gets an explicit ACL. |
| A backup file leaks | Password-encrypted backups use scrypt with parameters recorded inside the packet, so old backups keep opening while new ones get the stronger settings. AES-256-GCM — a modified packet fails to decrypt rather than decrypting into garbage. |
| A server impersonates one you trust | Host keys are pinned on first contact. A changed fingerprint stops the connection; when there is no user to ask (tunnel restore, multi-host run), it is refused outright rather than trusted silently. Multi-host execution skips hosts whose key was never confirmed, including every jump host in the chain. |
| A malicious server attacks the client | The SSH stack is `russh`, kept current; `cargo audit` runs in CI on every push and fails the build on any new advisory. |
| A mass operation does more than intended | Multi-host runs show the command and the host list before executing, and report per host. Deleting a folder says it takes the contents with it. |
| Credentials leaking into the UI layer | The frontend receives handles, not secrets. Secrets stay in the Rust side. |

### Out of scope

Being honest about this is part of the model, not a disclaimer:

- **An attacker who already controls your user account.** They can read what your session can
  read. No desktop client can fix that.
- **Malware with debugger rights on the running process.** Keys must exist in memory to open a
  connection. The master key is wiped after use; that raises the cost, it does not remove it.
- **The servers you connect to.** Serein does not audit them.
- **Physical access with an unlocked screen.**

## What Serein sends over the network, and where

**Only to the servers you configure — plus one exception, which you can turn off.**

The exception is the update check: a GET to `nehade.xyz/updates/terminal/latest.json`, which
returns a version number, release notes and signed download URLs. It carries no identifier of
you or your machine beyond what any HTTP request carries. Settings → **«Закрытый контур: не
обращаться в интернет»** disables it, including the check on startup, and the app then makes no
outbound requests at all.

There is no telemetry, no analytics, no crash reporting, and no account. Nothing to opt out of,
because nothing is collected. This is verifiable: the code is open, and there is exactly one
call to an external host in it.

## Releases

- Binaries are built locally and published to GitHub Releases with SHA-256 sums in the notes.
- Updates are signed with minisign; the public half is compiled into the app and verified before
  anything is installed. The private key never leaves the release host and is never in CI.
- The Rust toolchain version is pinned (`rust-toolchain.toml`) so the same tag builds the same way.
- An SBOM (CycloneDX) is generated per release: `scripts/make-sbom.sh`.
- **Windows builds are not code-signed.** SmartScreen will warn. Verify the SHA-256 from the
  release notes if that matters to you — that is a real check, whereas a certificate mostly buys
  a quieter dialog.

## Known accepted risks

- `RUSTSEC-2023-0071` (`rsa`, Marvin attack) has no upstream fix and is listed by ID with its
  reasoning in `.github/workflows/ci.yml`. The crate arrives through `russh` and is needed for
  ordinary `id_rsa` keys; the advisory concerns RSA decryption timing, while SSH uses RSA for
  signatures. It is not hidden behind a blanket ignore, and any new advisory still fails CI.
- Legacy mode (`sshLegacyAlgos`) deliberately offers old algorithms — `ssh-rsa`, CBC ciphers,
  `hmac-sha1`, DH group1 — because otherwise old network hardware cannot be reached at all. They
  are appended at the end of each list, so a modern server still negotiates a strong set, and
  they are never offered unless the profile asks.
