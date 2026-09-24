## Serein v1.5.1

A security and correctness release. The administrator policy from 1.5.0 now covers every way the
application opens a connection, not only SSH; the action log records what actually happened
instead of what was requested; and the file panel no longer mixes up servers. Nothing new to
learn — the changes are in what the application refuses and what it writes down.

If your organisation uses `allowedHosts`, update: in 1.5.0 the list could be bypassed with
ordinary profile settings.

### Administrator policy covers every way out

- **`allowedHosts` is checked on every path that opens a connection:** SSH and every jump host,
  telnet and raw TCP, the target of a local port forward, each destination requested through a
  SOCKS5 forward (the client picks it per request, so it is checked per request), databases,
  RDP and VNC, and the network utilities — from this machine and from the server. In 1.5.0 only
  SSH and telnet/TCP were checked, so a forbidden address could be reached through a tunnel, a
  database, a desktop or a utility via an allowed server. A refusal is written to the action log
  as `policy.deny`. A remote forward (`-R`) is not checked: its target is the server's own
  loopback.
- **Tunnels that start with the session are checked too.** Every tunnel in a server profile
  opens on connect; the check used to sit only behind the "open" button, so those went through
  unchecked. The check is now at the single point where a tunnel is opened.
- **A tunnel of unknown type is refused.** Anything that was not `remote` or `dynamic` used to
  open as a local forward, while the policy check only looked at `local`; a type such as `"L"`
  was a way around it.
- **A server marked as a COM port can be neither an SSH target nor a jump host.** Such an entry
  skipped the address check, yet a jump through it still went over SSH to its address.
- **Servers with a ProxyCommand are refused** while `allowedHosts` or `forbidLocalTerminal` is
  set: the command runs on this machine and connects wherever it likes, so the address in the
  profile means nothing, and the command itself is the local shell the policy forbids.
- **The policy file path comes from Windows** (`FOLDERID_ProgramData`), not from the
  `ProgramData` environment variable, which a user can change. If the path cannot be determined,
  the file is not applied and the error is shown in Settings.
- **The stand-only switch `SEREIN_TRUST_NEW_HOSTS` is gone from release builds.** It turned off
  host-key checks for background connections and now exists only in debug builds.
- SECURITY.md and the Group Policy template texts (en-US, ru-RU) describe the exact coverage.

### The action log records what actually happened

- **Uploads of several files** were recorded as successful even when some failed or were
  cancelled — SFTP reported success whatever happened to individual files. The record now
  carries the result: "failed 1 of 3: …" or "cancelled 1 of 1".
- **Dragging files to Explorer** is a download like any other and is now recorded; it was not
  recorded at all.
- **Changes made while setting up a remote desktop** are recorded: installing VNC or xrdp
  packages with sudo, enabling RDP (including opening the firewall on Windows), changing the VNC
  password. The password itself is never written — only the fact.
- **Tunnels opened on connect** are recorded, like the ones opened by hand.
- **Refusals are recorded too:** a Docker action with an invalid name, killing PID 1, a service
  action with an invalid name.
- **A log that stops writing is visible.** Write failures were reported only to a console the
  application does not have, so a log that silently stopped looked like a quiet one. Failures are
  counted, the last reason is shown in the log window and next to the switch in Settings, and
  sent to syslog if configured. Poisoned locks — the trace of a panic that interrupted a state
  change — are counted and shown as well.

### Files

- **The file panel no longer mixes servers.** It listened to transfers and edits of all sessions:
  with two servers open, each panel showed the other's transfers, and "Retry" sent the other
  server's file through its own session — to the wrong server. The panel now shows only its own
  session, and a retry goes through the session where the transfer started.
- **Dragging to Explorer no longer produces empty folders.** A file that failed to download
  arrived in Explorer as an empty folder with its name. Only what was actually downloaded is
  handed over now.
- **File permission errors are no longer swallowed.** Profile, secrets, session log, the
  editor's temporary copy, a generated key: if access cannot be restricted, the operation fails
  with the reason instead of leaving the file readable by everyone.

### Docker

- **Invalid names are refused, not trimmed.** Characters were silently cut out of container,
  project and service names: `web;x` became `webx`, which is a different container; an empty
  name turned a single container's stats into stats of all containers; a leading `-` was read by
  docker as an option. Such a name is now refused with the reason.
- **A compose file in a folder with a space in its name** is found again — the path is quoted
  whole instead of being trimmed.
- **`up` without a known compose file is refused.** Compose used to look for a file in the
  server's home directory and could start an unrelated project from there. Other actions work by
  project name, as before, including on Windows servers.
- A refusal is shown as an ordinary error instead of leaving the button stuck "busy".

### Processes and services

- "Kill" on PID 1 or an action on a service with an invalid name showed nothing and left the row
  stuck "busy". The reason is now shown.

### Smaller fixes

- The terminal size follows one rule for every kind of session: a huge value is clamped instead
  of wrapping (65,536 columns used to become 0), and the local terminal and SSH got the same
  limits as telnet.
- A second import of the same task gets a numbered name instead of a duplicate.
- The external editor path hint in Settings lost its backslashes and read
  "C:Program FilesNotepad++notepad++.exe".

### Under the hood

- The 3,127-line `lib.rs` is split: 172 Tauri commands live in `commands/` by area, parsing and
  decisions in the area modules with tests. The file transfer layer uses one transfer structure
  instead of 8–12 parameters per function; the file panel is split into parts (1,641 → 1,100
  lines). No `too_many_arguments` exceptions are left in the code.
- CI gates: `cargo fmt --check`, `cargo clippy -D warnings`, eslint.
- `cryptoki` 0.12.1 in the RDP helper for RUSTSEC-2026-0286.
- Tests: 383 Rust (351 in 1.5.0), 232 frontend (219), 13 in the RDP helper, plus the SSH stand.

### Checksums

```
```
