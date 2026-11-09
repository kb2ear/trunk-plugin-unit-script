# Releasing

## Versions

Use [semver](https://semver.org) in `Cargo.toml`:

- **Patch** (0.1.0 → 0.1.1): fixes.
- **Minor** (0.1.1 → 0.2.0): new features and new settings.
- **Major** (0.2.0 → 1.0.0): changes that break existing settings, or need
  users to do something.

Keep a `CHANGELOG.md` with a `## [0.1.1]` heading for each version. The release
workflow uses that section as the release notes.

## Making a release

1. Set the version in `Cargo.toml`, run `cargo build` so `Cargo.lock` follows,
   and commit both.
2. Tag the commit with the version, prefixed with `v`, and push the tag:
   ```sh
   git tag v0.1.1
   git push origin v0.1.1
   ```
3. The **Release** workflow checks that the tag matches `Cargo.toml`, then
   builds and publishes a GitHub release with:

   | File | |
   |---|---|
   | `<id>-<version>-x86_64-unknown-linux-gnu.tar.gz` | Linux, Intel/AMD 64-bit |
   | `<id>-<version>-aarch64-unknown-linux-gnu.tar.gz` | Linux, ARM 64-bit (Raspberry Pi 3/4/5 with a 64-bit OS) |
   | `<id>-<version>-universal-apple-darwin.tar.gz` | macOS, Apple silicon and Intel |
   | `<id>-<version>-x86_64-pc-windows-msvc.zip` | Windows |
   | `<id>-<version>.manifest.json` | What `--describe` prints |
   | `SHA256SUMS` | Checksums of all of the above |

   Each archive holds a folder with the executable, `README.md` and `LICENSE`.

   Every file also gets a **build provenance attestation**: a signed record,
   kept by GitHub, that Actions built it from your repository at the tagged
   commit. It's what lets the registry trust that a release came from your
   source. Anyone can check a file with
   `gh attestation verify <file> --repo <you>/<repo>`.

These are the platforms Trunk Recorder Pro itself ships for. The Linux builds
link against glibc 2.28, so they run on any distribution from about 2019 on.

**Keep the file names.** The recorder's installer finds the file for its
platform by name, and the registry pins them by checksum.

## Before you tag

- `cargo fmt --check`, `cargo clippy -- -D warnings` and `cargo test` pass, and CI is green.
- `--describe` shows the right name, description and repository. The release
  workflow refuses the template's placeholders.
- You've run the release build against real calls with `trunk-pro plugin run`.
- New settings have defaults, so existing installs keep working. See
  [Settings](settings.md#changing-your-settings-later).

## Dependencies that make releases hard

The release builds cross-compile. Pure Rust dependencies make that easy; C
libraries don't. In particular:

- **HTTP**: use a client with `rustls` for TLS (`ureq`, or `reqwest` with
  `rustls-tls`), not OpenSSL.
- **Other programs**: don't assume `curl`, `python` or `ffmpeg` exist on the
  user's machine. If you need one, check for it in `start` and say what to
  install.

## Then

Releasing on GitHub doesn't put your plugin in front of users. The
[registry](registry.md) does: submit the new version there.
