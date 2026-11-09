# Getting started

This walks you from the template to your own plugin running inside your
recorder. It takes about fifteen minutes, most of it compiling.

## What you need

- **Rust**, from [rustup.rs](https://rustup.rs). Any recent stable version.
- **Trunk Recorder Pro**, installed and set up to record something. You'll
  test against calls it has already recorded, so you don't need a radio
  attached while you work.
- **Optional: ffmpeg**, if your plugin wants M4A audio (see [Audio](audio.md)).

## 1. Make your repository

On GitHub, press **Use this template** on this repository. Name the new
repository after your plugin, for example `trunk-plugin-pager`. Clone it.

## 2. Name your plugin

A plugin has an **id**: lowercase letters, digits and dashes, like `pager` or
`mqtt-status`. The id names the executable, the release files, and the
plugin's entry in the registry. Once people have installed your plugin, the id
can't change. Pick it now.

Change these:

| File | Field | Example |
|---|---|---|
| `Cargo.toml` | `name`: the id | `pager` |
| `Cargo.toml` | `description`: one sentence, shown in the plugin store | `Pages your phone when a talkgroup goes active.` |
| `Cargo.toml` | `repository` | `https://github.com/you/trunk-plugin-pager` |
| `Cargo.toml` | `authors`, `license` | |
| `src/main.rs` | `name` in `manifest()`: the display name | `Pager` |
| `LICENSE` | Your license | |

Then tidy up what belongs to the template, not your plugin:

- **Delete `docs/`**. It documents the template; it lives on in the template's
  repository, where it's kept up to date.
- **Rewrite `README.md`** for your plugin's users. See
  [below](#a-readme-for-your-plugin).
- **Delete `examples/settings.json`**, or rewrite it with your settings.

## 3. Build it and ask it who it is

```sh
cargo build
./target/debug/pager --describe
```

`--describe` prints the plugin's **manifest**: its id, version, the events it
subscribes to, and the schema of its settings. The recorder reads this to
decide how to run the plugin and how to draw its settings form. See
[How plugins work](how-plugins-work.md).

## 4. Run it against calls you've recorded

`trunk-pro plugin run` starts a plugin the way the recorder does and sends
it calls from disk, each as a `call.concluded` event. It prints everything the
plugin says back:

```sh
trunk-pro plugin run ./target/debug/pager ~/TrunkRecorderPro --limit 5
```

```
M4A: none
[pager] status ok: running
[pager] ✓ sys1/2026/9/30/101-1790771550_857587500
[pager] ✓ sys1/2026/9/30/1039-1790771535_858587500
…
```

To give it settings, put them in a file, like [`examples/settings.json`](../examples/settings.json):

```json
{
  "config": { "file": "calls.jsonl" },
  "systems": { "sys1": { "label": "County" } }
}
```

```sh
trunk-pro plugin run ./target/debug/pager ~/TrunkRecorderPro --settings settings.json
```

`config` holds the plugin's settings. `systems` holds its settings for each
system, keyed by the system's short name. See [Settings](settings.md).

## 5. Write your plugin

Start with `src/main.rs`. The example shows each part a plugin has:

- `Config` and `SystemConfig`: the settings, with doc comments that become the
  form's labels.
- `manifest()`: the plugin's name and the events it subscribes to.
- `start()`: checks the settings and sets up. Returning an `Err` shows the user
  what's wrong.
- One method per event you subscribed to, here `call_concluded()`.
- `shutdown()`: finishes up before the recorder stops.

Read [How plugins work](how-plugins-work.md) before you do anything slow, such
as network calls, in an event method.

Run `cargo fmt` as you go. `rustfmt.toml` sets the recorder's own style (long
lines, compact expressions), and CI checks it. Keep `cargo test` passing too. The example's tests show how to drive a
plugin without the recorder. See [Testing](testing.md).

## 6. Run it inside your recorder

Open the recorder's **Plugins** page and press **Add from a file**. Give it the
path of your build, for example
`/Users/you/code/trunk-plugin-pager/target/release/pager`. The recorder runs it
with `--describe` and adds it, turned off, marked *your build*. Then:

1. Press **Set up**. It opens your plugin's card in Setup's **Plugins** tab,
   with the form drawn from your `Config`: this is where you see your labels,
   help text and defaults as users will.
2. Turn it **On**. Your `SystemConfig` form now shows on each system's card
   under **Systems**. Fill it in for a system or two.
3. Start recording. The card on the Plugins page shows whether it's running,
   how many calls it handled (from your `call_result`s), your status messages,
   and its recent log. Its log lines also go to the recorder's log, prefixed
   with its id.

Settings save as they're typed. While recording, a change to a plugin's
settings, or turning it off and on, restarts the plugins. After you rebuild,
turn yours off and on to run the new build.

It's all in the recorder's `config.json`:

- **macOS**: `~/Library/Application Support/trunk-pro/`
- **Linux**: `~/.config/trunk-pro/`
- **Windows**: `%APPDATA%\trunk-pro\`

```json
{
  "plugins": {
    "pager": {
      "enabled": true,
      "path": "/Users/you/code/trunk-plugin-pager/target/release/pager",
      "settings": { "file": "calls.jsonl" }
    }
  },
  "systems": [{ "shortName": "sys1", …, "plugins": { "pager": { "label": "County" } } }]
}
```

`path` points the recorder at your build instead of an installed copy.
`trunk-pro plugin list` shows what the recorder makes of it.

## A README for your plugin

Users read it on GitHub before they install, and reviewers read it before
listing your plugin. Cover:

```markdown
# Pager

Pages your phone when a talkgroup goes active. A plugin for Trunk Recorder Pro.

## What it needs
An ntfy.sh topic (free). Nothing to install.

## Settings
| Setting | |
|---|---|
| Topic | Your ntfy.sh topic. |
| Talkgroups | The talkgroups to page for, by number. |

## What it sends where
A notification with the talkgroup's name to ntfy.sh, for each call on your talkgroups. Nothing else.
```

"What it sends where" matters: users are trusting your plugin with their
recorder, and the [registry](registry.md) review checks it.

## 7. Release it

Tag a version and push the tag:

```sh
git tag v0.1.0
git push --tags
```

The release workflow builds your plugin for Linux (x86-64 and ARM64), macOS and
Windows, and attaches the builds to a GitHub release along with checksums. See
[Releasing](releasing.md). Then [submit it to the registry](registry.md), so
people can install it from the recorder's plugin store.
