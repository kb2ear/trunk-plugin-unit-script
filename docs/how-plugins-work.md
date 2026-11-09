# How plugins work

## A plugin is a separate program

The recorder runs each enabled plugin as its own process. It writes events to
the plugin's **stdin** and reads the plugin's replies from its **stdout**, one
JSON message per line. The plugin's **stderr** goes into the recorder's log.

Because of this:

- **A plugin can't crash the recorder.** If a plugin dies, the recorder logs it
  and starts it again. Recording goes on.
- **A plugin can't slow the recorder down.** Events are queued for each plugin.
  A plugin that falls far behind loses events; the recorder never waits for it.
- **Plugins only watch.** Nothing a plugin says changes what gets recorded,
  which radios are followed, or anything else about the recorder.
- **Plugins can be written in any language.** This template uses the Rust SDK,
  [`trunk-recorder-plugin`](https://docs.rs/trunk-recorder-plugin), which
  handles the protocol for you. [Protocol](protocol.md) describes the wire
  format for other languages.

## Lifecycle

```
recorder                                   plugin
────────                                   ──────
run `plugin --describe`  ───────────────►  prints its manifest, exits
                                           (id, version, topics, settings schema)
start `plugin` (no arguments)
hello  ─────────────────────────────────►  Plugin::start(host, setup)
       ◄─────────────────────────────────  ready          (or: status error, exit 78)
call.concluded  ────────────────────────►  Plugin::call_concluded(call)
       ◄─────────────────────────────────  call.result, log, status …
…
shutdown  ──────────────────────────────►  Plugin::shutdown(grace)
close stdin                                exits
(kill, if it's still running after the grace period)
```

1. **Describe.** Before starting a plugin, the recorder runs it with
   `--describe` and reads its [manifest](#the-manifest). The manifest tells the
   recorder which events to send and which audio formats to prepare. If the
   plugin needs a newer plugin API than the recorder has, it isn't started.
2. **Start.** The recorder starts the plugin with no arguments, in its data
   folder, and sends a `hello`: the plugin's settings, the systems being
   recorded, the capture folder, the plugin's data folder, and the audio formats
   calls will come with. The SDK parses the settings into your `Config` types
   and calls `Plugin::start`.
3. **Ready.** If `start` returns `Ok`, the SDK sends `ready`, and the recorder
   shows the plugin as running. If it returns `Err`, the SDK reports the error
   and exits with status 78, which tells the recorder the settings are wrong.
   The recorder doesn't restart the plugin while it keeps recording; it tries
   again the next time recording starts (after the user has fixed the settings,
   or installed what was missing).
4. **Events.** The SDK calls your method for each event, one at a time, in
   order.
5. **Shutdown.** When recording stops, the recorder sends `shutdown` and closes
   stdin. The SDK calls `Plugin::shutdown(grace)`, and the process exits when
   it returns. Anything still running after the grace period (10 seconds) is
   killed.

Plugins run while the recorder records. Starting and stopping recording starts
and stops them.

## The manifest

`Plugin::manifest()` returns it. Start from `trunk_recorder_plugin::manifest!()`,
which fills in the id, version, description, repository, authors and license
from `Cargo.toml`:

```rust
fn manifest() -> Manifest {
    Manifest {
        name: "Pager".into(),
        subscribe: vec![topic::CALL_START.into()],
        ..trunk_recorder_plugin::manifest!()
    }
}
```

| Field | |
|---|---|
| `id` | The plugin's permanent id (the crate name). |
| `name` | Display name. |
| `version` | Semver (the crate version). |
| `description` | One sentence for the plugin store. |
| `subscribe` | The [topics](events.md) to receive. Nothing else is sent. The recorder doesn't even build events nobody subscribes to, so subscribe only to what you use. |
| `audio_formats` | Extra formats for `call.concluded`: `["m4a"]`. See [Audio](audio.md). |
| `config`, `system_config` | Filled in by the SDK from your `Config` and `SystemConfig` types. See [Settings](settings.md). |

## One thread: return quickly

Your event methods run on the thread that reads stdin, one event at a time.
While a method runs, the next events wait in the plugin's queue. The queue
holds about a thousand. If it fills up, the recorder drops events for that
plugin and logs that it's falling behind.

So an event method should return in milliseconds. Hand anything slower to
another thread: network requests, spawning programs, big file copies.

- **For calls**, use the SDK's `CallQueue`. It runs your function on
  background threads, retries with backoff, reports results, and saves what's
  left at shutdown. See [Writing an uploader](uploading.md).
- **For other events** (live audio to a stream, unit events to a database),
  start a thread in `start` and send it work over a channel
  (`std::sync::mpsc`). Join it in `shutdown`.

The template's example ([`src/main.rs`](../src/main.rs)) writes a line to a
file inline, which is fast enough.

## Talking back

`Plugin::start` gets a `Host`. Keep it; it's how the plugin reports back. It's
cheap to clone and works from any thread.

| | Shown |
|---|---|
| `host.info(…)`, `warn`, `error`, `debug` | In the recorder's log, prefixed with the plugin's id. `debug` lines are dropped. |
| `host.status(State::Warning, "OpenMHz is down; 12 calls queued")` | Next to the plugin in the plugins list, until the next status. |
| `host.call_result(path, Outcome::Ok, "", url)` | What became of a call: `Ok`, `Skipped` (deliberately not handled) or `Failed`. |

> **Never print to stdout.** A plugin's stdout carries the protocol, so
> `println!` sends garbage to the recorder. The recorder logs a line it can't
> read rather than choke on it, but use `host.info()`, or `eprintln!` for raw
> output.

## When things go wrong

| What happens | What the recorder does |
|---|---|
| `start` returns `Err` (exit status 78) | Shows the error. Doesn't restart the plugin until recording next starts. |
| The plugin panics or exits | Logs it and restarts it after 1 s, then 2 s, 4 s, … up to a minute between tries. The delay resets once the plugin has run for a minute. Events wait in the queue meanwhile. |
| The plugin falls behind | Drops that plugin's events once its queue is full, and logs how many. |
| `--describe` fails or prints nonsense | Doesn't start the plugin, and says why. |
| It's still running after the shutdown grace period | Kills it. |

## Files

- **Data folder** (`setup.data_dir`): the plugin's own, kept across restarts and
  upgrades. Put queues, state and caches here. The process starts with this
  folder as its working directory.
- **Capture folder** (`setup.capture_dir`): where the recorder keeps calls.
  Read the files events point to, but don't change or delete them. Other
  plugins, and the recorder's call history, use them too.

## Trust

A plugin runs as the same user as the recorder, with the same access to files
and the network. Plugins in the [registry](registry.md) are reviewed before
they're listed, and pinned to the exact builds that were reviewed. Users
choose what they install.
