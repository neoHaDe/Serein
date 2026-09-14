## Serein v1.4.0

Tasks that run in order across a set of servers, all seven databases from the roadmap, folder
comparison with upload of what changed, named workspace profiles — and a full security and
reliability audit of the application worked through item by item.

### Tasks

A task is an ordered list of steps run on the servers you pick. Servers run in parallel, as in
Fleet; on each server the steps run strictly one after another.

- **Seven kinds of step:** a shell command; upload of a file or folder; download of a file or
  folder (from several servers, into a subfolder per server); folder sync that uploads only new
  and changed files; a service action — start, stop, restart — under systemd, OpenRC and Windows;
  a Docker container action — start, stop, restart, remove; and a health check: a command must
  exit with 0, a port must accept connections, or an address must answer over HTTP, checked from
  the server itself with a number of attempts and a pause between them.
- **Every step has a condition, a time limit and retries.** A step runs after successful ones,
  only after the task stopped on an error — the place for a rollback or clean-up — or always.
  The limit defaults to 300 seconds, retries go up to ten. Without "continue on error" a failed
  step stops the task on that server.
- **Dry run changes nothing.** It connects to every server and reports what it can check without
  consequences: the service and the container exist, how many files an upload or download would
  move, what a sync would send. Commands are not run at all, not even harmless-looking ones.
- **Progress in real time**, per server and per step, with the output of every step; "Stop"
  starts no new connections. The history keeps the last fifty runs with a report for each.
- Hosts with an unconfirmed key are skipped with a reason, as in Fleet.
- Tasks and workspace profiles are in the sidebar footer and in the command palette.

### Fleet

Running one command on several servers became something you can steer on a large fleet.

- **Concurrency** is chosen — 1, 4, 8, 16, 32 or your own number up to 64 — and remembered.
- **A time limit per host** covers the connection and the command together, 120 seconds by
  default.
- **"Stop" no longer keeps connecting.** Hosts already running are allowed to finish; the rest
  are marked "run stopped" at once instead of being connected to one by one.
- **Retry failed** re-runs the hosts that could not be reached or returned a non-zero code.
  Hosts skipped for an unconfirmed key are not retried — they would be skipped again.
- **A summary by exit code** under the results and **a report saved to a file**, failures first.
- The output of one host is capped at 256 KiB, keeping the end, where the error usually is.

### Folder comparison

"Compare with server" in the dual-pane file view compares the local folder with the remote one
and lists every file: new, changed, newer on the server, only on the server, identical.

- **The comparison is the dry run** — nothing is written on either side.
- **"Upload N"** sends only new and changed files through the normal transfer queue, with its
  progress and cancel. Files edited on the server later are left alone unless a separate box is
  ticked. Missing folders are created. Nothing is ever deleted on the server.
- On servers without SFTP the modification time is not available and the comparison goes by size
  only; the window says so.

### Workspace profiles

The current tabs — panes, servers and the open tool — are saved under a name such as
"Production" or "Logs" and opened later with one action, next to the current tabs or instead of
them. A profile can be overwritten with the current tabs or deleted. Panes that point at a server
deleted since are dropped, and the list says how many.

### Databases: all seven

- **SQL Server**, through a channel inside the SSH session like the others. Encryption is on,
  because new installations refuse connections without it; several result sets, dates as dates,
  and the server's error number in the message.
- **SQLite**, a file on the server: the query runs in `sqlite3` there, SQL goes to its standard
  input, and the answer is cut on the server at the size limit. A path to a missing file is an
  error, not a new empty database.
- **MongoDB** with its own protocol implementation — the official driver opens its own sockets and
  cannot use a channel. Login is SCRAM-SHA-256 or SCRAM-SHA-1 with the server signature verified.
  Queries are written the way mongosh reads them: `db.collection.find().sort().limit()`, `findOne`,
  `aggregate`, counts, `distinct`, inserts, updates, deletes, indexes, `show dbs`, `use`,
  `runCommand`. JavaScript is not executed. A filter is mandatory for updates and deletes, and
  dropping a collection asks first.
- **A "Stop" button** for a running query. PostgreSQL cancels the query and keeps the connection;
  the other databases close it and say so.
