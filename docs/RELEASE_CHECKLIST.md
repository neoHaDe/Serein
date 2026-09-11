# Release checklist

The steps were scattered across `LINUX_MIGRATION.md`, npm scripts and habit. Collected here
because the two ways a release goes wrong are both quiet: a step skipped, or a step done in the
wrong order. Every item below exists because it has been missed at least once.

## Before building

- [ ] `node scripts/set-version.mjs X.Y.Z` — sets `package.json` and `src-tauri/Cargo.toml`.
      Do not edit either by hand; `tauri.conf.json` follows `package.json` and needs no change.
      A mismatch now fails the build (`build.rs`) instead of shipping.
- [ ] **Both READMEs updated for the release** - `README.md` and `README.ru.md`. Version in the
      headline, the install list, the release-notes link, the installer size in the comparison
      table, and whatever the release actually added or removed. The two files must stay
      equivalent section by section and bullet by bullet: they have drifted before, and a
      reader of one of them has no way to notice.
- [ ] `docs/RELEASE_NOTES_vX.Y.Z.md` written from the running draft in the vault
      (`02-Projects/term-tauri/since-release-terminal.md`), SHA-256 block left empty for now.
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` — unit tests green.
- [ ] `cargo test --manifest-path rdp-helper/Cargo.toml --locked` — helper tests green.
- [ ] `npm run typecheck` — clean.
- [ ] Integration suite against the stand, if the release touches SSH or SFTP:
      `scripts/ssh-stand/up.sh`, then `cargo test -- --ignored`, then `down.sh`.
      CI runs this too; running it locally first saves a red push.
- [ ] `cargo audit --file src-tauri/Cargo.lock --ignore RUSTSEC-2023-0071` — no new advisories.
- [ ] `npm audit --omit=dev` — production dependencies clean.

## Building

- [ ] **Linux, on the build VM:** `./scripts/build-linux.sh` → `.deb` + AppImage.
      Copy the AppImage back into `src-tauri/target/release/bundle/appimage/`.
- [ ] **Windows, locally:** `bash scripts/build-rdp-helper.sh release`, then
      `npm run tauri -- build` with `TAURI_SIGNING_PRIVATE_KEY` and its password in the
      environment. Run `powershell -ExecutionPolicy Bypass -File scripts/package-windows-portable.ps1`.
      Portable is a ZIP with `Serein.exe` and `serein-rdp.exe`; a single copied EXE has no RDP.
- [ ] `./scripts/make-sbom.sh` — CycloneDX for Rust and npm into `dist/sbom/`.

## Publishing

- [ ] `sha256sum` every published artifact; paste into the release notes; commit.
- [ ] `git tag -a vX.Y.Z` and push the tag.
- [ ] `gh release create vX.Y.Z --notes-file docs/RELEASE_NOTES_vX.Y.Z.md <artifacts>`.
      Attach the SBOM files as well.
- [ ] Unpack the portable ZIP into a new empty directory and start RDP from that copy.
      Check that `serein-rdp.exe` is present next to `Serein.exe`; testing from `target/`
      can accidentally find a developer build and hide an incomplete archive.
- [ ] `npm run manifest -- X.Y.Z "заметки"` — signs whatever is unsigned and writes **two**
      manifests. If the signing step fails with a bare `try '--help'`, the flags are fine: the
      npm wrapper mangles arguments on Windows. Sign by hand with `npx tauri signer sign`
      and re-run.

      Two manifests, because the updater has no per-file fallback: the URL inside a manifest
      is a single string. What it does support is a **list of manifests**, tried in order.
      So `latest.json` carries GitHub URLs and `latest.mirror.json` carries ours, and the app
      falls back from one host to the other. Signatures are identical in both — minisign signs
      the bytes of the file, not the address it came from.

      GitHub is first: it is faster and does not load the home server. Ours is second, so a
      blocked or simply unreachable github.com does not stop updates for anyone. That is also
      the answer to the registry requirement about foreign parties being able to limit the
      software: there is no single foreign point of control.
- [ ] Attach the GitHub-flavoured manifest to the release itself — the primary endpoint reads
      it from there:

      ```
      gh release upload vX.Y.Z <bundle>/latest.json --clobber
      ```
- [ ] Upload the updater artifacts **and** the mirror manifest to our own host — the installer
      and the AppImage named exactly as in the manifest, the manifest itself renamed to
      `latest.json`. The script prints these exact commands; copy them from its output rather
      than retyping paths.
- [ ] Compare checksums against the local files. A truncated upload still answers `200`, and a
      broken AppImage looks exactly like a working one until somebody installs it.
- [ ] **Only after the artifacts are actually uploaded**, put the manifests in place. This is
      the step that turns the release on for everyone: a manifest pointing at files that are not
      there yet breaks auto-update for every user at once.
- [ ] Verify **both** endpoints from outside — each shows the new version, and every URL inside
      answers `200` with the right size:

      ```
      curl -sL https://github.com/neoHaDe/Serein/releases/latest/download/latest.json
      curl -s  https://nehade.xyz/updates/terminal/latest.json
      ```

      Keep the previous mirror manifest as `latest.json.<something>.bak` until the new one is
      confirmed: rolling back is then one `cp`.

## After

- [ ] Reset the running draft in the vault: heading plus the box naming the new version, empty list.
- [ ] Update the roadmap if the release closed any items.
- [ ] Install the update on a machine running the previous version and open a session. An update
      that installs but will not connect is worse than no update.

## Known traps

- **The signing key never goes into CI.** A Tauri signature is minisign over the artifact bytes,
  so it can be produced after the build — there is no reason to copy the key onto a build host
  or into a runner.
- **The Linux `.deb` cannot self-update**; only the AppImage can. The app already detects this
  and offers the release link instead of an install that would fail.
- **Windows builds are unsigned.** SmartScreen will warn. The SHA-256 in the notes is the check
  that actually means something.
