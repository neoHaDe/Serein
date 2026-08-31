## Serein v1.2.7

Both SSH-layer advisories are closed, the file manager finally works on servers that have no
SFTP subsystem at all, and the server list learned tags, environment labels and multi-select.

### Security

- **`russh` 0.45 → 0.63.** Closes RUSTSEC-2026-0153 and RUSTSEC-2026-0154, the two DoS
  advisories that 1.2.6 shipped with. The `cargo audit` ignore list is down from three entries
  to one: `rsa` (RUSTSEC-2023-0071), which has no upstream fix and is needed to read ordinary
  `id_rsa` keys.
- **Crypto backend is `ring`, not the `aws-lc-rs` default.** The default builds through
  `aws-lc-sys`, which needs a C toolchain, cmake and NASM — that would have broken the Windows
  build, the build VM and CI, and with them the reproducibility this project promises.
- **SBOM** (CycloneDX 1.5) is published with the release, and `cargo audit` / `npm audit` now
  run in CI on every push.
- **Threat model** in `SECURITY.md`: what is protected, from whom, and — in its own section —
  what is *not* promised.
- **Offline mode**: one switch turns off the update check at startup and every outbound
  request. There is exactly one outbound request in the whole codebase; it is named in the
  document.
- **Master password** must be at least 12 *characters* (not bytes — «пароль» would have passed
  a byte check), with no composition rules, following NIST SP 800-63B rather than the
  mandatory-special-character habit that produces `P@ssw0rd1`.

### File manager on servers without SFTP

Old switches and stripped-down images often ship without `Subsystem sftp`. Serein has had an
SCP fallback since 1.2.6, but nothing ever exercised it. A test server with the subsystem
removed found four bugs on the first run, and each one meant the fallback did not work at all:

- **The subsystem probe never read the server's refusal**, so the SFTP handshake waited for a
  reply that would never come. The file manager hung forever instead of switching to SCP.
- **Downloading and opening files hung forever.** The client waited for a byte that remote
  `scp -f` only sends in response to ours — both sides waiting for each other.
- **File size was always reported as zero** (`wc -c < -- file` makes the shell try to open a
  file literally named `--`), so the "file too large to edit" guard never triggered and a
  multi-gigabyte log would be pulled into memory.
- **Server errors were swallowed.** Warning and error codes counted as success and their text
  was never read, leaving the stream out of step; you got `SCP: неверный ack (115)` instead of
  the reason.

### Server list

- **Tags, favourites and environment labels** (`prod` / `stage` / `dev`). The search box
  understands `tag:web`, `env:prod` and `fav`, and they combine. Production servers carry a red
  badge in the list — visible before anyone runs something on them.
- **Multi-select** with Ctrl and Shift, as in a file manager. The context menu then applies to
  the whole selection: connect to all, label the environment, add to favourites, move to a
  group, delete.
- **Import from MobaXterm, XShell and SecureCRT** (PuTTY and `~/.ssh/config` were already
  there). Passwords are deliberately *not* imported: they are decryptable by design in those
  formats, and copying them into a second store would spread the problem.
- **Server overview page**: CPU, RAM, disk, load, network, uptime, OS and kernel, process
  count, failed services and Docker health — one screen per server.
- Local utilities that save a round trip to the browser: TLS certificate check, port test, DNS
  query, subnet calculator, hashes, JWT decode.

### Reliability

- **Auth failures are no longer retried.** A wrong password was retried five times in a row,
  which is how you trip fail2ban or lock an Active Directory account. Failures that cannot be
  fixed by trying again — bad credentials, unknown host key — now stop at the first attempt;
  network failures still retry.
- **A dead session says why it died.** The failure phase (connect / auth / jump / shell) travels
  with the error instead of being flattened into one string.
- Deleting a non-empty folder over SFTP works. It never did: `remove_dir` only removes empty
  directories, and the user saw `Failure: Failure`.
- Two hosts confirmed at the same time no longer lose one of the `known_hosts` entries.
- Sessions are owned by the backend, not by per-window bookkeeping, so a tab moving between
  windows cannot orphan a connection.
- Config schema is versioned: the profile is copied before any migration, and a profile from a
  newer version is refused rather than silently rewritten.

### Testing

- **A stand of real SSH servers** in Docker (Debian, Alpine/BusyBox, and one with the SFTP
  subsystem removed) with 22 integration tests: password, key, jump chain, host key change,
  server-side disconnect, SFTP edge cases and the SCP fallback. It runs in CI.
- **90 frontend tests** where there were none, covering reconnect rules, tab handling, the
  command palette, the server filter and selection.
- CI went from 17–22 minutes to about 2.

### Install

1. **Serein_1.2.7_x64-setup.exe** — Windows NSIS installer (RU+EN).
2. **Serein_1.2.7_x64-portable.exe** — Windows single exe. Profile in `%APPDATA%\serein`.
3. **Serein_1.2.7_amd64.deb** — Linux package (`/usr/bin/serein`, config in `~/.config/serein`).
4. **Serein_1.2.7_amd64.AppImage** — portable Linux binary.

In-place upgrade from any 1.x on Windows and from 1.2.5 on Linux. The AppImage is large
(~100 MB) because it carries the whole WebKitGTK stack; the `.deb` uses the system one.

SHA-256:

```
823def4af03d5d34a21bbeda47d6b7ea5a5c549da86d9a0e77bdb04baade6b85  Serein_1.2.7_x64-setup.exe
619353c29d8467aef19ea659e677b48443d4a593977903636293f39c9694516b  Serein_1.2.7_x64-portable.exe
ca3601275c8d41ce198443d36f1eae52b04bdf4a320aee8bdfe9d94826073288  Serein_1.2.7_amd64.deb
c250c36e2eb940f4a7e1065f4d191f98ab3e9e3a008d199d7c108bae9ad06fed  Serein_1.2.7_amd64.AppImage
```

### Limitations

- The Windows build is **unsigned**. SmartScreen will complain; the SHA-256 above is the check
  that actually means something.
- `rsa` (RUSTSEC-2023-0071) has no upstream fix and stays in the audit ignore list.
- No external penetration test or independent review of the crypto layer has been done yet.
- Telnet has not been run against real network hardware — only against the emulator from 1.2.4.
- Backup restore Windows→Linux does not rewrite absolute key paths.
- Drag-and-drop out of SFTP to the desktop is Windows-only.
