# Events

A plugin gets only the topics listed in its manifest's `subscribe`. The
recorder doesn't build events nobody subscribed to, so subscribing to a topic
you don't use costs everyone something.

| Topic | Method | When | Rate |
|---|---|---|---|
| `call.concluded` | `call_concluded` | A recorded call's files are on disk | One per call |
| `call.start` | `call_start` | A call starts (recorded or not) | One per call |
| `call.end` | `call_end` | A call ends | One per call |
| `unit` | `unit` | A radio registers, affiliates, … | Busy systems: many per second |
| `audio` | `audio` | Live audio of recording calls | About 50 per second per call |
| `status` | `status` | Every system's state | Every 5 seconds |

The examples below are the JSON on the wire. The Rust SDK hands you the same
data as structs; the field names match. Times are Unix seconds. Frequencies
are in hertz.

## `call.concluded`

A recorded call is finished and its files are written. This is the event for
uploaders and archivers.

```json
{
  "type": "call.concluded",
  "path": "county/2026/9/30/101-1790771550_857587500",
  "system": 0,
  "call": {
    "call_num": 412, "short_name": "county", "site": "county", "talkgroup": 101,
    "talkgroup_tag": "Fire Dispatch", "talkgroup_description": "", "talkgroup_group": "Fire", "talkgroup_group_tag": "Fire Dispatch",
    "freq": 857587500, "start_time": 1790771550, "stop_time": 1790771554, "call_length": 4,
    "emergency": 0, "encrypted": 0, "priority": 0, "phase2_tdma": 0, "tdma_slot": 0, "audio_type": "digital",
    "freqList": [{ "freq": 857587500, "time": 1790771550, "pos": 0, "len": 4, "error_count": 0, "spike_count": 0 }],
    "srcList": [{ "src": 1116707, "time": 1790771550, "pos": 0, "emergency": 0, "signal_system": "", "tag": "", "tag_ota": "E14" }]
  },
  "files": {
    "json": "/home/me/TrunkRecorderPro/county/2026/9/30/101-1790771550_857587500.json",
    "wav": "/home/me/TrunkRecorderPro/county/2026/9/30/101-1790771550_857587500.wav",
    "m4a": "/home/me/TrunkRecorderPro/county/2026/9/30/101-1790771550_857587500.m4a"
  }
}
```

- **`path`** is the call's key: its location relative to the capture folder,
  without an extension. Report results against it with `host.call_result(&call.path, …)`.
- **`system`** is the system's number for this run (`SystemInfo.index`). Know
  systems by `call.short_name` instead: that is the system's identity, and it
  stays the same from run to run.
