<div align="center">

<img src="src-tauri/icons/128x128.png" width="96" alt="Serein icon" />

# Serein

**Desktop client for servers and network gear — everything in one window.**

SSH, SFTP and SCP with an editor, serial consoles, telnet and raw TCP.
Tabs and split panes, port forwards, resource monitoring, a Docker panel
and a local terminal — in an installer of about **7.8 MB**.

Free, open source, Apache 2.0. Windows x64 and Linux x64, **v1.2.7**.

[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-stable-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![React](https://img.shields.io/badge/React-18-61DAFB?logo=react&logoColor=black)](https://react.dev)
[![TypeScript](https://img.shields.io/badge/TypeScript-5-3178C6?logo=typescript&logoColor=white)](https://www.typescriptlang.org)
[![CI](https://github.com/neoHaDe/Serein/actions/workflows/ci.yml/badge.svg)](https://github.com/neoHaDe/Serein/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](#license)

Windows · no bundled Chromium (system WebView2) ·
[Russian version](README.ru.md)

### → [Download the latest release](../../releases/latest)

<img src="docs/screenshot.png" width="820" alt="Serein screenshot" />

</div>

---

## Why Serein

The UI runs in the system WebView. SSH, SFTP, crypto, and the PTY live in one Rust binary.
No Chromium tax. We are not racing Tabby on feature count. The point is a **server workspace**
(terminal, files, Docker, logs, resources, tunnels), not "yet another SSH client".

| | **Serein (Tauri)** | Typical Electron client |
| --- | :---: | :---: |
| Installer size | **≈ 7.8 MB** | ≈ 85 MB |
| Idle RAM | **≈ 33 MB** | 150–250 MB |
| SSH engine | pure Rust [`russh`](https://github.com/Eugeny/russh) | libssh2 / native |
| Runtime | system WebView2 | full Chromium |

Numbers come from a live `tauri dev` session (RAM) and the 1.2.7 NSIS build (~7.8 MB packed).
A weak laptop will not magically match 33 MB.

---

## Features

### Server Workspace (v1.1.0)
- **Server workspace rail** on SSH tabs: Terminal, Docker, Logs, Processes, Services, Tunnels
- **Processes** — `ps` table plus CPU/RAM/disk metrics; **Docker** — compact rows, properties via right-click
- **SFTP** — side panel from TabBar; server list collapses on connect
- **Detach** a tab or workspace panel; **reattach** to main (SSH session stays alive)

### Terminal and UX
- **Multiple SSH tabs** and **split panes** (tree with drag-resize, pick a server per pane)
- **Local terminal** with a sensible shell (PowerShell → cmd, or a custom / WSL shell)
- Search (`Ctrl+F`), zoom, **17 UI themes**, **compact mode**
- **Custom window chrome** (no Windows caption): minimize / maximize / close
- **Broadcast input** (current tab only), tab restore, tab drag
- **Ctrl+Shift+C / Ctrl+Shift+V** — copy/paste in the terminal (not WebView2 DevTools)
- SSH output batching; 4 MiB buffer with warning when overloaded
- Keyboard pane navigation, remappable shortcuts, **command palette** (`Ctrl+Shift+P`)

### Connections
- Sidebar with **drag & drop groups**, right-click menus and search, **live connection status**
- Auth: **password · key · keyboard-interactive 2FA · SSH agent** (pick the key from the ring)
- **Serial / COM console** — baud, parity, flow control, DTR/RTS, send BREAK
- **Telnet** — option negotiation, terminal type, window size, BREAK/Interrupt/AYT, per-server Enter mode
- **Raw TCP** — untouched bytes, for console servers
- **ProxyJump / bastion** chains (recursive, `direct-tcpip`) and **ProxyCommand** (`%h %p %r`)
- **Compression** (`zlib@openssh.com`) and an opt-in **legacy algorithm** set for old gear
- **TOFU known-hosts** on every hop: verification dialog, management and OpenSSH import
- **Reconnect on drop** — manual or auto (up to 5 attempts, backoff)
- **Tags, favourites and environment labels (v1.2.7)** — search understands `tag:web`,
  `env:prod` and `fav`; production hosts carry a red badge before anyone runs anything on them
- **Multi-select (v1.2.7)** with Ctrl and Shift — connect, label, move or delete a whole group
- Import from **`~/.ssh/config`**, **PuTTY**, and (v1.2.7) **MobaXterm · XShell · SecureCRT**.
  Passwords are deliberately *not* imported: those formats store them decryptably by design,
  and copying them into a second store would only spread the problem

### Files (SFTP)
- Browse with **clickable breadcrumbs**, **inline rename**, drag & drop into the window
- **Parallel transfers** (pool of 4, up to 8 in settings), **transfer manager**: speed, ETA, pause, retry
- **Explorer-like columns** (name, type, mode, size, date), sort, resize, Ctrl/Shift selection
- **Drag-and-drop with Windows Explorer** — server to desktop/Downloads; PC to open SFTP folder
- chmod, hidden files, symlink follow, image preview; actions in the right-click menu
- Dual-pane (local ↔ remote); **detach SFTP** into its own OS window
- **Ctrl+wheel** (and Ctrl+/−/0) zooms text in SFTP and logs
- **Built-in editor** (CodeMirror 6) — atomic save back to the server
- **External editor** — OS default app, re-upload on save
- **SCP fallback (v1.2.7)** — servers with no `Subsystem sftp` (old switches, stripped images)
  are detected automatically and served over `scp` and `ls` instead

### Tunnels and ops
- Forwards: **local `-L`**, **remote `-R`**, **dynamic SOCKS5 `-D`** (tunnel create can be cancelled)
- **Resource monitor** — CPU / RAM / disk / load (`/proc` + `df`)
- **Docker workspace (v1.2.0)** — containers with live `docker stats`, ports and health; files inside a container; start/stop/restart/remove, shell
- **Docker Compose (v1.2.0)** — projects and services, up/down/start/stop/restart, per-service logs and shell, compose YAML
- **Docker logs** — coloured levels, follow (`-f`) with stop, wide panel; **detach** to a second monitor
- **Host logs (v1.2.0)** — `journalctl` with highlighting, filter and an error report exported to `.txt`
- **Session logging (v1.2.0)** — write terminal output to `%APPDATA%\serein\logs`, ANSI stripped
- **Server overview (v1.2.7)** — CPU, RAM, disk, load, network, uptime, OS and kernel, process
  count, failed services and Docker health on one screen
- **Run one command on several servers** (`Ctrl+Shift+M`) — results per host with exit code,
  stdout and stderr; hosts whose key is unknown are skipped with a reason, never trusted silently
- **Local utilities (v1.2.7)** — TLS certificate check, port test, DNS query, subnet calculator,
  hashes, JWT decode. No shell involved, nothing leaves the machine

### App windows
- Detached tabs, SFTP, logs, and workspace panels — **separate OS windows**, no Windows caption
- **Magnet**: windows snap flush and to guides (edges / center)
- The main window drags a docked group; drag an extra window to undock it
- Aux windows **minimize independently**; restore minimized aux via taskbar click or the main status bar
- Focusing any Serein window raises **all** of them; by default **one taskbar button**
  (settings can restore a button per window)
- Remembers detached window geometry after restart

### Security and storage
- Secrets via **DPAPI** (Windows) or the **keyring** (Linux) plus an optional **master password**
  (scrypt → AES-256-GCM); the UI never gets passwords or keys
- **Master password of 12+ characters** — no composition rules, following NIST SP 800-63B
  rather than the mandatory-symbol habit that produces `P@ssw0rd1`
- **Offline mode** — one switch stops the update check and every outbound request. There is
  exactly one outbound request in the whole codebase, and [SECURITY.md](SECURITY.md) names it
- **No auth retry storms** — a wrong password is not retried five times; that is how fail2ban
  trips and Active Directory accounts lock
- **Config schema is versioned**: the profile is copied before a migration, and a profile from
  a newer version is refused rather than silently rewritten
- Encrypted **`.tbk` backup** of servers, settings, and snippets
- **SSH keygen** (ed25519 / RSA) + `ssh-copy-id`
- Published with every release: SHA-256 sums and a **CycloneDX SBOM**; `cargo audit` and
  `npm audit` run in CI on every push

---

## Quick start

1. Install the setup exe or grab the portable `Serein_1.2.7_x64-portable.exe` from [Releases](../../releases/latest).
2. Import `~/.ssh/config` or add a host by hand.
3. Connect. The local terminal works with no SSH at all.

Target: install → first session in under two minutes.

---

## Requirements

| Need | Answer |
| --- | --- |
| OS | Windows 10 x64 **22H2+** or Windows 11 x64; Linux x64 (`.deb` / AppImage). No macOS yet |
| Web runtime | WebView2 on Windows (already there); `webkit2gtk-4.1` on Linux (pulled in by the `.deb`) |
| Privileges | admin is not required for daily use |
| Build from source | Node **18.18+** (tested on 24.16), Rust **stable** (`x86_64-pc-windows-msvc` / `x86_64-unknown-linux-gnu`), Tauri CLI **2.11.x**. Linux also needs `libudev-dev` — see [LINUX_MIGRATION.md](docs/LINUX_MIGRATION.md) |
| SSH agent | password, key file, **SSH agent**, or keyboard-interactive |

Matrix and smoke: [`docs/PHASE0.md`](docs/PHASE0.md).

---

## Install

From [Releases](../../releases/latest):

- **`Serein_1.2.7_x64-setup.exe`** — Windows installer (Start menu, uninstall).
- **`Serein_1.2.7_x64-portable.exe`** — Windows single file, no installer. Drop it and run. Settings still live in `%APPDATA%\serein`.
- **`Serein_1.2.7_amd64.deb`** — Debian/Ubuntu/Astra package (`/usr/bin/serein`).
- **`Serein_1.2.7_amd64.AppImage`** — portable Linux binary.

Every release publishes SHA-256 sums and a **CycloneDX SBOM** for both the Rust and the npm
dependency trees. Check the sums — the Windows build is **unsigned** and SmartScreen will
complain (*More info → Run anyway*).

Release notes: [RELEASE_NOTES_v1.2.7.md](docs/RELEASE_NOTES_v1.2.7.md).
Security policy and threat model: [SECURITY.md](SECURITY.md).

Auto-update is live (`nehade.xyz/updates/terminal/`), signed with minisign; the signing key
never enters CI. It can be switched off entirely — see offline mode below.

---

## Stack

| Layer | Tech |
| --- | --- |
| Shell | **[Tauri 2](https://tauri.app)** (Rust, system WebView2) |
| Frontend | **React 18** · **TypeScript 5** · **Vite** |
| Terminal | [`@xterm/xterm`](https://xtermjs.org) |
| Editor | [CodeMirror 6](https://codemirror.net) |
| SSH / SFTP | [`russh`](https://github.com/Eugeny/russh) **0.63** · [`russh-sftp`](https://github.com/AspectUnk/russh-sftp) |
| Local PTY | [`portable-pty`](https://crates.io/crates/portable-pty) |
| Crypto | `ring` backend · `aes-gcm` · `scrypt` · `ssh-key` (via `russh::keys`) · DPAPI / keyring |
| Tests | `vitest` (frontend) · real SSH servers in Docker (integration) |

---

## Architecture

```
┌─────────────────────────── WebView (React) ───────────────────────────┐
│  App · TabBar · Sidebar · SftpPanel · Monitor · Docker · CodeEditor    │
│  └── src/api  ──  window.api bridge  (invoke / listen)                 │
└───────────────────────────────┬───────────────────────────────────────┘
                     Tauri commands and events
┌───────────────────────────────┴───────────────────────────────────────┐
│  Rust backend (src-tauri/src)                                          │
│  ssh · ssh_agent · ssh_algos · proxycmd · serial · telnet ·            │
│  sftp · scp · remote_fs · tunnels · monitor · workspace · docker ·     │
│  pty · term_out · store · schema · vault · crypto · dpapi ·            │
│  os_secrets · keygen · importers · knownhosts · remoteedit ·           │
│  ownership · multihost · tools · error · sync                          │
└────────────────────────────────────────────────────────────────────────┘
```

- React talks to Rust through a thin `window.api` bridge (`invoke` / `listen`).
- One SSH connection multiplexes **shell + SFTP + exec + tunnels**. The handle is locked
  only briefly, so opening channels does not stall the others.
- Secrets decrypt **only in Rust**, at connect time. On Linux they live in the keyring and
  the file holds a `kr:{uuid}` handle — base64 is not encryption, so nothing is written in
  place of real storage.
- **Session ownership is one fact in Rust** (`ownership.rs`), not bookkeeping in each window.
  A `Set` in a frontend module is per-webview state and invisible to a second window; that
  mismatch is what killed detached-tab sessions before 1.2.6.
- The file manager picks its backend once per session: SFTP where the subsystem exists,
  `scp` + `ls` where it does not (`remote_fs.rs`).

---

## Build from source

You need [Rust](https://rustup.rs) (stable) and [Node.js](https://nodejs.org) 18+.
In non-interactive PowerShell, `cargo` is often off PATH — prepend `$env:USERPROFILE\.cargo\bin`.

The dev server binds **`127.0.0.1:1420`**, not `localhost` (IPv4 vs IPv6 otherwise sits on
"Waiting for frontend" forever).

```bash
npm install
npm run tauri dev
npm run tauri build
```

Installer output: `src-tauri/target/release/bundle/nsis/`.

```bash
npm run smoke
```

(`tsc --noEmit` + `cargo check`. Full protocol: `docs/PHASE0.md`.)

### Tests

```bash
npm test                                          # frontend, ~100 tests
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

Integration tests run against **real SSH servers in Docker** — Debian, Alpine/BusyBox, and one
with the SFTP subsystem removed on purpose:

```bash
./scripts/ssh-stand/up.sh
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --test-threads=1
./scripts/ssh-stand/down.sh
```

`--test-threads=1` is not optional: every test shares one `known_hosts.json`, which the app
rewrites whole. The stand runs in CI too, and it has already paid for itself — it found a
`known_hosts` race, a broken non-empty folder delete, and four bugs that made the SCP path
hang forever rather than fall back.

---

## Data and diagnostics

Profiles, secrets, known_hosts, vault live in `%APPDATA%\serein\` on Windows and
`~/.config/serein/` on Linux (`servers.json`, `secrets.json`, `vault.json`,
`known_hosts.json`, …). **Settings shows the exact path in use** — worth checking when the
server list looks empty, because a launcher with a different `HOME` opens a different profile.

Secrets never go to the UI. On Windows the payload is encrypted with DPAPI and stored in the
file; on Linux it lives in the **keyring** and the file only holds a `kr:{uuid}` handle — so
Windows secrets do not travel to a Linux machine, and a master password is required where no
Secret Service is running.

Per-session terminal logs can be switched on and land in the `logs` subfolder with ANSI
stripped; there is no application-wide log file yet — for that, use `npm run tauri dev` output.

---

## Known limitations

- The Windows build is **unsigned** — SmartScreen will fight you. Verify the SHA-256 sums.
- No **macOS** build. It needs a Mac to even test the launch, so it waits for hardware.
- No **X11 forwarding**.
- No external penetration test or independent review of the crypto layer yet.
- `rsa` carries RUSTSEC-2023-0071 with no upstream fix; it stays for plain `id_rsa` keys.
- Telnet has not been run against real network hardware, only an emulator.
- Not planned: cloud sync, mobile, plugins, a generic LLM chat pane.

Product plan: security first (that is what 1.2.7 was), then parity — **RDP and VNC**,
databases over the existing tunnels, and Windows Server support. RDP/VNC were previously
out of scope; that changed when the target changed to teams rather than a single user.
See [release notes 1.2.7](docs/RELEASE_NOTES_v1.2.7.md).

---

## License

[Apache License 2.0](LICENSE) © 2026 HaDe