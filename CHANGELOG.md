# Changelog

## [0.1.1]

- **Conventional systems are left out by default**, as Trunk Recorder's per-system `unitScript`
  was: **Include conventional systems** runs the script for them too (`call` only).
- **Systems**: tick which systems run the script (none ticked: every trunked system).
- **The plugin's card has figures**: runs, failures, timeouts, drops, runs waiting, run time, and
  the last success and error.

## [0.1.0]

- First release: runs a Trunk Recorder-style `unitScript`
  (`<script> <shortName> <radioID> <action> <talkgroup>`) for registrations, affiliations,
  locations, other unit events and the radios heard on calls.