- **The database stops a long query itself** at 28 seconds — PostgreSQL, MariaDB, MySQL for
  `SELECT`, MongoDB for reads — so abandoned work no longer keeps loading the server.
- **Limits for every result:** 5,000 rows, 16 MiB in total, 64 KiB per cell, and the truncation
  said in words. Several result sets are kept apart, with a switch between them.
- **The result table is paged**, 200 rows per page.

### Server overview

- **Charts for the last hour** — CPU, memory, disk, load per core, network in and out. One
  collector per SSH session keeps the history through tab switches and detaching.
- **Health in words** — "Normal / Attention / Bad" with the reasons and a link to the right tool.
  CPU becomes a reason only when it stays above the threshold for five minutes.
- **Thresholds** are set globally and overridden per server.

### Remote desktop

- **A VPN profile** asks the server to save bandwidth, uses 16-bit colour and turns off desktop
  effects; the LAN profile keeps full quality. Updates are combined and shown at up to 30 FPS over
  VPN and 60 FPS on LAN.
- **The desktop uses its own SSH connection** with a small window, and Nagle's algorithm is off on
  every SSH connection — keystrokes and pointer moves no longer wait behind picture data.
- **Input:** vertical and horizontal wheel, coalesced pointer movement, modifiers released when
  focus is lost, optional shortcut capture, and a Ctrl+Alt+Del command. **Full screen** for RDP and
  VNC, left with `Esc`.
- **Closing a session is complete:** one stop signal for every part, and the helper process is
  removed if it does not leave on its own.
- The local RDP bridge requires a one-time pass, and the screen size announced by the server is
  checked before memory is allocated.
- **The portable build is a ZIP with both programs.** A single `.exe` had no RDP.

### Security and reliability

- **File names from a server can no longer write outside the download folder** — separators,
  `..`, drive letters, reserved device names are refused with a reason shown in the transfer list.
- **"Open in editor" no longer launches the file**; a configurable editor program opens it.
- **Saving over SFTP no longer loses the original or its permissions**, and downloads go to a
  `.part` file renamed at the end. The same size no longer counts as an already downloaded file.
- **The external editor no longer loses an edit** after a network error, and does not overwrite a
  file changed on the server meanwhile.
- **The profile and secrets are written whole or not at all**, and a damaged file no longer turns
  into an empty profile.
- **One profile, one process.** A second copy of the application with the same profile explains
  why and exits instead of silently overwriting the first one's changes.
- **Host key trust is explicit:** background connections accept only confirmed keys, and an
  unreadable fingerprint store refuses the connection instead of treating every host as new.
- **Connections have time limits** for login, the channel to the next jump host and the handshake
  over it — while the time spent answering a host key question or a second factor does not count.
  Opening any channel is limited to 15 seconds.
- **Backup import shows what is inside first**, and every ProxyCommand stays disabled until it is
  confirmed separately. The backup now includes workspace profiles and tasks; older backups still
  open.
- **Folder traversal over SCP** has the same limits as SFTP and does not follow links to folders;
  traversal stops as soon as the session closes.
- **Switching RDP on for Windows no longer opens the firewall** unless a separate box is ticked,
  and the result is verified by the listening port.
- **Clipboard writes from a server** are accepted only from the active terminal and within limits.
- **A damaged settings file no longer turns off offline mode.**
- **The supply chain is checked whole:** `cargo audit` covers the RDP helper's own dependency tree,
  and the check also runs daily on a schedule.

### Checksums

```
aacf5b1aecbb4336c88be9271812ee914c74ae8ae9568e1c5d862a317f34335b Serein_1.4.0_x64-setup.exe
ad3375d1a9230681dcc07cb50bf35dd05366c8d474dd65aae053de9b77808812 Serein_1.4.0_x64-portable.zip
9531ea12cad0b615e387266ce6d1c948ac4d22ba8b215b892f5c0012eaef7207 Serein_1.4.0_amd64.deb
24bbba72d40786f760e150196d81a961efc27a2561d1ca0d45f78f99d175a41e Serein_1.4.0_amd64.AppImage
dad2b283c2c120a5a8431002645c495a07ad49d6e4d6dd5d6395739ffc9b0eb0 Serein-1.4.0-1.x86_64.rpm
```
