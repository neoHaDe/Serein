## Serein v1.3.0

Three parity items in one release: the remote desktop, databases next to the server, and
servers that are not Linux. All three go through the SSH session that is already open, so
nothing new is exposed to the network.

### Remote desktop over VNC

An RFB client of our own, speaking to the server over a channel inside the SSH session rather
than a separate connection. VNC almost always listens on `127.0.0.1`, and it should: the
protocol guards itself with an eight-character DES password, which has no business facing a
network.

- Pixels travel as raw bytes over a `Channel`, not as JSON. A 1920x1080 frame in BGRA is eight
  megabytes; in a JSON array it inflates by an order of magnitude.
- **A chooser comes before the connection.** Two tiles: VNC, and the slot where RDP will go.
  RDP is greyed out and states the reason instead of promising "soon": its library pulls a
  crypto version incompatible with our SSH core.
- **The gear installs VNC on the server when it is missing.** `x11vnc` on Debian and Astra,
  `tigervnc-server` on Fedora, and they are two different programs with a similar job, not one
  package under two names. The sudo password goes to standard input, never into the command
  line, which is visible in the server's process list to everyone with an account on the box.
- Setting the desktop password uses `vncpasswd -f` where tigervnc is installed and
  `x11vnc -storepasswd` otherwise. The first reads the password from stdin, so it never
  appears in a command line at all.
- Listening ports are found through `ss`, then `netstat`, and failing both by reading
  `/proc/net/tcp` directly. Minimal images have neither tool, and without that last path a
  running desktop was reported as "not started".

### Databases next to the server

PostgreSQL, MySQL/MariaDB and Redis, again over a channel inside the SSH session. The database
listens on the server's loopback, which is where we arrive, so no port forward is involved.

- Result grid, query history, and a second confirmation for destructive statements. `DELETE`
  without `WHERE` asks twice, and `#` counts as a comment in MySQL as well as `--`.
- The connection survives switching tabs and detaching the panel into its own window.
- **The MySQL client is ours.** Neither `mysql_async` nor `sqlx` will accept a ready stream;
  both insist on opening the socket themselves. `mysql_common` provides packet parsing and the
  password scrambles, and the handshake and queries live in `src/mysql.rs`.
- Stored procedures return extra result sets. Leaving them unread desynchronised the
  connection, so the client asks for `CLIENT_MULTI_RESULTS` and drains what follows.

### Servers that are not Linux

One short probe per session, remembered afterwards. It tells Linux, BusyBox, Windows and
"unclear" apart, and the answer is shown in the overview header.

- **Windows Server.** Processes, services, the event log, disks and network go through
  PowerShell instead of `ps` and `systemctl`. Fields are joined by PowerShell itself with tabs:
  parsing by column width cannot work, because the width depends on the console size and the
  headers on the system language.
- **BusyBox, where three panels were quietly lying.** The process list was empty, because
  BusyBox `ps` understands neither `--sort` nor a `pcpu` column and answered with usage text on
  stderr; an empty table looked like a working one. The process count on the overview read zero
  for the same reason and is now counted from `/proc`. The failed-services tile reported that
  all was well on systems with no systemd at all, and is no longer drawn there.
- BusyBox reports no per-process CPU whatsoever. That column now shows **a dash, not a zero**:
  a zero would read as "this process is idle", which would be untrue.
- Logs are located by a non-empty answer rather than by the presence of a command, and
  `logread` is included — the BusyBox ring buffer that systemd knows nothing about.
- Load average does not exist as a concept on Windows, so the panel omits it there instead of
  drawing zeros.

### Utilities in a tab of their own

They used to open as a small modal covering the application. Now they are a tab laid out like
an open server: a rail on the left listing all eleven, the utility itself on the right, with
those that need no server (subnet calculator, hashes, JWT decode) below a divider.

- New: **port range scan, traceroute, HTTP request, LDAP query, file comparison.**
- **Checks can run from the server instead of from your machine.** "The site is down for me"
  and "the site is down from the server" are different faults with different answers.
- Traceroute from the server runs `tracepath` first: `traceroute` without root replies
  `Operation not permitted`. An empty hop list is always an error now, quoting the program.
- Files for comparison are chosen through SFTP rather than by typing a remote path from memory.
- LDAP shows at most fifty entries at a time, because the panel is a panel and not a dump.

