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
| A server impersonates one you trust | Host keys are pinned on first contact and confirmed by you. Background connections — tunnel restore, multi-host runs, the desktop's own SSH connection — accept **only** keys you already confirmed: an unknown key there is refused, not trusted silently. A changed fingerprint always stops the connection. If the fingerprint store itself cannot be read, connections are refused rather than treated as first contact, because that is exactly what a substituted key would look like. Multi-host execution skips hosts whose key was never confirmed, including every jump host in the chain. The integration stand is the single documented exception and it is opt-in through the `SEREIN_TRUST_NEW_HOSTS` environment variable, which no release path sets. |
| A malicious server attacks the client | The SSH stack is `russh`, kept current; `cargo audit` runs in CI on every push — over both dependency trees, the app's and the RDP helper's — and fails the build on any new advisory. Sizes announced by a server are checked before memory is allocated for them: an RDP desktop larger than 40 megapixels is refused rather than allocated, query results are capped by rows, bytes and cell size, and file reads stop at their limit instead of trusting the size the server reported. |
| Another local process hijacks the RDP tunnel | The helper reaches the server through a loopback socket the app holds open, and any local process can connect to a loopback port. The helper must present a one-time pass, handed to it on stdin (never on the command line, which is visible to anyone who can list processes) and checked before a single byte is forwarded. |
| The RDP endpoint is not who it claims to be | Trust here comes from the SSH session the traffic runs inside: the desktop connects to the server's own loopback through a channel on a connection whose host key you confirmed. The RDP certificate itself is not validated against a trust store, because RDP servers are almost always self-signed and requiring otherwise would refuse every real server; the server's public key fingerprint is written to the desktop log in the same SHA256 form SSH uses, so a change on a known server is at least visible. |
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

**On its own, Serein makes exactly one kind of request: the update check. Everything else
goes where you point it.**

What you point it at: the SSH servers you configure and whatever you reach through them, and
the network tools you run from your own machine — port check, DNS lookup, HTTP request, TLS
certificate, traceroute, LDAP. Those connect to the host you typed, when you press the button.

The action log is written locally and, **only if you enable it and type an address**, each
entry is also sent to that syslog server (RFC 5424, UDP or TCP, no TLS — use a local relay with
TLS if the path is untrusted). The entry is the same JSON that is written to disk: who, when,
which server, what action. Nothing else leaves the machine for it, and it is off by default.

The update check, in order:

1. A GET to `github.com/neoHaDe/Serein/releases/latest/download/latest.json`.
2. Only if GitHub does not answer: the same manifest from our own mirror,
   `nehade.xyz/updates/terminal/latest.json`.

A manifest holds a version number, release notes and download URLs — GitHub's in the first,
the mirror's in the second. Nothing is downloaded unless you accept the update, and the download
is verified against a minisign signature compiled into the app before anything is installed.
The request carries no identifier of you or your machine beyond what any HTTP request carries.

Settings → **«Закрытый контур: не обращаться в интернет»** turns the update check off — both
the check on startup and the button. The switch is a network policy for the app's *own*
requests, not a sandbox: connections you start yourself still go out. If the settings file
cannot be read, the switch is treated as **on**; failing closed is the safer mistake here.

There is no telemetry, no analytics, no crash reporting, and no account. Nothing to opt out of,
because nothing is collected. This is verifiable: the code is open, and the update check is the
only request the app makes without you asking for it.

## Releases

- Binaries are built locally and published to GitHub Releases with SHA-256 sums in the notes.
- Updates are signed with minisign; the public half is compiled into the app and verified before
  anything is installed. The private key never leaves the release host and is never in CI.
- The Rust toolchain version is pinned (`rust-toolchain.toml`) so the same tag builds the same way.
- An SBOM (CycloneDX) is generated per release: `scripts/make-sbom.sh`.
- **Windows builds are not code-signed.** SmartScreen will warn. Verify the SHA-256 from the
  release notes if that matters to you — that is a real check, whereas a certificate mostly buys
  a quieter dialog.

## Administrator policy

An administrator can fix settings so that users on the machine cannot change them — for example
keep the action log on and forwarded to syslog, keep the app offline, and forbid legacy SSH
algorithms. The policy is taken only from places a user without administrator rights cannot
replace:

- on Windows, the `HKLM\SOFTWARE\Policies\Serein` registry key — writable only by administrators. Use
  the Group Policy template in `docs/policy` (`Serein.admx` with `en-US` and `ru-RU` language files),
  or a single `Policy` string value holding the JSON below. **This is the recommended source.**
