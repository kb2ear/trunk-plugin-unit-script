# Writing an uploader

Most plugins send calls somewhere: OpenMHz, Broadcastify Calls, rdio-scanner,
a webhook, cloud storage. They all need the same things: settings for each
system, the right audio format, uploads off the event thread, retries, and
honest reporting.

This guide builds one step by step. The
[OpenMHz plugin](https://github.com/TrunkRecorder/trunk-plugin-openmhz) is a
complete example, about 250 lines with its tests. Read it alongside.

## 1. Settings

Services usually give each system its own key, so the key goes in
`SystemConfig` (a system's sites share it; see [Settings](settings.md) for a
site a service knows on its own). The server goes in `Config`, with the real service as the
default:

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
struct Config {
    /// Upload server
    ///
    /// Leave this as it is, unless you run your own server.
    #[schemars(url)]
    server: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", default)]
struct SystemConfig {
    /// API key
    ///
    /// Leave it empty to not upload this system.
    #[schemars(extend("x-secret" = true))]
    api_key: String,
}
```

An empty key means "don't upload this system". Don't make it an error: most
people record more systems than they upload.

## 2. The manifest

Subscribe to `call.concluded`, and ask for M4A if the service takes it:

```rust
fn manifest() -> Manifest {
    Manifest {
        name: "OpenMHz".into(),
        subscribe: vec![topic::CALL_CONCLUDED.into()],
        audio_formats: vec![format::M4A.into()],
        ..trunk_recorder_plugin::manifest!()
    }
}
```

## 3. Check everything in `start`

Everything that would make every upload fail belongs in `start`, as an
`Err` the user can act on:

```rust
fn start(host: Host, setup: Setup<Config, SystemConfig>) -> Result<Self, String> {
    if !setup.has_format(format::M4A) {
        return Err("OpenMHz needs calls as M4A, and there's no M4A encoder on this computer. \
                    Install ffmpeg, then start recording again.".into());
    }
    let mut keys = HashMap::new();
    for s in &setup.systems {
        let Some(c) = &s.config else { continue };
        if !c.api_key.trim().is_empty() {
            host.info(format!("uploading {} (key …{})", s.short_name, last2(&c.api_key)));
            keys.insert(s.short_name.clone(), c.api_key.trim().to_string());
        }
    }
    if keys.is_empty() {
        return Err("Add your API key to the systems you want to upload.".into());
    }
    // …
}
```

A log line per system saying what will be uploaded, and where, saves users a
lot of guessing. **Never log a whole key.** The last two characters are enough
to tell keys apart.

## 4. Upload off the event thread, with `CallQueue`

An upload takes from a fraction of a second to a minute. `call_concluded` has
to return in milliseconds (see
[How plugins work](how-plugins-work.md#one-thread-return-quickly)). The SDK's
`CallQueue` does the rest:

- It works calls on background threads (2 by default).
- It retries failures after 10 s, 1 min, 5 min and 15 min, then gives up.
- It shows a warning status while calls wait to retry, and clears it when
  they're through.
- It reports each call's result for you.
- At shutdown, it keeps working until the grace period is nearly up. With
  `QueueOptions::saved_in(data_dir)`, it saves calls still waiting and sends
  them first at the next start.
- It reports its health to the recorder's dashboard (the Plugins page): how
  many calls wait, how long an upload takes, bytes sent, and whether the
  service is **up**, **degraded** (retries) or **down** (three failures in a
  row, or five minutes without a success while calls wait). Name the service
  with `endpoint`.

```rust
struct MyUploader {
    queue: CallQueue,
}

// in start:
let opts = QueueOptions { noun: "upload", endpoint: Some("My Service".into()), ..QueueOptions::saved_in(&setup.data_dir) };
let queue = CallQueue::start(host, opts, move |call: &ConcludedCall| {
    // By short name: a call saved for the next run still finds its system.
    let Some(key) = keys.get(&call.call.short_name) else {
        return Attempt::Skip("no API key for this system".into());
    };
    let Some(m4a) = &call.files.m4a else {
        return Attempt::Fail("this call couldn't be encoded as M4A".into());
    };
    upload(&server, key, &call.call, m4a)
});
Ok(MyUploader { queue })

fn call_concluded(&mut self, call: ConcludedCall) {
    self.queue.push(call);
}

fn shutdown(&mut self, grace: Duration) {
    self.queue.shutdown(grace);
}
```

The work function runs on the queue's threads, so it has to be `Send + Sync`.
Move what it needs (keys, an HTTP client) into the closure.

A plugin that doesn't use the queue (a streamer, say) can report the same
itself with `host.metrics(&Metrics { … })` every few seconds: any of
`queued`, `latency_ms`, `bytes_sent`, `endpoints` (each with its state), and
figures of its own in `extra`, shown as they are.

### One queue for each server

A system can change your plugin's settings for itself, so different systems
can upload to different servers (see [Settings](settings.md#settings-a-system-overrides)).
Read the server from `s.settings` for each system. Then start the queue with
`start_by`, which also takes a closure naming the server each call goes to.
Each server becomes a lane with its own workers, retries and up/down status,
named `My Service at <lane>` on the dashboard. A server that's down then
doesn't hold up uploads to the others:

```rust
let queue = CallQueue::start_by(host, opts, move |call| lanes.get(&call.call.short_name).cloned().unwrap_or_default(), work);
```

## 5. Say what happened: `Attempt`

Your upload function decides what each outcome means:

| Return | When | Then |
|---|---|---|
| `Attempt::Done { url }` | It worked. `url` links to the call on the service, if there's one (or `String::new()`). | Reported `ok`. |
| `Attempt::Skip(why)` | Not for this plugin: a system without a key, a talkgroup the service ignores. | Reported `skipped`. |
| `Attempt::Retry(why)` | It might work later: no network, a timeout, a 5xx, a 429. | Retried; `failed` after the last retry. |
| `Attempt::Fail(why)` | It won't work: a wrong key, an unknown system, a file it refuses. | Reported `failed`, now. |

Getting this right is most of what makes an uploader good. Retrying a wrong
key an hour later helps nobody. Failing a call because of a moment's network
trouble loses it.

Services often say what's wrong in the response body, not the status code.
OpenMHz answers most refusals with a 500 and a message. Read the service's
source or docs, and match its messages:

```rust
fn outcome(status: u16, text: &str) -> Attempt {
    if status == 200 {
        return Attempt::Done { url: String::new() };
    }
    if text.contains("API Keys do not match") {
        return Attempt::Fail("OpenMHz refused the API key".into());
    }
    if text.contains("Talkgroup does not exist") {
        return Attempt::Skip("OpenMHz ignores talkgroups it doesn't know".into());
    }
    match status {
        400..=499 if status != 408 && status != 429 => Attempt::Fail(format!("HTTP {status}")),
        _ => Attempt::Retry(format!("HTTP {status}: {}", text.trim())),
    }
}
```

Keep the explanation short and in the user's terms: it's what they see next to
the call.

## 6. The request

Use an HTTP client with Rust TLS, so the release builds cross-compile.
[`ureq`](https://docs.rs/ureq/3) is small and blocking, which suits
`CallQueue`'s threads:

```toml
[dependencies]
ureq = "3"
```

```rust
let agent: ureq::Agent = ureq::Agent::config_builder()
    .timeout_global(Some(Duration::from_secs(60)))
    // So a 500 comes back as a response you can read, not an error.
    .http_status_as_error(false)
    .user_agent(concat!("my-uploader/", env!("CARGO_PKG_VERSION")))
    .build()
    .into();
```

Set a timeout, and keep it well under a minute: a hung upload holds a queue
thread and delays shutdown.

Most upload services take `multipart/form-data`. The SDK builds the body, for
any client:

```rust
let audio = std::fs::read(m4a)?;
let file_name = m4a.file_name().unwrap().to_string_lossy();
let (body, content_type) = Multipart::new()
    .file("call", &file_name, "application/octet-stream", &audio)
    .text("talkgroup_num", call.talkgroup.to_string())
    .text("start_time", call.start_time.to_string())
    .text("api_key", key)
    .finish();
let resp = agent.post(&url).header("Content-Type", &content_type).send(&body);
```

Send the file under its real name. Services check the extension: OpenMHz
refuses anything but `.m4a` and `.mp3`.

## 7. Match what Trunk Recorder sent

If the service already takes uploads from Trunk Recorder, send exactly the
fields it sent, formatted the same way. The service's code was written against
it. The call's JSON (`call.call`) has Trunk Recorder's field names, and the
SDK sums what TR's uploaders summed: `error_count()`, `spike_count()`. Name
your settings so TR's can be pasted in: `#[serde(alias = "apiKey")]`.

TR's uploaders take talkgroup allow and deny lists of glob patterns
(`"507*"`, `"12?45"`). The SDK's `TalkgroupFilter` matches them the same way,
and `filter::patterns` reads a list that mixes numbers and strings:

```rust
#[serde(deserialize_with = "trunk_recorder_plugin::filter::patterns")]
#[schemars(with = "Vec<String>")]
talkgroup_allow: Vec<String>,
// …
let filter = TalkgroupFilter::new(&c.talkgroup_allow, &c.talkgroup_deny);
if !filter.passes(call.call.talkgroup) {
    return Attempt::Skip("talkgroup filter".into());
}
```

## 8. Test it without the service

`testing::MockServer` stands in for the service; see
[Testing](testing.md#testing-against-a-server). The tests worth having:

- A call uploads, with every field the service reads checked.
- Each refusal the service gives maps to the right `Attempt`.
- A server that's down leaves the call in `queue.jsonl`, not failed.
- `start` refuses no keys, a bad server address, and no M4A (if you need it).
- A system without a key is skipped, with no request made.

Then run it against a stand-in on your computer with
`trunk-pro plugin run`, and inside the recorder, before you release.
