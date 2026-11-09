//! call-log — a Trunk Recorder Pro plugin that writes a line of JSON for
//! every recorded call.
//!
//! It's the template's example: replace it with your plugin. It shows the
//! parts every plugin has — settings (for the plugin and for each system), a
//! manifest, startup, handling an event, reporting back, and shutting down.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use trunk_recorder_plugin::{ConcludedCall, Host, Manifest, Outcome, Plugin, Setup, topic};

/// The plugin's settings. The recorder draws a form from this (doc comments
/// become the labels and help text) and passes what the user entered.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
struct Config {
    /// Log file
    ///
    /// Where the lines go. A relative path is in the plugin's data folder.
    file: String,
    /// Log encrypted calls
    include_encrypted: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { file: "calls.jsonl".into(), include_encrypted: false }
    }
}

/// Settings for each system (repeated under each in the form).
#[derive(Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", default)]
struct SystemConfig {
    /// Leave this system out
    skip: bool,
    /// Label
    ///
    /// Written with its calls instead of the system's short name.
    label: String,
}

struct CallLog {
    host: Host,
    out: BufWriter<File>,
    /// Per system, by short name: the label (None to skip it), and whether
    /// its encrypted calls are logged.
    labels: Vec<(String, Option<String>, bool)>,
}

impl Plugin for CallLog {
    type Config = Config;
    type SystemConfig = SystemConfig;

    fn manifest() -> Manifest {
        Manifest {
            name: "Call log".into(),
            subscribe: vec![topic::CALL_CONCLUDED.into()],
            // The id, version, description, … come from Cargo.toml.
            ..trunk_recorder_plugin::manifest!()
        }
    }

    fn start(host: Host, setup: Setup<Config, SystemConfig>) -> Result<Self, String> {
        // There's one log file: the plugin's own setting (`setup.config`).
        let mut path = PathBuf::from(&setup.config.file);
        if path.is_relative() {
            path = setup.data_dir.join(path);
        }
        // An Err here is shown to the user as a settings problem.
        let file = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("can't open {}: {e}", path.display()))?;
        let labels = setup
            .systems
            .iter()
            .map(|s| {
                let c = s.config.as_ref();
                let label = match c {
                    Some(c) if c.skip => None,
                    Some(c) if !c.label.is_empty() => Some(c.label.clone()),
                    _ => Some(s.short_name.clone()),
                };
                // A system can override the plugin's settings: read them from `s.settings`.
                (s.short_name.clone(), label, s.settings.include_encrypted)
            })
            .collect();
        host.info(format!("logging calls to {}", path.display()));
        Ok(CallLog { host, out: BufWriter::new(file), labels })
    }

    fn call_concluded(&mut self, call: ConcludedCall) {
        // Systems are known by short name: every event carries it.
        let system = self.labels.iter().find(|(n, ..)| *n == call.call.short_name);
        let Some((_, Some(label), include_encrypted)) = system.cloned() else {
            self.host.call_result(&call.path, Outcome::Skipped, "system not logged", "");
            return;
        };
        if call.call.encrypted && !include_encrypted {
            self.host.call_result(&call.path, Outcome::Skipped, "encrypted", "");
            return;
        }
        let c = &call.call;
        let line = json!({
            "system": label,
            "talkgroup": c.talkgroup,
            "tag": c.talkgroup_tag,
            "start": c.start_time,
            "seconds": c.call_length,
            "units": c.src_list.iter().map(|s| s.src).collect::<Vec<_>>(),
            "wav": call.files.wav,
        });
        let r = writeln!(self.out, "{line}").and_then(|_| self.out.flush());
        match r {
            Ok(()) => self.host.call_result(&call.path, Outcome::Ok, "", ""),
            Err(e) => self.host.call_result(&call.path, Outcome::Failed, e.to_string(), ""),
        }
    }

    fn shutdown(&mut self, _grace: std::time::Duration) {
        let _ = self.out.flush();
    }
}

fn main() {
    trunk_recorder_plugin::run::<CallLog>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use trunk_recorder_plugin::{HostMessage, testing};

    #[test]
    fn logs_a_call() {
        let dir = testing::temp_dir("call-log");
        let mut hello = testing::hello(&dir, json!({ "file": "out.jsonl" }));
        hello.systems[0].config = json!({ "label": "County" });
        let call = testing::call(&dir, "sys1", 101);
        let out = testing::run::<CallLog>([HostMessage::Hello(hello), HostMessage::CallConcluded(call.clone())]);
        assert!(out.ready());
        assert_eq!(out.results(), vec![(call.path, Outcome::Ok, String::new(), String::new())]);
        let line: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("data/out.jsonl")).unwrap()).unwrap();
        assert_eq!(line["system"], "County");
        assert_eq!(line["talkgroup"], 101);
    }

    #[test]
    fn skips_a_system_left_out() {
        let dir = testing::temp_dir("call-log");
        let mut hello = testing::hello(&dir, Value::Null);
        hello.systems[0].config = json!({ "skip": true });
        let out = testing::run::<CallLog>([HostMessage::Hello(hello), HostMessage::CallConcluded(testing::call(&dir, "sys1", 7))]);
        assert_eq!(out.results()[0].1, Outcome::Skipped);
    }

    #[test]
    fn bad_settings_stop_it() {
        let dir = testing::temp_dir("call-log");
        let hello = testing::hello(&dir, json!({ "file": 42 }));
        let out = testing::run::<CallLog>([HostMessage::Hello(hello)]);
        assert!(!out.ready());
        assert_eq!(out.exit_code, trunk_recorder_plugin::EXIT_CONFIG);
    }
}
