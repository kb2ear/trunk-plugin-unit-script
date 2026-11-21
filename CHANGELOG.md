# Changelog

## [0.1.3]

- **The card reports on every recorded call, like other plugins**: Sent, Skipped, Last result,
  Last success, age at delivery and the per-minute bars now move. The plugin takes recorded calls
  (`call.concluded`) for this and reports on each as it is recorded: Sent when the system runs the
  script for `call`, Skipped when it doesn't. It never holds a call's files back.
- The plugin's own figures are named for what they count: **script runs**, **script failures**,
  **timed out**, **dropped** (script failures are no longer called "failed", which the card now
  uses for calls).

## [0.1.2]

- **A system can set Script, Working folder, Actions and Timeout again for itself** on
  Trunk Recorder Pro 0.4.0 and later. Since plugin SDK 0.13.0 a system's card offers only the
  settings its plugin names (`system_overrides`), and overrides of others are ignored, so without
  this the per-system script stopped working. **Systems** and **Include conventional systems**
  stay whole-recorder settings.
- Built on plugin SDK 0.14.0 (was 0.10.0).
- Tests: a system's own Actions are used, and every setting offered to a system exists.

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
