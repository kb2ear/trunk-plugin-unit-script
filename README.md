# Unit script

A [Trunk Recorder Pro](https://github.com/TrunkRecorder/trunk-recorder-pro) plugin that runs a
Trunk Recorder–style `unitScript` for radio activity, so scripts written for Trunk Recorder's
`unitScript` work unchanged.

For each event it runs:

    <script> <shortName> <radioID> <action> <talkgroup>

| Radio activity | `action` | `talkgroup` |
|---|---|---|
| registered / deregistered | `on` / `off` | empty |
| affiliated | `join` | the talkgroup |
| reported its location | `location` | the talkgroup |
| heard on a call (once per radio per call) | `call` | the call's talkgroup |
| acknowledged, data grant, answer request | `ackresp`, `data`, `ans_req` | as sent |
| call alert, emergency alarm | `call_alert`, `emergency` | as sent |

Only the actions ticked under **Actions** are passed on; by default `on`, `off`, `join`, `call`
and `location`.

## Settings

| Setting | Default | |
|---|---|---|
| **Script** (`script`) | (required) | The executable to run |
| **Working folder** (`workdir`) | the script's folder | Where it runs: relative paths in the script are found there |
| **Actions** (`actions`) | `on, off, join, call, location` | Which actions to pass |
| **Timeout, s** (`timeoutS`) | 20 | A run taking longer is stopped |
| **Systems** (`systems`) | none ticked | Which systems run the script. None ticked: every trunked system |
| **Include conventional systems** (`includeConventional`) | off | With no systems ticked, conventional systems run it too |

As with Trunk Recorder's per-system `unitScript`, **conventional systems are left out** unless
you tick them or turn on **Include conventional systems**. They have no registrations or
affiliations, so for them the script only gets `call`, for each radio heard (P25 / DMR IDs, MDC,
FleetSync). The plugin's log names the systems left out at start.

A system can override Script, Working folder, Actions and Timeout for itself (its card's
*settings for this system*), to run another script for it.

## How it runs

- Runs happen one at a time, in order, on a thread of their own: a slow script never holds up the
  recorder. Up to 5000 runs can wait; past that, events are dropped and the plugin's card warns.
- The plugin's card shows runs, failures, timeouts, drops, runs waiting, the run time and the
  last success and error (every few seconds while it works). Its Sent / Failed figures, which
  count recorded calls, stay at 0: this plugin doesn't take recorded calls.
- A run that exits non-zero is logged with the last line it wrote to stderr; one that times out is
  stopped and logged.
- A radio that keys up partway through a call is passed as `call` when the call ends.

## Differences from Trunk Recorder

- `call_alert` and `emergency` are new (Trunk Recorder has no such actions).
- `on` and `off` pass an empty talkgroup.

## Building

    cargo build --release
    ./target/release/unit-script --describe
    trunk-pro plugin run ./target/release/unit-script <calls folder> --settings examples/settings.json

## License

MIT
