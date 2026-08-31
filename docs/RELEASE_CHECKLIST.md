# Release checklist

The steps were scattered across `LINUX_MIGRATION.md`, npm scripts and habit. Collected here
because the two ways a release goes wrong are both quiet: a step skipped, or a step done in the
wrong order. Every item below exists because it has been missed at least once.

## Before building

- [ ] `node scripts/set-version.mjs X.Y.Z` — sets `package.json` and `src-tauri/Cargo.toml`.
      Do not edit either by hand; `tauri.conf.json` follows `package.json` and needs no change.
      A mismatch now fails the build (`build.rs`) instead of shipping.
- [ ] `docs/RELEASE_NOTES_vX.Y.Z.md` written from the running draft in the vault
      (`02-Projects/term-tauri/since-release-terminal.md`), SHA-256 block left empty for now.
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` — unit tests green.
- [ ] `npm run typecheck` — clean.
- [ ] Integration suite against the stand, if the release touches SSH or SFTP:
      `scripts/ssh-stand/up.sh`, then `cargo test -- --ignored`, then `down.sh`.
      CI runs this too; running it locally first saves a red push.
- [ ] `cargo audit --file src-tauri/Cargo.lock --ignore RUSTSEC-2023-0071` — no new advisories.
- [ ] `npm audit --omit=dev` — production dependencies clean.

## Building

- [ ] **Linux, on the build VM:** `./scripts/build-linux.sh` → `.deb` + AppImage.
      Copy the AppImage back into `src-tauri/target/release/bundle/appimage/`.
- [ ] **Windows, locally:** `npm run tauri -- build` with `TAURI_SIGNING_PRIVATE_KEY` and its
      password in the environment. Copy `target/release/serein.exe` to
      `bundle/nsis/Serein_X.Y.Z_x64-portable.exe`.
- [ ] `./scripts/make-sbom.sh` — CycloneDX for Rust and npm into `dist/sbom/`.

## Publishing

- [ ] `sha256sum` all four artifacts; paste into the release notes; commit.
- [ ] `git tag -a vX.Y.Z` and push the tag.
- [ ] `gh release create vX.Y.Z --notes-file docs/RELEASE_NOTES_vX.Y.Z.md <four artifacts>`.
      Attach the SBOM files as well.
- [ ] `npm run manifest -- X.Y.Z "заметки"` — signs whatever is unsigned and writes `latest.json`.
      If the signing step fails with a bare `try '--help'`, the flags are fine: the npm wrapper
      mangles arguments on Windows. Sign by hand with `npx tauri signer sign` and re-run.
- [ ] **Only after the artifacts are actually uploaded**, copy `latest.json` to the site. This is
      the step that turns the release on for everyone: a manifest pointing at files that are not
      there yet breaks auto-update for every user at once.
- [ ] Verify from outside: `curl https://nehade.xyz/updates/terminal/latest.json` shows the new
      version and both platforms.

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
