# Settings

A plugin describes its settings as Rust structs. The recorder draws a form from
them, stores what the user enters, and hands it back to the plugin at startup.
You never write form code.

There are two kinds of settings:

- **`Config`**: settings for the plugin as a whole, like a server URL.
- **`SystemConfig`**: settings the form repeats for each system being
  recorded, like an API key per system. Many services give each system its own
  key, so uploaders need this. A system's sites share them; for a service that
  knows a site as a system of its own, add a list of sites with what differs
  (the [Broadcastify](https://github.com/TrunkRecorder/trunk-plugin-broadcastify)
  and [OpenMHz](https://github.com/TrunkRecorder/trunk-plugin-openmhz) plugins'
  **Sites of their own**) and pick by the call's `site`.

Use `NoConfig` for either if you don't need it.

## A settings struct

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
struct Config {
    /// Server
    ///
    /// Where calls are uploaded. Leave it as it is unless you run your own.
    #[schemars(url)]
    server: String,
    /// Upload encrypted calls
    upload_encrypted: bool,
    /// Retries
    ///
    /// How many times to try a call before giving up.
    #[schemars(range(min = 0, max = 10))]
    retries: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config { server: "https://api.example.com".into(), upload_encrypted: false, retries: 3 }
    }
}
```

The attributes do the following:

- **`Deserialize`** reads what the user entered.
- **`JsonSchema`** describes the struct to the form. It comes from the
  [`schemars`](https://docs.rs/schemars/1) crate, version 1.
- **`Serialize`** lets the schema include your defaults, so the form starts
  filled in with them. Without it, fields start empty.
- **`#[serde(default)]`** lets the user leave any field empty. The field then
  takes its value from `Default`. Leave it out and every field becomes
  required, and an existing install breaks the day you add a field.
- **`#[serde(rename_all = "camelCase")]`** isn't required, but it's the
  recorder's convention for its own settings.

Doc comments become the form's text. The first paragraph is the field's
**label**; the rest is **help** shown under it. Wrap them however you like: line
breaks inside a paragraph are joined up. Keep labels short, and write help
for someone who isn't a programmer. The doc comment on the struct itself is
for programmers, and isn't shown.

## What the form can show

| Rust | Form |
|---|---|
| `String` | Text box |
| `String` with `#[schemars(extend("x-secret" = true))]` | Password box (hidden as it's typed, and never logged) |
| `String` with `#[schemars(url)]` | Text box that checks for a URL |
| `String` with `#[schemars(extend("x-multiline" = true))]` | Text area |
| `bool` | Switch |
| `u32`, `i64`, `f64`, … with `#[schemars(range(min = …, max = …))]` | Number box |
| An enum of unit variants | Menu. A variant's doc comment is its label. |
| `Vec<String>`, `Vec<u32>`, … | List |
| `Option<T>` | Same as `T`. Empty means `None`. |
| A struct | A group of its fields |
| `Vec` of a struct | A list of groups, with Add and Remove buttons. The struct's doc comment names each one ("Stream 1", "Add stream"). |
| `String` with `#[schemars(extend("x-system" = true))]` | Menu of the recorder's systems, by short name. When a system is renamed, the recorder changes the setting to match. |

Fields appear in the order you declare them.

### Fields that have to be filled in

Mark a field the plugin can't do without with `x-required`:

```rust
/// API key
#[schemars(extend("x-secret" = true, "x-required" = true))]
api_key: String,
```

Until it's filled in, the recorder says so. A required field in `Config`
marks the plugin **Needs setting up**. A required field in `SystemConfig`
marks that system **Not set up** for your plugin, both on the system's card
and in the row of systems on your plugin's card. Use it for what makes a
system count as set up, such as an upload key, even when an empty key just
means "don't upload this system".

`#[schemars(required)]` does nothing on a `#[serde(default)]` struct, which
is why it's marked this way. The SDK (0.1.1 and later) turns these marks into
the schema's standard `required` list. Marking a field doesn't stop the user
from saving without it, so still check in `start`.

The form doesn't handle maps or enums that carry data. Keep
settings flat, and name things users know, like "API key" rather than
"auth token".

For a menu:

```rust
#[derive(Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
enum Quality {
    /// Low (smaller files)
    Low,
    #[default]
    /// Normal
    Normal,
}
```

## Settings for each system

Settings like an upload key differ from system to system. Put those in
`SystemConfig`; the recorder shows them on each system's card in Setup.

```rust
#[derive(Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", default)]
struct SystemConfig {
    /// API key
    #[schemars(extend("x-secret" = true))]
    api_key: String,
}
```

In `start`, `setup.systems` lists every system, each with its short name,
kind (`p25`, `smartnet`, `dmr` or `conventional`), and your `SystemConfig`.
The config is `None` when the user left that system's settings empty.

A system's **short name** is its identity: no two of the recorder's systems
share one, and every event carries it (`call.short_name`; a concluded call's
`call.call.short_name`). Build your lookup in `start` by short name:

```rust
let keys: HashMap<String, String> = setup
    .systems
    .iter()
    .filter_map(|s| Some((s.short_name.clone(), s.config.as_ref()?.api_key.clone())))
    .filter(|(_, k)| !k.is_empty())
    .collect();
```

Each system also has an `index`, the number events carry as `system`. It is
only good for this run: the same system can have another number next time.
So don't key anything you save on it, such as calls queued for a later run.
`setup.system_named(name)` finds a system by short name.

Each conventional system is a system of its own, with its own short name and
settings. Systems are numbered from 0, trunked first, then conventional; tell
a conventional one by its `kind` (`"conventional"`), not by its number. (Before
Trunk Recorder Pro v0.1.16, conventional systems were numbered down from 65535
and the SDK had a `CONVENTIONAL` constant; SDK 0.5.0 drops it.)

## Settings a system overrides

A user can change any of your plugin's own settings (`Config`) for one
system, for example to send its calls to another server. Each system's
`settings` is your `Config` with that system's changes applied. Read your
settings from there for anything a system's calls use, and from
`setup.config` only for what belongs to the whole plugin, like a log file:

```rust
for s in &setup.systems {
    let server = s.settings.server.trim();
    // …
}
```

When systems can name different servers, keep a separate queue for each so
one that's down doesn't hold up the others: start the `CallQueue` with
`CallQueue::start_by`, giving it a closure that names each call's server
(its lane). Each lane gets its own workers, retries and endpoint status.

## Checking settings

The SDK rejects settings that don't parse, such as text where a number goes,
before `start` runs. Check everything else in `start`, and return an `Err`
that tells the user what to fix:

```rust
if keys.is_empty() {
    return Err("Add an API key for at least one system".into());
}
```

The recorder shows the error, and doesn't start the plugin again until
recording next starts.

## Seeing the form

Plugins are set up in the recorder's **Setup**, like the rest of it. Add your
build on the **Plugins** page (see
[Getting started](getting-started.md#6-run-it-inside-your-recorder)) and
press **Set up** to see your forms the way users will:

- `Config` is on your plugin's card in Setup's **Plugins** tab, under its
  on/off switch, with a row of the systems showing which are set up.
- `SystemConfig` is on each system's card under **Systems** (and under
  **Conventional**), once your plugin is on.

Fields come in declaration order, with labels and help from your doc
comments, defaults as placeholder text and secrets hidden behind **Show**.
Everything saves as it's typed. Empty fields aren't saved, so your defaults
apply.

When `SystemConfig` has a field with the same name as one in `Config`, the
system's field shows the `Config` value as its placeholder: a setting for
every system that each can override. Your plugin decides what an empty
system field means; the upload-script plugin uses its main script.

## Where settings live

In the recorder's `config.json`. Your plugin's switch and `Config` are under
`plugins`; its `SystemConfig` for a system is inside that system:

```json
{
  "plugins": {
    "my-plugin": { "enabled": true, "settings": { "server": "https://api.example.com", "retries": 3 } }
  },
  "systems": [
    { "shortName": "county", …, "plugins": { "my-plugin": { "apiKey": "…" } } }
  ]
}
```

A system's settings go with it when it's renamed, and when it's removed.
Uninstalling a plugin removes its settings everywhere.

## Changing your settings later

Users upgrade plugins but keep their settings, so:

- **Adding a field** is safe, as long as the struct has `#[serde(default)]`.
- **Renaming a field** loses what users entered. Use `#[serde(alias = "old")]`
  to keep reading the old name. Aliases also let people coming from Trunk
  Recorder paste their old plugin settings: the OpenMHz plugin reads
  `uploadServer` and `openmhzSystemId`, TR's names, as well as its own.
- **Removing a field** is safe, since unknown fields are ignored.
- **Changing a field's type** breaks existing settings. Add a new field instead.

## Seeing the schema

`./target/debug/my-plugin --describe` prints the manifest, which includes the
schemas the form is drawn from. They use a small subset of
[JSON Schema](https://json-schema.org). [Protocol](protocol.md#settings-schemas)
lists it, for plugins that write their schemas by hand.