- `%ProgramData%\Serein\policy.json` on Windows, `/etc/serein/policy.json` on Linux — **only if the
  file and its folder are protected.** A location alone guarantees nothing: any user can create a
  folder in `ProgramData` and would then own it and could swap the file an administrator later
  puts there. The folder itself is asked of the system (`FOLDERID_ProgramData`), never taken from
  the `ProgramData` environment variable, which any user can set for their own session; if the
  system does not answer, the file is not used and the error is shown in settings. On Windows the file and folder must be owned by Administrators, SYSTEM or
  TrustedInstaller, and no one else may write, delete or change permissions; on Linux they must
  be owned by root and not writable by group or others. Otherwise the file is not applied.

The registry value overrides the file key by key.

```json
{
  "settings": {
    "actionLog": true,
    "actionLogSyslog": { "enabled": true, "host": "siem.corp.local", "port": 514, "protocol": "tcp" },
    "offline": true
  },
  "allowedHosts": ["*.corp.local", "10.0.0.0/8"],
  "forbidLegacySshAlgorithms": true,
  "forbidSavedPasswords": true,
  "requireMasterPassword": true,
  "forbidLocalTerminal": true,
  "forbidSessionRecording": true
}
```

- `allowedHosts` — connections only to these addresses: a full name, all subdomains
  (`*.corp.local`), a subnet or `*`. Checked on **every** path that opens a connection the user
  chose: SSH (including every jump host), telnet and raw TCP, the target of a local port forward,
  each destination requested through a SOCKS5 forward, databases, RDP and VNC, and the utilities —
  port check and range, DNS, TLS certificate, HTTP request, traceroute, LDAP — both from this
  machine and from the server. A refusal is written to the action log. A remote forward (`-R`) is
  not checked: its target is the server's own loopback, chosen by the server, not by the user. The
  address is compared as written; names are not resolved, so a subnet matches only an address
  written as an IP, and a DNS alias of a forbidden host is not caught.
- `forbidSavedPasswords` — passwords and key passphrases are not saved and saved ones are not
  used; they are asked for on connection. Saved ones already on disk are left in place, unused.
- `requireMasterPassword` — the master password cannot be turned off, and secrets are not saved
  until it is on.
- `forbidLocalTerminal`, `forbidSessionRecording` — no local shell, no recording terminal output
  to a file.

`settings` accepts any key of `settings.json`; those keys are shown as locked and changes to them
are dropped before anything is written. The policy is read once at startup. A policy that cannot
be read, is not protected, or cannot be parsed — including an unknown top-level key, so that a
typo does not silently forbid nothing — is not applied at all; the error is shown in Settings and
recorded in the action log.

## Known accepted risks

- `RUSTSEC-2023-0071` (`rsa`, Marvin attack) has no upstream fix and is listed by ID with its
  reasoning in `.github/workflows/ci.yml`. The crate arrives through `russh` and is needed for
  ordinary `id_rsa` keys; the advisory concerns RSA decryption timing, while SSH uses RSA for
  signatures. It is not hidden behind a blanket ignore, and any new advisory still fails CI.
- `cargo audit` also reports warnings that are not vulnerabilities, and they do not fail CI. They
  are listed here so that "no vulnerabilities" is never read as "no findings". Reviewed
  2026-09-15, next review with each release:
  - `glib` 0.18.5, `RUSTSEC-2024-0429` (unsound `VariantStrIter`) and `proc-macro-error`
    (unmaintained, used by `glib-macros`): arrive through GTK on Linux only (`tauri`, `tray-icon`,
    `drag`). Serein never calls `glib` itself and has no `Variant` code; the fix needs
    `gtk-rs` 0.19+, which waits on Tauri moving off GTK 3 bindings.
  - `unic-*` (unmaintained): build-time URL-pattern parsing inside `tauri-utils`, no user input.
  - `serial` 0.4 (unmaintained): pulled by `portable-pty` for its unused serial backend; Serein's
    COM-port support uses `serialport`.
  - `atomic-polyfill` (unmaintained, RDP helper): `heapless` inside smart-card code of
    `sspi`/IronRDP; smart-card login is not used.
- Yanked crate versions are not allowed to stay: `serialport` 4.10.0 and `wnaf` 0.14.0 were
  replaced with 4.10.1 and 0.14.1 as soon as they were noticed.
- GitHub Actions in CI are pinned to commit SHAs, not tags: a tag can be moved to other code, a
  commit cannot. Dependabot proposes updates monthly as reviewable pull requests.
- Legacy mode (`sshLegacyAlgos`) deliberately offers old algorithms — `ssh-rsa`, CBC ciphers,
  `hmac-sha1`, DH group1 — because otherwise old network hardware cannot be reached at all. They
  are appended at the end of each list, so a modern server still negotiates a strong set, and
  they are never offered unless the profile asks.
