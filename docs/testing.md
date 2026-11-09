# Testing

There are three ways to exercise a plugin, from fastest to most real:

1. **Unit tests** drive the plugin in-process with made-up events.
2. **`trunk-pro plugin run`** runs the built plugin against calls you've
   recorded.
3. **Inside the recorder**, added with **Add from a file** (`path` in its config).

## Unit tests

The SDK's `testing` module runs a plugin the way the recorder does, without
the recorder:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use trunk_recorder_plugin::{testing, HostMessage, Outcome};

    #[test]
    fn logs_a_call() {
        let dir = testing::temp_dir("my-plugin");
        let mut hello = testing::hello(&dir, json!({ "file": "out.jsonl" }));
        hello.systems[0].config = json!({ "label": "County" });
        let call = testing::call(&dir, "sys1", 101);

        let out = testing::run::<MyPlugin>([HostMessage::Hello(hello), HostMessage::CallConcluded(call.clone())]);

        assert!(out.ready());
        assert_eq!(out.results()[0].1, Outcome::Ok);
    }
}
```

| | |
|---|---|
| `testing::temp_dir(name)` | A fresh, empty folder. |
| `testing::hello(dir, config)` | A `hello` with your settings as JSON, and one P25 system, `sys1` (index 0). Its settings for that system are `hello.systems[0].config`. The data folder is `dir/data`. Calls come as WAV and M4A; set `hello.audio_formats = vec!["wav".into()]` for a recorder with no M4A encoder. |
| `testing::call(dir, short_name, tg)` | A 3-second call on system 0, with its files written under `dir`: the JSON, a silent WAV, and a placeholder M4A in `files.m4a` (not real audio). Set `files.m4a = None` for a call that couldn't be encoded, or `system` for another system. |
| `testing::run::<P>(messages)` | Runs the plugin on the messages, then ends its input, so `shutdown` runs, and returns what it said. |
| `out.ready()` | It started: its settings were good. |
| `out.exit_code` | `EXIT_CONFIG` (78) if `start` refused the settings. |
| `out.results()`, `out.logs()`, `out.status()` | What it reported. |
| `testing::capture()` | A `Host` that keeps what it's sent, for testing a part of your plugin on its own. `captured.output()` reads it back. |

Things worth a test:

- Good settings start the plugin; each kind of bad setting doesn't, with a
  message that says what to fix.
- A call is handled; calls you skip on purpose are reported as `Skipped`.
- No M4A (`files.m4a = None`) is handled the way you decided.
- A failure (a server that's down, say) is reported as `Failed`, not lost.
- Calls in flight are finished or saved at shutdown.

### Testing against a server

Keep tests off the internet. `testing::MockServer` is a web server on
`127.0.0.1` that answers with a function of yours and keeps every request:

```rust
use trunk_recorder_plugin::testing::MockServer;

let server = MockServer::start(|req| {
    let key = String::from_utf8(req.form_field("api_key").unwrap_or_default()).unwrap();
    if key == "good" { (200, String::new()) } else { (500, "API Keys do not match!\n".into()) }
});
let hello = testing::hello(&dir, json!({ "server": server.url() }));
// … run the plugin …
let req = &server.requests()[0];
assert_eq!(req.path, "/sys1/upload");
assert_eq!(req.form_field("talkgroup_num").unwrap(), b"101");
assert!(req.form_file_name("call").unwrap().ends_with(".m4a"));
```

`Request` has `method`, `path`, `headers`, `body`, `header(name)`, and
`form_field` / `form_file_name` for `multipart/form-data` bodies.

Answer the way the real service does, errors included. Copy its error
messages exactly, because your plugin tells them apart. For a server that's
down, point the plugin at a port nothing listens on, like
`http://127.0.0.1:9`. See the
[OpenMHz plugin's tests](https://github.com/TrunkRecorder/trunk-plugin-openmhz/blob/main/src/main.rs)
for a full set.

## Against your recorded calls

`trunk-pro plugin run` starts your build the way the recorder does and sends
it calls from disk, each as `call.concluded`:

```sh
cargo build
trunk-pro plugin run ./target/debug/my-plugin ~/TrunkRecorderPro --limit 5 --settings settings.json
```

| Option | |
|---|---|
| `<calls…>` | Call `.json` files, or folders. For a folder, the newest `--limit` calls in it (default 10). |
| `--settings file.json` | `{"config": {…}, "systems": {"<short name>": {…}}}` |
| `--capture-dir dir` | The calls' capture folder. Defaults to the recorder's. |
| `--encoder auto\|ffmpeg\|afconvert\|fdkaac\|none` | To test with, or without, M4A. |
| `--grace 30` | Seconds the plugin gets to finish after the last call. |
| `--data-dir dir` | The plugin's data folder. By default it's a folder in the temp folder that's kept between runs, never the installed plugin's, so a test run doesn't leave work for the real plugin. |

It prints the plugin's log lines and statuses, and a line for each result:
`✓` done, `–` skipped, `✗` failed.

If you give it an id instead of a path, it runs that installed plugin with its
settings from the recorder's config.

Calls you give it that are already in the recorder's capture folder keep
their usual `path`. Others use their full path as the `path`.

To try a real service without touching the real one, run a stand-in for it on
your computer and point `server` (or whatever your setting is) at it. When
`plugin run` is happy, run the plugin inside the recorder for a while: that's
the only place the timing is real.

## By hand

The protocol is JSON lines, so you can also type at a plugin:

```sh
./target/debug/my-plugin
{"type":"hello","api":1,"config":{},"systems":[{"index":0,"short_name":"sys1","kind":"p25"}]}
```

It answers `{"type":"ready"}` if it started. See [Protocol](protocol.md).

## Developing against an unreleased SDK

To try changes to the SDK itself, point Cargo at a local checkout of the
recorder's repository. Create `.cargo/config.toml` (git ignores it):

```toml
[patch.crates-io]
trunk-recorder-plugin = { path = "../trunk-recorder-pro/crates/trunk-recorder-plugin" }
```

Delete the file, then run `cargo update -p trunk-recorder-plugin`, before
committing `Cargo.lock`. Otherwise the lock file records your local path, and
the release build fails.
