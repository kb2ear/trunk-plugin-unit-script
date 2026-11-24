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

`shortName` is the system's short name, except on a system with several sites, where it is the
name of the site the event came from (in Trunk Recorder each site was a system of its own). A
system of one site names that site by its short name, and a conventional system has no sites, so
both get the short name. The script also gets these in its environment:

| Variable | |
|---|---|
| `TR_SHORT_NAME` | The system's short name, whatever `shortName` is |
| `TR_SYSTEM_ID` | The system's id: the same across renames |
| `TR_SITE` | The site's name; empty for a conventional system |
| `TR_SITE_ID` | The site's id; empty for a conventional system |

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

### For a system, and each site

A system can have its own **Script**, **Working folder**, **Actions** and **Timeout** (on its
card under **Plugins** > **Unit script**); empty ones are the plugin's. A system with several
sites has a row for each site there, where a site can set those four again; empty ones are the
system's. So each site can run a script of its own, as each site's system could in Trunk
Recorder. A working folder left empty everywhere is the script's own folder.

Events go by the site they came from: a unit event by the site whose control channel told it, a
call's radios by the site that had the call first, and a recorded call (for the card) by the site
its copy was kept from. **Systems** and **Include conventional systems** are for the whole
recorder. The recorder's own switches on the plugin's card (Setup > Plugins) turn it off for a
system or a site; a call from a site it's off for runs nothing. Sites need Trunk Recorder Pro
0.4.0 or later (plugin SDK 0.14.0+).

## How it runs

- Runs happen one at a time, in order, on a thread of their own: a slow script never holds up the
  recorder. Up to 5000 runs can wait; past that, events are dropped and the plugin's card warns.
- The plugin's card counts recorded calls as other plugins' do: **Sent** for a call from a system
  or site that runs the script for `call`, **Skipped** for one that doesn't (a conventional system left
  out, or `call` not ticked under **Actions**), with **Last success** and the age at delivery to
  match. The plugin reports on each call as soon as it is recorded, so it never holds a call's
  files back from the recorder's **Archive** settings. Failed scripts aren't failed calls: they
  are counted on the plugin's own line (script runs, script failures, timed out, dropped, runs
  waiting, the run time and the last error), updated every few seconds while it works.
- A run that exits non-zero is logged with the last line it wrote to stderr; one that times out is
  stopped and logged.
- A radio that keys up partway through a call is passed as `call` when the call ends.

## Differences from Trunk Recorder

- `call_alert` and `emergency` are new (Trunk Recorder has no such actions).
- The `TR_*` environment variables are new.
- `on` and `off` pass an empty talkgroup.

## Building

    cargo build --release
    ./target/release/unit-script --describe
    trunk-pro plugin run ./target/release/unit-script <calls folder> --settings examples/settings.json

## License

MIT
