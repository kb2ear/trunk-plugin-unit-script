# The plugin registry

The recorder's plugin store, on its **Plugins** page (and `trunk-pro plugin
search | install | update`), installs from this registry.

The registry is a GitHub repository, [`TrunkRecorder/plugins`](https://github.com/TrunkRecorder/plugins), listing the
plugins the recorder's plugin store offers. Each entry pins an exact release
of a plugin by checksum. The recorder installs only files that match, so what
users get is what was reviewed.

## An entry

Each plugin has a file, `plugins/<id>.json`:

```json
{
  "id": "openmhz",
  "name": "OpenMHz",
  "description": "Uploads recorded calls to OpenMHz.",
  "repository": "https://github.com/TrunkRecorder/trunk-plugin-openmhz",
  "homepage": "https://openmhz.com",
  "license": "GPL-3.0-or-later",
  "tier": "official",
  "version": "0.1.1",
  "api": 1,
  "tag": "v0.1.1",
  "commit": "669ea2d10b193728b82276ec9ed77c81118f164f",
  "assets": {
    "x86_64-unknown-linux-gnu": {
      "url": "https://github.com/TrunkRecorder/trunk-plugin-openmhz/releases/download/v0.1.1/openmhz-0.1.1-x86_64-unknown-linux-gnu.tar.gz",
      "sha256": "74c54539…"
    },
    "aarch64-unknown-linux-gnu": { "url": "…", "sha256": "…" },
    "universal-apple-darwin": { "url": "…", "sha256": "…" },
    "x86_64-pc-windows-msvc": { "url": "…", "sha256": "…" }
  }
}
```

`tier` is `official` for plugins maintained with the recorder, and `community`
for everyone else's. The store shows the difference.

`commit` is the commit the tag pointed at when the entry was added: a tag can
be moved, so the commit records which source was reviewed.

You don't write an entry by hand. The registry's `add-release` script writes
it from your release's manifest and `SHA256SUMS`.

## Getting listed

1. Release your plugin with the template's release workflow (see
   [Releasing](releasing.md)).
2. Open a pull request on the registry that adds `plugins/<id>.json`. Run the
   registry's `add-release` script to write it, then `check` to check it:
   `./add-release https://github.com/you/trunk-plugin-pager v0.1.0`.
3. CI checks every file's checksum and build provenance, and runs the Linux
   build's `--describe`. A maintainer reviews the rest.

The registry's [README](https://github.com/TrunkRecorder/plugins#readme) has
the details.

What review looks for:

- **The source is public**, and the release was built from it by GitHub
  Actions, not uploaded from someone's computer. The release workflow's
  provenance attestations show this (see [Releasing](releasing.md)).
- **It does what it says**, and nothing else. In particular, it sends no data
  anywhere the user didn't configure.
- **Secrets are marked** `x-secret`, and never logged.
- **It copes with missing M4A** if it asks for M4A (see [Audio](audio.md)).
- **It starts with its default settings**, or explains in `start` what to set.
- **It's licensed**, and the id isn't taken.

## Updating

Release the new version, then open a pull request that updates your entry's
entry. `add-release` writes it.

The store shows **Update available** when the registry has a newer version
than the one installed. Users update when they choose. Settings carry over.

## How the recorder installs a plugin

1. It downloads the archive for its platform from the entry's `url`.
2. It checks the archive's SHA-256 against the entry, and stops if they differ.
3. It unpacks the archive into `plugins/<id>/` in its config folder (next to
   `config.json`).
4. It runs `<id> --describe`, and checks that the id matches the entry and that
   it supports the plugin's `api`.
5. It shows the plugin's settings form, and the user turns it on.

The recorder fetches the registry's list when the store first needs it, and
then uses that copy for 15 minutes before fetching it again (60 seconds, after
a fetch that failed). **Check again** on the Plugins page fetches it at once,
as does each `trunk-pro plugin search | install | update`. So a new entry, once
merged, shows up in a running recorder within about 15 minutes (GitHub caches
the file for a few minutes too).

When the registry can't be reached, the recorder uses the list it fetched last,
and failing that the copy it ships with, so the store works offline.

## Unlisted plugins

A plugin doesn't have to be in the registry to run. The store can install one
from a GitHub release URL, with a warning that nobody has reviewed it. And for
development, **Add from a file** on the Plugins page runs any executable. See
[Getting started](getting-started.md#6-run-it-inside-your-recorder).
