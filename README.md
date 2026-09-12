<div align="center">

<img src="src-tauri/icons/128x128.png" width="96" alt="Serein icon" />

# Serein

**Desktop client for servers and network gear - everything in one window.**

SSH, SFTP and SCP with an editor, serial consoles, telnet and raw TCP.
Tabs and split panes, port forwards, resource monitoring, a Docker panel
and a local terminal - in an installer of about **7.8 MB**.

Free, open source, Apache 2.0. Windows x64 and Linux x64, **v1.3.1**.

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

## What it is

A **server workspace** in one window: terminal, files, Docker, logs, resources, tunnels,
databases and two remote desktops. The UI runs in the system WebView; SSH, SFTP, crypto and
the PTY live in a single Rust binary.

| | **Serein (Tauri)** | Typical Electron client |
| --- | :---: | :---: |
| Installer size | **≈ 7.4 MB** | ≈ 85 MB |
| Idle RAM | **≈ 100 MB** | 150–250 MB |
| SSH engine | pure Rust [`russh`](https://github.com/Eugeny/russh) | libssh2 / native |
| Runtime | system WebView2 | full Chromium |

Measured on 1.3.1 with no session open: about 100 MB private across the tree of seven
processes, and 70 MB on a fresh profile. Adding up Working Set in Task Manager gives about
480 MB - the same memory counted once per WebView2 process that shares it. The Electron
figures are public measurements normalised to the same metric; size is the 1.3.1 installer.

---

## Features

### Server Workspace (v1.1.0)
- **Server workspace rail** on SSH tabs: Terminal, Docker, Logs, Processes, Services, Tunnels
- **Processes** - `ps` table plus CPU/RAM/disk metrics; **Docker** - compact rows, properties via right-click
- **SFTP** - side panel from TabBar; server list collapses on connect
- **Detach** a tab or workspace panel; **reattach** to main (SSH session stays alive)

### Terminal and UX
- **Multiple SSH tabs** and **split panes** (tree with drag-resize, pick a server per pane)
- **Local terminal** with a sensible shell (PowerShell → cmd, or a custom / WSL shell)
- Search (`Ctrl+F`), zoom, **17 UI themes**, **compact mode**
- **Custom window chrome** (no Windows caption): minimize / maximize / close
- **Broadcast input** (current tab only), tab restore, tab drag
- **Ctrl+Shift+C / Ctrl+Shift+V** - copy/paste in the terminal (not WebView2 DevTools)
- SSH output batching; 4 MiB buffer with warning when overloaded
- Keyboard pane navigation, remappable shortcuts, **command palette** (`Ctrl+Shift+P`)

### Connections
- Sidebar with **drag & drop groups**, right-click menus and search, **live connection status**
- Auth: **password · key · keyboard-interactive 2FA · SSH agent** (pick the key from the ring)
- **Serial / COM console** - baud, parity, flow control, DTR/RTS, send BREAK
- **Telnet** - option negotiation, terminal type, window size, BREAK/Interrupt/AYT, per-server Enter mode
- **Raw TCP** - untouched bytes, for console servers
- **ProxyJump / bastion** chains (recursive, `direct-tcpip`) and **ProxyCommand** (`%h %p %r`)
- **Compression** (`zlib@openssh.com`) and an opt-in **legacy algorithm** set for old gear
- **TOFU known-hosts** on every hop: verification dialog, management and OpenSSH import
- **Reconnect on drop** - manual or auto (up to 5 attempts, backoff)
- **Tags, favourites and environment labels (v1.2.7)** - search understands `tag:web`,
  `env:prod` and `fav`; production hosts carry a red badge before anyone runs anything on them
- **Multi-select (v1.2.7)** with Ctrl and Shift - connect, label, move or delete a whole group
- Import from **`~/.ssh/config`**, **PuTTY**, and (v1.2.7) **MobaXterm · XShell · SecureCRT**.
  Passwords are not imported

### Databases (v1.3)
- **PostgreSQL, MySQL/MariaDB and Redis** over a channel inside the SSH session you already
  have - no port forward to set up
- Result grid, query history, and a confirmation for destructive statements: `DELETE`
  without `WHERE` asks twice, and a `#` in MySQL counts as a comment too
- The connection survives switching tabs and detaching the panel into its own window
- The MySQL client is our own, built on `mysql_common`

### Remote desktop (v1.3, RDP in v1.3.1)
- **VNC and RDP over a channel inside the SSH session**, with no port exposed to the network
- Frames travel as raw bytes rather than JSON, so a 1920x1080 screen costs eight megabytes
  instead of ten times that
- **RDP logs in once (v1.3.1)** - credentials go to the server with the autologon flag, and
  its own login window never appears. Settings: resolution, colour depth, traffic saving
- **VPN profile:** the saved default advertises a limited-bandwidth connection, uses 16-bit
  colour and disables desktop effects. The LAN profile keeps full quality for a fast network
- **Bounded presentation:** RDP updates are combined against the current framebuffer and shown
  at up to 30 FPS over VPN or 60 FPS on LAN, instead of redrawing for every dirty rectangle
- **Complete input:** vertical and horizontal wheel scrolling, coalesced pointer movement,
  optional shortcut capture while the canvas is focused, and a Ctrl+Alt+Del toolbar command
- **Resolution follows the window (v1.3.1)**: stretch it and the server redraws the desktop
  at the new size, with no reconnection
- **The desktop session belongs to the SSH connection (v1.3.1)**, not to a window: switching
  tabs keeps it, and detaching moves the live picture into the new window
- **The gear installs the server side when it is missing:** `x11vnc` or `tigervnc-server`
  for VNC, `xrdp` with `xorgxrdp` for RDP. The sudo password goes to standard input, never
  into the command line

### What kind of server is this (v1.3)
- One short probe per session, remembered afterwards. It tells **Linux, BusyBox, Windows**
  and "unclear" apart, and the label shows in the overview header
- On **Windows Server**, processes, services, the event log, disks and network go through
  PowerShell instead of `ps` and `systemctl`, which are not there
- On **BusyBox** the process list works too; per-process CPU is a dash there, since BusyBox
  does not report it
- The failed-services tile is not drawn where systemd does not exist

### Files (SFTP)
- Browse with **clickable breadcrumbs**, **inline rename**, drag & drop into the window
- **Parallel transfers** (pool of 4, up to 8 in settings), **transfer manager**: speed, ETA, pause, retry
- **Explorer-like columns** (name, type, mode, size, date), sort, resize, Ctrl/Shift selection
- **Drag-and-drop with Windows Explorer** - server to desktop/Downloads; PC to open SFTP folder
- chmod, hidden files, symlink follow, image preview; actions in the right-click menu
- Dual-pane (local ↔ remote); **detach SFTP** into its own OS window
- **Ctrl+wheel** (and Ctrl+/−/0) zooms text in SFTP and logs
- **Built-in editor** (CodeMirror 6) - atomic save back to the server
- **External editor** - OS default app, re-upload on save
- **SCP fallback (v1.2.7)** - servers with no `Subsystem sftp` (old switches, stripped images)
  are detected automatically and served over `scp` and `ls` instead

### Tunnels and ops
- Forwards: **local `-L`**, **remote `-R`**, **dynamic SOCKS5 `-D`** (tunnel create can be cancelled)
- **Resource monitor** - CPU / RAM / disk / load (`/proc` + `df`)
- **Docker workspace (v1.2.0)** - containers with live `docker stats`, ports and health; files inside a container; start/stop/restart/remove, shell
- **Docker Compose (v1.2.0)** - projects and services, up/down/start/stop/restart, per-service logs and shell, compose YAML
- **Docker logs** - coloured levels, follow (`-f`) with stop, wide panel; **detach** to a second monitor
- **Host logs (v1.2.0)** - `journalctl` with highlighting, filter and an error report exported to `.txt`
- **Session logging (v1.2.0)** - write terminal output to `%APPDATA%\serein\logs`, ANSI stripped
- **Hardware in the overview (v1.3)** - CPU, GPU with its driver, memory size and speed, and
  every disk with how full it is; the main one in gigabytes rather than percent. The GPU is
  found through `/sys`, with no `lspci` on the server. Load average is spelled out in words
- **Server overview (v1.2.7)** - CPU, RAM, disk, load, network, uptime, OS and kernel, process
  count, failed services and Docker health on one screen
- **Run one command on several servers** (`Ctrl+Shift+M`) - results per host with exit code,
  stdout and stderr; hosts whose key is unknown are skipped with a reason, never trusted silently
- **Utilities (v1.3)** moved out of a modal covering the app into a tab of their own with a
  rail: port check, port range, traceroute, HTTP request, DNS, TLS certificate, LDAP query,
  file comparison. The subnet calculator, hashes and JWT decode need no server at all
- **Checks run from the server as well as from your machine.** Traceroute from the server
  uses `tracepath`, which needs no root
- Files for comparison are picked over SFTP

### App windows
- Detached tabs, SFTP, logs, and workspace panels - **separate OS windows**, no Windows caption
- **Magnet**: windows snap flush and to guides (edges / center)
- The main window drags a docked group; drag an extra window to undock it
- Aux windows **minimize independently**; restore minimized aux via taskbar click or the main status bar
- Focusing any Serein window raises **all** of them; by default **one taskbar button**
  (settings can restore a button per window)
- Remembers detached window geometry after restart

### Security and storage
- Secrets via **DPAPI** (Windows) or the **keyring** (Linux) plus an optional **master password**
  (scrypt → AES-256-GCM); the UI never gets passwords or keys
- **Master password of 12+ characters**, no composition rules (NIST SP 800-63B)
- **Offline mode** - one switch stops the update check, the only outbound request in the
  codebase; [SECURITY.md](SECURITY.md) names it
- **A wrong password is not retried** - no fail2ban bans, no locked domain accounts
- **Config schema is versioned**: the profile is copied before a migration, and a profile from
  a newer version is refused
- Encrypted **`.tbk` backup** of servers, settings, and snippets
- **SSH keygen** (ed25519 / RSA) + `ssh-copy-id`
- Published with every release: SHA-256 sums and a **CycloneDX SBOM**; `cargo audit` and
  `npm audit` run in CI on every push

---

## Quick start

1. Install the setup exe from [Releases](../../releases/latest) (or the portable build - see [Install](#install)).
2. Import `~/.ssh/config` or add a host by hand.
3. Connect. The local terminal works with no SSH at all.

Target: install → first session in under two minutes.

---

## Requirements

| Need | Answer |
| --- | --- |
| OS | Windows 10 x64 **22H2+** or Windows 11 x64; Linux x64 (`.deb`, `.rpm`, AppImage). No macOS yet |
| Web runtime | WebView2 on Windows (already there); `webkit2gtk-4.1` on Linux (pulled in by the `.deb`) |
| Privileges | admin is not required for daily use |
| Build from source | Node **20.19+** or **22.12+** (tested on 24.16; Vite 8 does not start on older ones), Rust **stable** (`x86_64-pc-windows-msvc` / `x86_64-unknown-linux-gnu`), Tauri CLI **2.11.x**. Linux also needs `libudev-dev` - see [LINUX_MIGRATION.md](docs/LINUX_MIGRATION.md) |
| SSH agent | password, key file, **SSH agent**, or keyboard-interactive |

Matrix and smoke: [`docs/PHASE0.md`](docs/PHASE0.md).

---

## Install

From [Releases](../../releases/latest):

- **`Serein_1.3.1_x64-setup.exe`** - Windows installer (Start menu, uninstall).
- **`Serein_1.3.1_x64-portable.exe`** - Windows single file, no installer. Drop it and run. Settings still live in `%APPDATA%\serein`. In 1.3.1 this file has **no RDP**: remote desktop over RDP needs the helper `serein-rdp.exe` next to it, and the single-file build does not carry it. From the next release the portable build is a ZIP with both files.
- **`Serein_1.3.1_amd64.deb`** - Debian/Ubuntu/**Astra** package (`/usr/bin/serein`).
- **`Serein-1.3.1-1.x86_64.rpm`** - **Fedora** package. Installs and runs on a clean Fedora; on RedOS and Alt it has not been checked yet.
- **`Serein_1.3.1_amd64.AppImage`** - portable Linux binary, one file for both families.

Every release publishes SHA-256 sums and a **CycloneDX SBOM** for both the Rust and the npm
dependency trees. Check the sums - the Windows build is **unsigned** and SmartScreen will
complain (*More info → Run anyway*).

Release notes: [RELEASE_NOTES_v1.3.1.md](docs/RELEASE_NOTES_v1.3.1.md).
Security policy and threat model: [SECURITY.md](SECURITY.md).

Auto-update checks GitHub Releases first and falls back to our own mirror
(`nehade.xyz/updates/terminal/`) when GitHub does not answer. Both manifests carry the same
minisign signatures; the signing key never enters CI. The check can be switched off entirely -
see offline mode below, and [SECURITY.md](SECURITY.md) for exactly what goes out.

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
  the file holds a `kr:{uuid}` handle - base64 is not encryption, so nothing is written in
  place of real storage.
- **Session ownership is one fact in Rust** (`ownership.rs`), not bookkeeping in each window.
  A `Set` in a frontend module is per-webview state and invisible to a second window; that
  mismatch is what killed detached-tab sessions before 1.2.6.
- The file manager picks its backend once per session: SFTP where the subsystem exists,
  `scp` + `ls` where it does not (`remote_fs.rs`).

---

## Build from source

You need [Rust](https://rustup.rs) (stable) and [Node.js](https://nodejs.org) 18+.
In non-interactive PowerShell, `cargo` is often off PATH - prepend `$env:USERPROFILE\.cargo\bin`.

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

Integration tests run against **real SSH servers in Docker** - Debian, Alpine/BusyBox, and one
with the SFTP subsystem removed on purpose:

```bash
./scripts/ssh-stand/up.sh
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --test-threads=1
./scripts/ssh-stand/down.sh
```

`--test-threads=1` is not optional: every test shares one `known_hosts.json`, which the app
rewrites whole. The stand runs in CI too, and it has already paid for itself - it found a
`known_hosts` race, a broken non-empty folder delete, and four bugs that made the SCP path
hang forever rather than fall back.

---

## Data and diagnostics

Profiles, secrets, known_hosts, vault live in `%APPDATA%\serein\` on Windows and
`~/.config/serein/` on Linux (`servers.json`, `secrets.json`, `vault.json`,
`known_hosts.json`, …). **Settings shows the exact path in use** - worth checking when the
server list looks empty, because a launcher with a different `HOME` opens a different profile.

Secrets never go to the UI. On Windows the payload is encrypted with DPAPI and stored in the
file; on Linux it lives in the **keyring** and the file only holds a `kr:{uuid}` handle - so
Windows secrets do not travel to a Linux machine, and a master password is required where no
Secret Service is running.

Per-session terminal logs can be switched on and land in the `logs` subfolder with ANSI
stripped; there is no application-wide log file yet - for that, use `npm run tauri dev` output.

---

## Known limitations

- The Windows build is **unsigned** - SmartScreen will fight you. Verify the SHA-256 sums.
- No **macOS** build. It needs a Mac to even test the launch, so it waits for hardware.
- No **X11 forwarding**.
- No external penetration test or independent review of the crypto layer yet.
- `rsa` carries RUSTSEC-2023-0071 with no upstream fix; it stays for plain `id_rsa` keys.
- Telnet has not been run against real network hardware, only an emulator.
- The `.rpm` is built in a Fedora container and checked on Fedora only. RedOS and Alt are close
  relatives, not the same system: older libraries there can stop a Fedora-built binary.
- The 1.3.1 portable single-file `.exe` has no RDP - the helper is a separate program and the
  single file does not carry it. Use the installer, or the portable ZIP from the next release.
- RDP over a slow VPN is usable but heavier than `mstsc`: plain bitmaps, no RemoteFX/H.264 yet.
- Not planned: cloud sync, mobile, plugins, a generic LLM chat pane.

Product plan. 1.2.7 was about security; 1.3 closed three parity items at once: **VNC**,
**databases** over our own channels, and **Windows Server**; 1.3.1 added **RDP**. Still open:
the other three databases (SQLite, MongoDB, SQL Server), macOS, and the Russian software
registry. See [release notes 1.3.1](docs/RELEASE_NOTES_v1.3.1.md).

---

## License

[Apache License 2.0](LICENSE) © 2026 HaDe
