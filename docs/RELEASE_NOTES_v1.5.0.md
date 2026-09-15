## Serein v1.5.0

The first corporate layer — an action log that a security team can verify and forward to its
SIEM, and administrator policies delivered through Group Policy — plus tasks with variables,
secrets and environments, and a reliability pass that found and closed several ways to lose or
damage a file.

### Tasks: variables, secrets, environments

- **Variables** — write `{{name}}` in any field of any step. Each variable has a default and two
  switches: "ask before running" and "secret". Built-ins differ per server: `{{server.name}}`,
  `{{server.host}}`, `{{server.user}}`. Docker templates such as `{{.Names}}` are left alone.
- **An unknown variable is an error before the run**, not an empty gap in a command. Fields are
  checked after substitution as strictly as without variables: a service name that becomes
  `nginx; rm -rf /` is refused on that server.
- **Values are asked for before the run** in the task window, secrets in a password field.
- **A secret is never stored** — not in the task, not in an environment, not in an export. It is
  typed in for every run, and in step output, errors and run history it is replaced with `••••`.
- **Environments** (prod, stage, …) carry their own variable values and, if set, their own
  servers. The environment is picked next to the Run button and recorded in the report and the
  action log.
- **Ready-made templates:** restart a service and check it (with the service journal on failure),
  deploy (sync a folder, restart, check), update a Docker Compose project.
- **Export and import.** A task goes to a file without secrets; servers are written by address and
  name, because ids differ between machines. Import finds servers by address, port and user, then
  by name, and names the ones it could not find.

### Action log

Who did what, when and on which server.

- **What is recorded:** connections and disconnections with the reason (closed or dropped); file
  operations — folder creation, deletion, rename, permissions, saves from the editor, uploads,
  downloads, the external editor and its uploads; containers and Compose; services; killed
  processes; database connections and queries; Fleet per host; tasks per server; tunnels; RDP and
  VNC; application start.
- **Terminal lines** are recorded on Enter as typed. Completion and history recall are marked as
  text added by the server. **The line after a password prompt is not recorded at all.**
- **Integrity:** monthly JSONL files in the settings folder; every record carries a sequence
  number and the hash of the previous one. "Verify" in the log window finds deleted and altered
  records.
- **Forwarding to syslog or a SIEM** on request: RFC 5424 over UDP or TCP, with counters of sent
  and failed records. There is no TLS, and the settings and SECURITY.md say so.
- On by default. It can be switched off in settings — the switch-off is the last record written.
- The "Action log" window (palette and settings) has multi-word search, verification and export
  of the whole log as one file.

### Administrator policies

An administrator fixes settings a user cannot change.

- **Sources:** `HKLM\SOFTWARE\Policies\Serein` (Group Policy) and a policy file —
  `%ProgramData%\Serein\policy.json` on Windows, `/etc/serein/policy.json` on Linux. The registry
  wins over the file.
- **A Group Policy template** — `docs/policy/Serein.admx` with en-US and ru-RU language files:
  the action log, syslog, offline mode, allowed servers and every restriction below, set with the
  mouse in the Group Policy editor.
- **Any setting** can be locked: it is applied over the user's choice, greyed out in settings,
  and never written to the user's file.
- **Allowed servers** — SSH, telnet and TCP only to listed addresses: a name, `*.domain`, a subnet
  or `*`. Every jump host is checked, and a refusal goes to the action log. The address from the
  profile is compared; names are not resolved, so a DNS alias of a forbidden host is not caught
  (said in SECURITY.md).
- **No saved passwords** — passwords and key passphrases are not stored, and those saved earlier
  are not used. Serein asks at connection time: the password in a prompt, including for servers
  that offer only the "password" method, and the passphrase of an encrypted key in a prompt too.
- **Mandatory master password**, **no local terminal** (no buttons anywhere, and the backend
  refuses), **no session recording**, **no legacy SSH algorithms** even if a server profile asks.
- **A policy that cannot be read or understood — including an unknown key, i.e. a typo — is not
  applied at all.** The error is shown in settings and written to the action log at start.
- **The policy file must be protected.** It is used only if the file and its folder belong to
  Administrators, SYSTEM or TrustedInstaller and nobody else can change them (on Linux: root, no
  group or world write). Otherwise a user could create `C:\ProgramData\Serein` first and put
  their own file there. The registry is the recommended source on Windows.
- A banner in settings lists every restriction in words.

### Files: nothing is lost or damaged

- **An SFTP upload no longer damages a file when it is cancelled or cut off.** It used to write
  straight into the target, so a cancelled replacement of a config left a truncated file behind.
  The upload now goes to a temporary file next to it and takes its place at the end. The old
  file's permissions are kept; a symlink stays a symlink and its target gets the new content; if
  the folder is not writable, the upload writes in place as before.
