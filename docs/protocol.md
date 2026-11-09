# Protocol

This is the wire protocol between the recorder and a plugin, for writing a
plugin in a language other than Rust. The Rust SDK implements all of it. The
Rust types in [`protocol.rs`](https://docs.rs/trunk-recorder-plugin/latest/trunk_recorder_plugin/protocol/index.html)
are the reference.

This is plugin API version **1**.

## Transport

- The recorder runs the plugin's executable **with no arguments**, with its
  working directory set to the plugin's data folder.
- The recorder writes to the plugin's **stdin**, and reads the plugin's
  **stdout**. Each message is one JSON object on one line, UTF-8, ending in
  `\n`.
- The plugin's **stderr** is read line by line into the recorder's log.
- **Both sides ignore** message types and fields they don't know. New fields
  and types can appear within an API version; removing or changing one needs a
  new version.

## `--describe`

Run with the single argument `--describe`, a plugin prints its manifest as one
JSON object on stdout and exits 0:

```json
{
  "id": "openmhz",
  "name": "OpenMHz",
  "version": "1.0.0",
  "description": "Uploads calls to OpenMHz.",
  "api": 1,
  "subscribe": ["call.concluded"],
  "audio_formats": ["m4a"],
  "config": { "type": "object", "properties": { … }, "x-order": [ … ] },
  "system_config": { "type": "object", "properties": { … }, "x-order": [ … ] },
  "repository": "https://github.com/TrunkRecorder/trunk-plugin-openmhz",
  "authors": ["…"],
  "license": "GPL-3.0-or-later"
}
```

| Field | Required | |
|---|---|---|
| `id` | yes | `[a-z0-9][a-z0-9-]*` |
| `name`, `version`, `description` | yes | |
| `api` | yes | The plugin API version it speaks. The recorder refuses a higher one than it knows. |
| `subscribe` | yes | Topics: `call.start`, `call.end`, `call.concluded`, `unit`, `audio`, `status`. |
| `audio_formats` | no | `["m4a"]` to get M4A with `call.concluded`. |
| `config`, `system_config` | no | [Settings schemas](#settings-schemas). Leave them out for none. |
| `homepage`, `repository`, `authors`, `license` | no | |

It must finish within 10 seconds.

## Recorder → plugin

Every message has a `type`. The first is always `hello`.

### `hello`

```json
{
  "type": "hello",
  "api": 1,
  "host": { "name": "trunk-pro", "version": "0.4.0" },
  "config": { "server": "https://api.openmhz.com" },
  "systems": [
    { "index": 0, "short_name": "county", "kind": "p25", "config": { "apiKey": "…" } },
    { "index": 1, "short_name": "city", "kind": "p25", "config": { "apiKey": "…" },
      "settings": { "server": "https://openmhz.example.org" } },
    { "index": 2, "short_name": "conv", "kind": "conventional", "config": null }
  ],
  "capture_dir": "/home/me/TrunkRecorderPro",
  "data_dir": "/home/me/.config/trunk-pro/plugin-data/openmhz",
  "audio_formats": ["wav", "m4a"]
}
```

- **`config`** holds the plugin's settings as the user entered them. It's
  `null` when there are none.
- A system's **`config`** holds the plugin's settings for that system, or is
  `null`.
- A system's **`settings`**, when it's there, is the plugin's `config` with
  the changes that system makes to it (another server, say). Without it, the
  plugin's `config` applies to the system as it is.
- **`audio_formats`** lists what `call.concluded` will carry: always `wav`,
  plus the formats the plugin asked for that the recorder can make.

The plugin answers `ready` when it's running. If it can't run with these
settings, it sends a `status` of `error` saying why, and exits with status
**78**.

### Events

`call.concluded`, `call.start`, `call.end`, `unit`, `audio` and `status`. Each
message is the event's fields plus `type`. See [Events](events.md).

### `shutdown`

```json
{ "type": "shutdown", "grace_s": 10 }
```

The recorder is stopping. Finish or save what's in progress, then exit. The
recorder closes stdin after this message. It kills the process if it's still
running `grace_s` seconds later. Treat stdin closing without a `shutdown` the
same way.

## Plugin → recorder

### `ready`

```json
{ "type": "ready" }
```

### `log`

```json
{ "type": "log", "level": "info", "message": "Uploading calls for county" }
```

`level` is `error`, `warn`, `info` or `debug`. The recorder drops `debug`.

### `status`

```json
{ "type": "status", "state": "warning", "message": "OpenMHz isn't answering; 12 calls waiting" }
```

`state` is `ok`, `warning` or `error`. It's shown with the plugin until the
next status.

### `call.result`

```json
{ "type": "call.result", "path": "county/2026/9/30/101-1790771550_857587500", "outcome": "ok", "message": "", "url": "https://openmhz.com/system/county?call=…" }
```

`path` is the `path` of the `call.concluded`. `outcome` is `ok`, `skipped`
(not handled, on purpose) or `failed` (given up on). `message` and `url` are
optional.

A line that isn't JSON is logged as it is.

## Exit status

| Status | Meaning | The recorder |
|---|---|---|
| 0 | Stopped as asked | Nothing. |
| 78 | Can't run with these settings | Shows the error, and doesn't restart it until recording next starts. |
| Anything else, or a signal | Crashed | Restarts it after 1, 2, 4, … up to 60 seconds. |

## Delivery

The recorder queues about a thousand messages for each plugin, and drops new
ones while the queue is full. Messages queued while a plugin restarts are
delivered to the new process, after its `hello`.

## Settings schemas

`config` and `system_config` are [JSON Schema](https://json-schema.org)
objects. The recorder's settings form understands this subset:

| Schema | Form |
|---|---|
| `{"type": "object", "properties": {…}, "x-order": [keys]}` | A group of fields, in `x-order` (else in key order) |
| `{"type": "string"}` | Text box |
| `…, "x-secret": true` | Password box |
| `…, "format": "uri"` | Text box that checks for a URL |
| `…, "x-multiline": true` | Text area |
| `{"type": "integer"}`, `{"type": "number"}` with `minimum`, `maximum` | Number box |
| `{"type": "boolean"}` | Switch |
| `{"type": "string", "enum": [...], "x-enum-labels": [...]}` | Menu |
| `{"type": "array", "items": {"type": "string"}}` (or number) | List |
| `{"type": "array", "items": {"type": "object", …}}` | A list of groups, added and removed one by one |
| `{"type": "string", "x-system": true}` | Menu of the recorder's systems (short names); follows renames |

On any field: `title` (the label; the key if there's none), `description`
(help text), `default`.

On an object: `required`, the keys that have to be filled in. Until they are,
the recorder shows the plugin (for `config`) or that system (for
`system_config`) as not set up. `"x-required": true` on a field means the same.
