//! unit-script — a Trunk Recorder Pro plugin that runs a Trunk Recorder
//! style `unitScript` for radio activity.
//!
//! For each event it runs `<script> <shortName> <radioID> <action> <talkgroup>`
//! with the actions Trunk Recorder used: `on`, `off`, `join`, `call`,
//! `location`, `ackresp`, `data`, `ans_req` (plus `call_alert` and
//! `emergency`). Scripts run one at a time on a thread of their own, so a slow
//! script never holds up the recorder.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use trunk_recorder_plugin::{CallInfo, Host, Manifest, NoConfig, Plugin, Setup, State, UnitEvent, topic};

/// How many runs may wait before events are dropped.
const QUEUE: usize = 5000;

/// An action passed to the script, as Trunk Recorder named it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Action {
    /// Registered (on the system)
    On,
    /// Deregistered (off the system)
    Off,
    /// Affiliated with a talkgroup
    Join,
    /// Heard on a call
    Call,
    /// Reported its location
    Location,
    /// Acknowledged
    Ackresp,
    /// Got a data grant
    Data,
    /// Sent an answer request
    AnsReq,
    /// Sent a call alert
    CallAlert,
    /// Sent an emergency alarm
    Emergency,
}

impl Action {
    fn as_str(self) -> &'static str {
        match self {
            Action::On => "on",
            Action::Off => "off",
            Action::Join => "join",
            Action::Call => "call",
            Action::Location => "location",
            Action::Ackresp => "ackresp",
            Action::Data => "data",
            Action::AnsReq => "ans_req",
            Action::CallAlert => "call_alert",
            Action::Emergency => "emergency",
        }
    }
    /// The action for a unit event's `kind`.
    fn of_kind(kind: &str) -> Option<Action> {
        Some(match kind {
            "registration" => Action::On,
            "deregistration" => Action::Off,
            "affiliation" => Action::Join,
            "location" => Action::Location,
            "acknowledge" => Action::Ackresp,
            "data_grant" => Action::Data,
            "answer_request" => Action::AnsReq,
            "call_alert" => Action::CallAlert,
            "emergency_alarm" => Action::Emergency,
            _ => return None,
        })
    }
}

/// The plugin's settings. A system can override any of them for itself.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
struct Config {
    /// Script
    ///
    /// The unitScript to run, as `<script> <shortName> <radioID> <action> <talkgroup>`.
    /// It must be executable.
    #[schemars(extend("x-required" = true))]
    script: String,
    /// Working folder
    ///
    /// Where the script runs (it finds relative files there). Empty: the script's own folder.
    workdir: String,
    /// Actions
    ///
    /// Which actions to pass to the script.
    actions: Vec<Action>,
    /// Timeout, s
    ///
    /// A run of the script that takes longer than this is stopped.
    #[schemars(range(min = 1, max = 300))]
    timeout_s: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            script: String::new(),
            workdir: String::new(),
            actions: vec![Action::On, Action::Off, Action::Join, Action::Call, Action::Location],
            timeout_s: 20,
        }
    }
}

/// What to run for one system.
#[derive(Clone, Debug)]
struct Target {
    script: PathBuf,
    workdir: PathBuf,
    actions: HashSet<Action>,
    timeout: Duration,
}

