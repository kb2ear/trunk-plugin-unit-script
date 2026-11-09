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

A system can override any of them for itself (its card's *settings for this system*), to run
another script for it. A system with no script isn't handled.

## How it runs

- Runs happen one at a time, in order, on a thread of their own: a slow script never holds up the
  recorder. Up to 5000 runs can wait; past that, events are dropped and the plugin's card warns.
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