- **SCP streams files** instead of holding them in memory, shows progress and can be cancelled.
  An SCP upload also goes through a temporary file replaced by one `mv`, keeping the original
  whole and its permissions intact — which also fixed saving from the built-in editor over SCP,
  which used to reset permissions to 644.
- **The external editor no longer drops an edit after a conflict.** When the file changed on the
  server, the edit is neither uploaded nor forgotten: the copy stays on disk and its path is shown
  in the file panel. If the server version cannot be read, the upload is retried, not skipped.
- **Downloads no longer touch files with similar names.** Every transfer gets its own temporary
  name, created only as a new file: `report.part` next to `report` is not overwritten, and two
  transfers never share a file.
- **Links inside the download folder cannot lead a write outside it.** If the destination path
  contains a symlink or junction, nothing is written and the reason is in the transfer list.
- **Folder comparison no longer calls unverified files identical.** A file of the same size edited
  on the server later is "newer on the server". Without a modification time (SCP) the same size is
  "can't tell", uploaded only with a separate box ticked. An SFTP upload carries the local
  modification time to the server, so your own fresh upload does not look like a server edit.
- **Uploading after a comparison does not overwrite a change made since.** The window compares
  again before uploading and stops with the list of changed files; a task's sync step checks each
  file right before writing and skips the changed ones.
- **Tasks put downloads from servers with the same name into separate folders.**

### Sign-in

- **Saved passwords work with keyboard-interactive.** Servers with `PasswordAuthentication no`
  and a password through PAM refused tunnels, Fleet and tasks altogether, and a tab asked for the
  password on every login although it was saved. The saved password now answers such a prompt
  once. Second-factor codes and "new password" prompts still go to a person, and a password the
  server has already refused is not sent again.
- The passphrase of an encrypted key is asked for in a prompt when it is not saved, instead of a
  silent failure.

### Processes and containers

- **Sort by any column** in processes (PID, user, CPU, memory, state, command) and containers;
  CPU and memory sort descending first, and the choice is remembered. The server now returns up to
  2,000 processes instead of the top 80 by CPU, so sorting by memory sees them all.
- **Search** in processes and containers: several words, each matching any field; "N of M" and
  `Esc` to clear.
- **CPU and memory columns for containers** from one `docker stats --no-stream` for all running
  containers; the list shows at once and the numbers follow.

### Remote desktop

- **Win, Alt+Tab, Ctrl+Esc and Alt+Shift go to the remote desktop.** Windows used to take them
  first. While the canvas of a live session has focus and shortcut capture is on, Serein installs
  a system keyboard hook; clicking outside, switching apps, a drop or closing the session returns
  the keyboard to Windows at once and releases held keys on the server. Leave with
  Ctrl+Alt+Pause or Ctrl+Alt+Home. Ctrl+Alt+Del stays a button; Win+L is not given up by Windows.

### Tray

- **Closing the main window hides Serein in the tray** instead of quitting: sessions, tunnels and
  transfers keep running. Click the icon to bring the windows back; the icon menu has "Open Serein"
  and "Quit", and the palette has "Quit Serein". On by default on Windows, off on Linux, where
  GNOME has no tray out of the box. If the icon cannot be placed, closing quits as before.

### Reliability

- **"Stop" in a task stops the current work,** not the next step: the connection, the step and a
  dry-run check are interrupted at once, and transfers drop the file at the next chunk with two
  seconds to remove their temporary files. A dry-run check has a limit of 120 seconds per step.
- **The SQL result limit covers the whole answer** — 5,000 rows and 16 MiB across all result sets,
  at most 100 sets; before, each set had its own limit.
- **MongoDB sign-in cannot make the machine compute for a server:** the hashing iteration count is
  capped at a million, salt and server reply by size, all checked before computing, which runs off
  the async thread, at most two at a time.

### Supply chain and tests

- `rustls` 0.23.45 in the application and the RDP helper for RUSTSEC-2026-0285; yanked
  `serialport` and `wnaf` versions replaced.
- CI actions are pinned by commit SHA and updated by Dependabot. The remaining `cargo audit`
  warnings (unmaintained and unsound `glib` on Linux) are listed in SECURITY.md with why they are
  not reachable.
- The SSH stand gained a port that accepts passwords only through keyboard-interactive and a real
  `ssh-agent`; new tests cover agent login, keyboard-interactive, interrupted uploads and downloads,
  permissions and symlinks on upload, and name conflicts.

### Checksums

```
```