impl Target {
    fn of(short_name: &str, c: &Config) -> Result<Option<Target>, String> {
        if c.script.trim().is_empty() {
            return Ok(None);
        }
        let script = PathBuf::from(c.script.trim());
        if !is_executable(&script) {
            return Err(format!("{short_name}: the script {} isn't there or isn't executable", script.display()));
        }
        let workdir = if c.workdir.trim().is_empty() {
            script.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/"))
        } else {
            PathBuf::from(c.workdir.trim())
        };
        if !workdir.is_dir() {
            return Err(format!("{short_name}: the working folder {} isn't there", workdir.display()));
        }
        let actions: HashSet<Action> = c.actions.iter().copied().collect();
        Ok(Some(Target { script, workdir, actions, timeout: Duration::from_secs(c.timeout_s.clamp(1, 300)) }))
    }
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// One run of the script.
struct Job {
    target: Target,
    args: [String; 4],
}

struct UnitScript {
    host: Host,
    /// By system short name: what to run (systems without a script aren't here).
    targets: HashMap<String, Target>,
    /// Radios already passed as `call` for each call in progress.
    calls: HashMap<u32, HashSet<u32>>,
    jobs: Option<SyncSender<Job>>,
    worker: Option<JoinHandle<()>>,
    dropped: u64,
}

impl UnitScript {
    fn queue(&mut self, short_name: &str, unit: u32, action: Action, talkgroup: Option<u32>) {
        let Some(target) = self.targets.get(short_name) else { return };
        if !target.actions.contains(&action) {
            return;
        }
        let job = Job {
            target: target.clone(),
            args: [short_name.to_string(), unit.to_string(), action.as_str().to_string(), talkgroup.map_or(String::new(), |t| t.to_string())],
        };
        let Some(tx) = &self.jobs else { return };
        match tx.try_send(job) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped += 1;
                if self.dropped % 100 == 1 {
                    self.host.warn(format!("the script can't keep up: {} event(s) dropped", self.dropped));
                    self.host.status(State::Warning, format!("{} event(s) dropped: the script is too slow", self.dropped));
                }
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    /// The radios of a call not yet passed to the script as `call`.
    fn call_units(&mut self, call: &CallInfo, ended: bool) {
        let units: Vec<u32> = if call.units.is_empty() { call.sources.iter().map(|s| s.unit).collect() } else { call.units.clone() };
        let seen = self.calls.entry(call.id).or_default();
        let new: Vec<u32> = units.into_iter().filter(|&u| u != 0 && seen.insert(u)).collect();
        if ended {
            self.calls.remove(&call.id);
        }
        for unit in new {
            self.queue(&call.short_name, unit, Action::Call, Some(call.talkgroup));
        }
    }
}

/// Runs the queued jobs in order until the channel closes.
fn work(host: Host, rx: Receiver<Job>) {
    for job in rx {
        let what = job.args.join(" ");
        match run(&job) {
            Ok(None) => {}
            Ok(Some(problem)) => host.warn(format!("{what}: {problem}")),
            Err(e) => host.error(format!("can't run {}: {e}", job.target.script.display())),
        }
    }
}

/// Run one job: None when it exited 0, else what went wrong.
fn run(job: &Job) -> std::io::Result<Option<String>> {
    let mut child = Command::new(&job.target.script)
        .args(&job.args)
        .current_dir(&job.target.workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    // Read stderr meanwhile, so a chatty script can't fill the pipe and stall.
    let mut err = child.stderr.take().expect("piped stderr");
    let reader = thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + job.target.timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let stderr = reader.join().unwrap_or_default();
    let last = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
    Ok(match status {
        None => Some(format!("timed out after {} s", job.target.timeout.as_secs())),
        Some(s) if s.success() => None,
        Some(s) => Some(format!("{s}{}", if last.is_empty() { String::new() } else { format!(": {last}") })),
    })
}

impl Plugin for UnitScript {
    type Config = Config;
    type SystemConfig = NoConfig;

    fn manifest() -> Manifest {
        Manifest {
            name: "Unit script".into(),
            subscribe: vec![topic::UNIT.into(), topic::CALL_START.into(), topic::CALL_END.into()],
            ..trunk_recorder_plugin::manifest!()
        }
    }

    fn start(host: Host, setup: Setup<Config, NoConfig>) -> Result<Self, String> {
        let mut targets = HashMap::new();
        for s in &setup.systems {
            if let Some(t) = Target::of(&s.short_name, &s.settings)? {
                targets.insert(s.short_name.clone(), t);
            }
        }
        if targets.is_empty() {
            return Err("Set the script to run (Script).".into());
        }
        for (name, t) in &targets {
            let mut actions: Vec<&str> = t.actions.iter().map(|a| a.as_str()).collect();
            actions.sort_unstable();
            host.info(format!("{name}: {} in {} for {}", t.script.display(), t.workdir.display(), actions.join(", ")));
        }
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        let worker_host = host.clone();
        let worker = thread::spawn(move || work(worker_host, rx));
        host.status(State::Ok, format!("running the script for {} system(s)", targets.len()));
        Ok(UnitScript { host, targets, calls: HashMap::new(), jobs: Some(tx), worker: Some(worker), dropped: 0 })
    }

    fn unit(&mut self, e: UnitEvent) {
        if let Some(action) = Action::of_kind(&e.kind) {
            self.queue(&e.short_name, e.unit, action, e.talkgroup);
        }
    }

    fn call_start(&mut self, call: CallInfo) {
        self.call_units(&call, false);
    }

    fn call_end(&mut self, call: CallInfo) {
        self.call_units(&call, true);
    }

    fn shutdown(&mut self, grace: Duration) {
        // Close the queue and let what's in it run, within the grace period.
        self.jobs = None;
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + grace.saturating_sub(Duration::from_secs(1));
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn main() {
    trunk_recorder_plugin::run::<UnitScript>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use trunk_recorder_plugin::{CallSource, HostMessage, testing};

    /// A script that appends its working folder and arguments to `calls.log`.
    fn script(dir: &Path) -> PathBuf {
        let p = dir.join("unit.sh");
        std::fs::write(&p, "#!/bin/sh\necho \"$(pwd) | $*\" >> calls.log\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    fn unit(kind: &str, unit: u32, tg: Option<u32>) -> HostMessage {
        HostMessage::Unit(UnitEvent { short_name: "sys1".into(), kind: kind.into(), unit, talkgroup: tg, ..Default::default() })
    }

    fn call(id: u32, tg: u32, units: &[u32]) -> CallInfo {
        CallInfo { id, short_name: "sys1".into(), talkgroup: tg, units: units.to_vec(), ..Default::default() }
    }

    fn log(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default().lines().map(|l| l.split(" | ").nth(1).unwrap().to_string()).collect()
    }

    #[cfg(unix)]
    #[test]
    fn runs_the_script_as_trunk_recorder_did() {
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let hello = testing::hello(&dir, json!({ "script": s }));
        let out = testing::run::<UnitScript>([
            HostMessage::Hello(hello),
            unit("affiliation", 1221002, Some(21401)),
            unit("registration", 1221002, None),
            unit("acknowledge", 5, None),
            HostMessage::CallStart(call(7, 65189, &[1221002])),
            HostMessage::CallEnd(call(7, 65189, &[1221002, 1200012])),
        ]);
        assert!(out.ready());
        assert_eq!(
            log(&dir),
            ["sys1 1221002 join 21401", "sys1 1221002 on ", "sys1 1221002 call 65189", "sys1 1200012 call 65189"],
            "ackresp is off by default; each radio once per call"
        );
        let cwd = std::fs::read_to_string(dir.join("calls.log")).unwrap();
        assert!(cwd.starts_with(dir.to_str().unwrap()), "runs in the script's folder: {cwd}");
    }

    #[cfg(unix)]
    #[test]
    fn sources_count_when_units_are_empty() {
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let mut c = call(9, 100, &[]);
        c.sources = vec![CallSource { unit: 42, ..Default::default() }];
        let out = testing::run::<UnitScript>([HostMessage::Hello(testing::hello(&dir, json!({ "script": s }))), HostMessage::CallStart(c)]);
        assert!(out.ready());
        assert_eq!(log(&dir), ["sys1 42 call 100"]);
    }

    #[cfg(unix)]
    #[test]
    fn actions_choose_what_runs() {
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let hello = testing::hello(&dir, json!({ "script": s, "actions": ["ackresp", "emergency"] }));
        let out = testing::run::<UnitScript>([
            HostMessage::Hello(hello),
            unit("affiliation", 1, Some(2)),
            unit("acknowledge", 3, None),
            unit("emergency_alarm", 4, Some(5)),
        ]);
        assert!(out.ready());
        assert_eq!(log(&dir), ["sys1 3 ackresp ", "sys1 4 emergency 5"]);
    }

    #[test]
    fn no_script_is_a_settings_error() {
        let dir = testing::temp_dir("unit-script");
        let out = testing::run::<UnitScript>([HostMessage::Hello(testing::hello(&dir, Value::Null))]);
        assert!(!out.ready());
        assert_eq!(out.exit_code, trunk_recorder_plugin::EXIT_CONFIG);
    }

    #[test]
    fn a_missing_script_is_a_settings_error() {
        let dir = testing::temp_dir("unit-script");
        let out = testing::run::<UnitScript>([HostMessage::Hello(testing::hello(&dir, json!({ "script": "/nonexistent/unit.sh" })))]);
        assert!(!out.ready());
        assert!(out.status().is_some_and(|(_, m)| m.contains("isn't there")));
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_script_is_logged() {
        let dir = testing::temp_dir("unit-script");
        let p = dir.join("bad.sh");
        std::fs::write(&p, "#!/bin/sh\necho 'no database' >&2\nexit 3\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = testing::run::<UnitScript>([HostMessage::Hello(testing::hello(&dir, json!({ "script": p }))), unit("registration", 1, None)]);
        assert!(out.logs().iter().any(|(_, m)| m.contains("no database")), "{:?}", out.logs());
    }
}