### Server overview

- **Hardware:** CPU, GPU with its driver, memory size and speed. The GPU is identified by PCI
  device class in `/sys` rather than through `lspci`, which servers rarely have installed. RAM
  speed comes from `dmidecode`, and when it is absent or unprivileged the panel says so instead
  of leaving a blank.
- **Every mounted filesystem** with how full it is, the main one in gigabytes rather than
  percent.
- **Load average in words:** "right now N of M cores are busy". Three bare numbers meant
  nothing to anyone, including the author.

### Packaging and updates

- **An RPM for Fedora** and its relatives, built inside a Fedora container. A package built on
  Debian installs on Fedora and then refuses to start, because the binary wants the glibc it
  was linked against — and that looks like broken software rather than a package built in the
  wrong place.
- **GitHub is now the primary update source, our own host the fallback.** The updater takes a
  list of manifests and tries them in order, so a blocked or simply unreachable github.com
  stops updates for nobody. The fallback is at the manifest level, not per file: if GitHub
  serves the manifest but not the artifact, the download does not hop to the mirror.
- A stray 12 MB benchmark binary was being shipped inside the `.deb`. It was declared as a
  `[[bin]]` and has moved to `examples/`.

### Interface

- **Form controls on Linux were drawn white on a dark theme.** The page never declared a colour
  scheme, so WebKitGTK used its own light one for inputs and dropdowns. Visible on both Astra
  and Fedora.

### Install

1. **Serein_1.3.0_x64-setup.exe** — Windows NSIS installer (RU+EN).
2. **Serein_1.3.0_x64-portable.exe** — Windows single exe. Profile in `%APPDATA%\serein`.
3. **Serein_1.3.0_amd64.deb** — Debian, Ubuntu and Astra (`/usr/bin/serein`, config in
   `~/.config/serein`).
4. **Serein-1.3.0-1.x86_64.rpm** — Fedora and relatives.
5. **Serein_1.3.0_amd64.AppImage** — portable Linux binary, one file for both families.

In-place upgrade from any 1.x on Windows and from 1.2.5 on Linux. The AppImage is large
(~110 MB) because it carries the whole WebKitGTK stack; the `.deb` and the `.rpm` use the
system one.

**Everything else attached here is machinery, not a download.** `latest.json` is the update
manifest the application fetches to find out whether a newer version exists; its name is part
of the URL, so it cannot be renamed. `serein-1.3.0-sbom.zip` is the dependency inventory
(CycloneDX, 467 Rust crates and 44 npm packages) for whoever has to review what is inside.

SHA-256:

```
928ed997756ae6d5d5318671c4e45e91f7d3220eb59b2a4ffe731f13fdad8666  Serein_1.3.0_x64-setup.exe
3de6d33c7b618cef891c00c662f82a22dbed94db0dfea92012d369da0fa6fb1e  Serein_1.3.0_x64-portable.exe
be512899d2a207a306cd57062a844fb3b5f9b2fc75ac6766d60904bb4218c420  Serein_1.3.0_amd64.deb
c836d8faae1a692a6ab7c69bac5b4256e391fed1a80f5bf48bf07b0ef93ccbdc  Serein-1.3.0-1.x86_64.rpm
7a55fe8b1013ffa25bf07485830d67c994377e6a71683064c713a83b2d43b89b  Serein_1.3.0_amd64.AppImage
```

### Limitations

- The Windows build is **unsigned**. SmartScreen will complain; the SHA-256 above is the check
  that actually means something.
- **RDP is not here.** Its library needs a crypto version that conflicts with our SSH core.
- The Windows Server code paths are built against recorded command output: there is no Windows
  Server on the test stand and it cannot be run in Docker. The OpenRC branch is untested too —
  the Alpine image does not ship it.
- RedOS and Alt are not tested separately; the RPM is built for Fedora and expected to work on
  them, which is not the same as verified.
- Four databases are still missing: SQLite, MongoDB, SQL Server, and the rest of the list.
- `rsa` (RUSTSEC-2023-0071) has no upstream fix and stays in the audit ignore list.
- No external penetration test or independent review of the crypto layer has been done yet.
- Telnet has not been run against real network hardware.
- The Linux `.deb` and `.rpm` cannot update themselves; only the AppImage can. The application
  detects this and offers the release page instead of an install that would fail.
