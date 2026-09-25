## Serein v1.5.2

A fix release, and an important one: **confirmations before irreversible actions did not work
in 1.5.1 and earlier.** Update if you delete servers or files, run queries against databases or
edit files on servers from Serein. The release also brings the first user guide.

### Confirmations wait for an answer again

- Every "are you sure?" in Serein ran the action without waiting for the answer. The dialog
  plugin replaces the browser's `confirm` with an asynchronous one, so the check got a promise
  instead of the answer, and a promise always counts as "yes". In dialog plugin 2.7.2 the
  replacement also calls a command that no longer exists, so no dialog appeared at all.
- In practice: a `DELETE` or `UPDATE` without `WHERE`, `DROP`, `TRUNCATE` and `FLUSHALL` ran at
  once; saving a file in the editor after someone had changed it on the server overwrote their
  change; deleting servers, files, containers, tasks, profiles and groups, killing processes,
  stopping services, `compose down`, uploading from the folder comparison and closing a tab with
  unsaved edits - all happened without a question. Found while checking every feature for the
  user guide.
- Now each of the 22 places shows a system dialog with "Да" and "Отмена" and waits for the
  answer. If the dialog cannot be shown, the answer is "no". A lint rule forbids the plain
  `confirm`, so it cannot come back unnoticed.

### Run one command on several servers

- A host that answers instantly - for example one whose key has not been confirmed yet and is
  skipped - could be missing from the results, the summary and the saved report: the window
  subscribed to results after it had drawn itself. The final list now comes from the complete
  result the run returns.

### Import from `~/.ssh/config`

- **`ProxyJump` is carried over.** It used to be read and thrown away, so a server behind a
  bastion arrived as a direct connection. Now the gateway is set to the matching server from the
  same file or from the Serein list (by name, then by address). A chain of several gateways
  (`ProxyJump a,b`) cannot be expressed by one field: such servers are added as direct
  connections and named after the import, so the gateway can be set by hand.
- **A second import adds nothing.** Servers with the same address, port and user are skipped
  instead of being duplicated.
- Lines after a `Match` block no longer end up in the previous `Host`.

### Remote desktop setup

- An installed `xrdp` was not found for a regular user on Debian and Astra: it lives in
  `/usr/sbin`, which is not on their `PATH`, so the panel offered to install it again. The check
  now looks there too, for VNC as well.
- On a server without `ss` and `netstat`, a desktop listening only on IPv6 looked stopped.
  `/proc/net/tcp6` is read too.
- A server listening on `::1` was shown as `[::]`, which reads as "open to the network". The
  loopback is now shown as `[::1]`.

### User guide

- **A user guide with screenshots** in [docs/guide](docs/guide/README.md): where things are and
  how they work, from installation to the action log, in plain language. It is in Russian, like
  the interface. The same guide is attached to this release as **`Serein-user-guide-ru.pdf`**.
- README: there is no `Ctrl+Shift+M` shortcut for running a command on several servers (it is in
  the server list menu); a `DELETE` without `WHERE` asks once, not twice; the process list has no
  metrics on top - they are in the overview. The backup hint in Settings now mentions profiles
  and tasks, which the backup has contained since 1.4.

### Under the hood

- Tests: 391 Rust (383 in 1.5.1), 235 frontend (232), 13 in the RDP helper, plus the SSH stand.

### Checksums

```
```