- **`call`** is the call's JSON file, in
  [Trunk Recorder's format](https://trunkrecorder.com/docs/notes/CALLFILE),
  so existing tools and services understand it. The SDK's `CallRecord` names
  the common fields; the rest are in `call.extra`. `call.error_count()` and
  `call.spike_count()` add up the per-frequency counts.
- **`call.patched_talkgroups`** is there only when the call's talkgroup was
  patched with others during the call: every talkgroup in the patch, its own
  included, in ascending order (`CallRecord::patched_talkgroups`, empty when
  absent). A patched call is usually recorded on the patch's supergroup, which
  is often not in the talkgroup list, so this is how to tell what it carried.
- **`files.m4a`** is there only if the plugin asked for M4A and the recorder
  could make it. See [Audio](audio.md).

Only recorded calls conclude. Calls the recorder skipped (encrypted, no free
recorder, an unknown talkgroup when those aren't recorded) have no files.

A system can have several sites, each a control channel the recorder follows.
A call heard on more than one is one call, saved once from the site that heard
it best: `short_name` is the system, and **`site`** is the site the call was
kept from (a system of one site: its short name; none for a conventional
channel's call).

## `call.start` and `call.end`

A call started, or ended. Unlike `call.concluded`, these cover calls that
aren't recorded too. `call.end` comes as soon as the call ends; its files, if
it was recorded, come later in `call.concluded`.

```json
{
  "type": "call.start",
  "id": 412, "system": 0, "short_name": "county", "site": "county",
  "talkgroup": 101, "talkgroup_tag": "Fire Dispatch",
  "freq_hz": 857587500, "tdma_slot": null,
  "analog": false, "encrypted": false, "emergency": false,
  "recording": true, "reason": null,
  "start_time": 1790771550.2,
  "units": [1116707],
  "patched_talkgroups": []
}
```

- **`id`** is unique while the recorder runs. It's the same number as
  `call_num` in the concluded call, except for a call heard on several sites:
  it starts and ends once, as its first site's copy (`site`), and the copy
  that concludes may be another site's, with another number. Match those by
  system, talkgroup and time if you need to.
- **`recording`**: `false` means the call isn't recorded, and **`reason`**
  says why: `encrypted`, `unknown_tg` (not in the talkgroup list, and unknown
  talkgroups aren't recorded), `no_recorder` (all in use) or `no_source` (no
  radio covers its frequency). More reasons may be added.
- **`units`**: radios heard on the call so far. At `call.end`, all of them.
- **`patched_talkgroups`**: the talkgroups patched with this one so far, its
  own included, ascending; empty when it isn't patched.

## `unit`

A radio did something on the control channel.

```json
{ "type": "unit", "system": 0, "short_name": "county", "site": "county", "kind": "affiliation", "unit": 1116707, "talkgroup": 101, "time": 1790771551.4 }
```

`kind` is one of:

| Kind | Meaning | `talkgroup` |
|---|---|---|
| `registration` | The radio joined the system | none |
| `deregistration` | It left (switched off, out of range) | none |
| `affiliation` | It selected a talkgroup | the talkgroup |
| `acknowledge` | It acknowledged the system | none |
| `location` | It reported its location | the talkgroup |
| `data_grant` | It was granted a data channel | none |
| `answer_request` | Another radio asked to call it | the talkgroup |
| `call_alert` | It was paged | the talkgroup |

P25 systems report all of these. SmartNet systems report fewer.

## `audio`

Live audio of a call being recorded, as it's decoded: 16-bit mono PCM,
little-endian, base64-encoded. `chunk.samples()` decodes it.

```json
{ "type": "audio", "call_id": 412, "system": 0, "talkgroup": 101, "sample_rate": 8000, "pcm": "AAABAP7/…" }
```

Chunks of one call come in order. Chunks of different calls are interleaved.
Use `call_id` to tell them apart, and `call.start` / `call.end` (subscribe to
them too) to know when a call's audio begins and ends. A call heard on several
sites plays from whichever site's copy has it at the moment, so a chunk can
carry the `call_id` of a copy that had no `call.start`: it is the call of the
same system and talkgroup that did.

This topic is busy. Hand the audio to your own thread straight away.

## `status`

Every system's state, every 5 seconds: an entry for each site, so a system of
several sites has several, with the same `index` and `short_name` and its own
`site_name`.

```json
{
  "type": "status", "time": 1790771560.0,
  "systems": [
    { "index": 0, "short_name": "county", "site_name": "county", "control_channel_hz": 851012500, "decode_rate": 38.5, "active_calls": 3, "recording": 2 }
  ]
}
```

- **`control_channel_hz`** is `null` while the system searches for its
  control channel.
- **`decode_rate`** is control channel messages decoded per second. A healthy
  P25 control channel gives about 40.

## Order and loss

Events of one topic arrive in the order they happened. `call.concluded` comes
after `call.end`, sometimes by a moment longer when M4A is being made. A
plugin that falls behind loses events (see
[How plugins work](how-plugins-work.md#one-thread-return-quickly)), so treat
`call.start` without a matching `call.end` as possible, and time out calls you
track.

## Topics may grow

New fields and new topics may appear in later versions of the recorder.
Ignore what you don't know; the SDK does this for you.
