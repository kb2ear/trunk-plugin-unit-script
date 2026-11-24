//! unit-script — a Trunk Recorder Pro plugin that runs a Trunk Recorder
//! style `unitScript` for radio activity.
//!
//! For each event it runs `<script> <shortName> <radioID> <action> <talkgroup>`
//! with the actions Trunk Recorder used: `on`, `off`, `join`, `call`,
//! `location`, `ackresp`, `data`, `ans_req` (plus `call_alert` and
//! `emergency`). Scripts run one at a time on a thread of their own, so a slow
//! script never holds up the recorder. Like Trunk Recorder's per-system
//! `unitScript`, conventional systems are left out unless asked for.
//!
//! A system, and each site of a system with several, can have a script of its
//! own. `<shortName>` is the site's name on a system with several sites (each
//! was a system of its own in Trunk Recorder), else the system's short name.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use trunk_recorder_plugin::{CallInfo, ConcludedCall, Host, Manifest, Metrics, Outcome, Plugin, Setup, State, UnitEvent, topic};

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

/// The plugin's settings. A system, and each site of one with several, can set Script,
/// Working folder, Actions and Timeout again for itself ([`Own`]).
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
    /// Systems
    ///
    /// Which systems run the script. None ticked: every trunked system (and the
    /// conventional ones too, with Include conventional systems).
    #[schemars(extend("x-systems" = true))]
    systems: Vec<String>,
    /// Include conventional systems
    ///
    /// With no systems ticked, run the script for conventional systems too. They
    /// have no registrations or affiliations: only `call`, for the radios heard.
    include_conventional: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            script: String::new(),
            workdir: String::new(),
            actions: vec![Action::On, Action::Off, Action::Join, Action::Call, Action::Location],
            timeout_s: 20,
            systems: Vec::new(),
            include_conventional: false,
        }
    }
}

/// Settings for a system (on its card), and for each site of a system with several
/// (a row for each on the system's card). Empty: as the plugin's; a site's: as its system's.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
struct Own {
    /// Script
    ///
    /// The unitScript to run for this system or site. Empty: the plugin's.
    script: String,
    /// Working folder
    ///
    /// Where it runs. Empty: the plugin's (with none there either, the script's own folder).
    workdir: String,
    /// Actions
    ///
    /// Which actions to pass to it. None ticked: the plugin's.
    actions: Vec<Action>,
    /// Timeout, s
    ///
    /// A run that takes longer than this is stopped. Empty: the plugin's.
    #[schemars(range(min = 1, max = 300))]
    timeout_s: Option<u64>,
}

impl Own {
    /// The plugin's settings `c` with those set here.
    fn over(&self, c: &Config) -> Config {
        let mut e = c.clone();
        if !self.script.trim().is_empty() {
            e.script = self.script.clone();
        }
        if !self.workdir.trim().is_empty() {
            e.workdir = self.workdir.clone();
        }
        if !self.actions.is_empty() {
            e.actions = self.actions.clone();
        }
        if let Some(t) = self.timeout_s {
            e.timeout_s = t;
        }
        e
    }
}

/// What to run for one system or site.
#[derive(Clone, Debug, PartialEq)]
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

/// What the worker has done, for the plugin's card.
#[derive(Default)]
struct Stats {
    runs: AtomicU64,
    failed: AtomicU64,
    timed_out: AtomicU64,
    dropped: AtomicU64,
    /// Waiting to run (queued, not yet started).
    waiting: AtomicU64,
    last: Mutex<Last>,
}

/// The latest outcomes, Unix seconds.
#[derive(Default)]
struct Last {
    ok: Option<f64>,
    error: Option<f64>,
    error_text: String,
    /// A run's time, smoothed, ms.
    run_ms: Option<f64>,
}

impl Stats {
    fn metrics(&self) -> Metrics {
        let last = self.last.lock().unwrap();
        let mut m = Metrics {
            queued: Some(self.waiting.load(Ordering::Relaxed)),
            latency_ms: last.run_ms,
            last_ok: last.ok,
            last_error: last.error,
            last_error_text: last.error_text.clone(),
            ..Default::default()
        };
        for (k, v) in [("script runs", &self.runs), ("script failures", &self.failed), ("timed out", &self.timed_out), ("dropped", &self.dropped)] {
            m.extra.insert(k.into(), v.load(Ordering::Relaxed).into());
        }
        m
    }
}

fn now_s() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// One run of the script.
struct Job {
    target: Target,
    args: [String; 4],
    env: [(&'static str, String); 4],
}

/// Where an event came from.
struct Origin<'a> {
    short_name: &'a str,
    system_id: u32,
    /// The site's name; empty for none (a conventional system).
    site: &'a str,
    site_id: u32,
}

impl Origin<'_> {
    /// `<shortName>` for the script: the site's name on a system with several sites (each was
    /// a system of its own in Trunk Recorder), else the system's short name. A system of one
    /// site names it by its short name, so the site's name serves both.
    fn name(&self) -> &str {
        if self.site.is_empty() { self.short_name } else { self.site }
    }
}

fn id_text(id: u32) -> String {
    if id == 0 { String::new() } else { id.to_string() }
}

struct UnitScript {
    host: Host,
    /// What to run, by system short name and site id (0: the system's, for its sites
    /// without their own). Systems without a script aren't here.
    targets: HashMap<(String, u32), Target>,
    /// The ids of each system's sites the plugin is on for (empty: none listed).
    sites_on: HashMap<String, HashSet<u32>>,
    /// Radios already passed as `call` for each call in progress.
    calls: HashMap<u32, HashSet<u32>>,
    jobs: Option<SyncSender<Job>>,
    worker: Option<JoinHandle<()>>,
    stats: Arc<Stats>,
}

impl UnitScript {
    /// What to run for a site of a system: the site's own, else the system's. None for a site
    /// the plugin is off for (calls as they go are the system's, and still come for it).
    fn target(&self, short_name: &str, site_id: u32) -> Option<&Target> {
        if site_id != 0 && self.sites_on.get(short_name).is_some_and(|on| !on.is_empty() && !on.contains(&site_id)) {
            return None;
        }
        self.targets.get(&(short_name.to_string(), site_id)).or_else(|| self.targets.get(&(short_name.to_string(), 0)))
    }

    fn queue(&mut self, from: &Origin, unit: u32, action: Action, talkgroup: Option<u32>) {
        let Some(target) = self.target(from.short_name, from.site_id) else { return };
        if !target.actions.contains(&action) {
            return;
        }
        let job = Job {
            target: target.clone(),
            args: [from.name().to_string(), unit.to_string(), action.as_str().to_string(), talkgroup.map_or(String::new(), |t| t.to_string())],
            env: [
                ("TR_SHORT_NAME", from.short_name.to_string()),
                ("TR_SYSTEM_ID", id_text(from.system_id)),
                ("TR_SITE", from.site.to_string()),
                ("TR_SITE_ID", id_text(from.site_id)),
            ],
        };
        let Some(tx) = &self.jobs else { return };
        match tx.try_send(job) {
            Ok(()) => {
                self.stats.waiting.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Full(_)) => {
                let dropped = self.stats.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                if dropped % 100 == 1 {
                    self.host.warn(format!("the script can't keep up: {dropped} event(s) dropped"));
                    self.host.status(State::Warning, format!("{dropped} event(s) dropped: the script is too slow"));
                    self.host.metrics(&self.stats.metrics());
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
        let from = Origin { short_name: &call.short_name, system_id: call.system_id, site: &call.site, site_id: call.site_id };
        for unit in new {
            self.queue(&from, unit, Action::Call, Some(call.talkgroup));
        }
    }
}

/// How often the card's figures are sent, at most.
const METRICS_EVERY: Duration = Duration::from_secs(5);

/// Runs the queued jobs in order until the channel closes.
fn work(host: Host, rx: Receiver<Job>, stats: Arc<Stats>) {
    let mut sent = Instant::now() - METRICS_EVERY;
    for job in rx {
        stats.waiting.fetch_sub(1, Ordering::Relaxed);
        let what = job.args.join(" ");
        let t = Instant::now();
        let outcome = run(&job);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        stats.runs.fetch_add(1, Ordering::Relaxed);
        {
            let mut last = stats.last.lock().unwrap();
            last.run_ms = Some(last.run_ms.map_or(ms, |avg| avg * 0.9 + ms * 0.1));
            match &outcome {
                Ok(None) => last.ok = Some(now_s()),
                Ok(Some(problem)) => {
                    last.error = Some(now_s());
                    last.error_text = format!("{what}: {problem}");
                }
                Err(e) => {
                    last.error = Some(now_s());
                    last.error_text = format!("can't run {}: {e}", job.target.script.display());
                }
            }
        }
        match outcome {
            Ok(None) => {}
            Ok(Some(problem)) => {
                stats.failed.fetch_add(1, Ordering::Relaxed);
                if problem.starts_with("timed out") {
                    stats.timed_out.fetch_add(1, Ordering::Relaxed);
                }
                host.warn(format!("{what}: {problem}"));
            }
            Err(e) => {
                stats.failed.fetch_add(1, Ordering::Relaxed);
                host.error(format!("can't run {}: {e}", job.target.script.display()));
            }
        }
        if sent.elapsed() >= METRICS_EVERY {
            host.metrics(&stats.metrics());
            sent = Instant::now();
        }
    }
    host.metrics(&stats.metrics());
}

/// Run one job: None when it exited 0, else what went wrong.
fn run(job: &Job) -> std::io::Result<Option<String>> {
    let mut child = Command::new(&job.target.script)
        .args(&job.args)
        .envs(job.env.iter().map(|(k, v)| (k, v)))
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
    type SystemConfig = Own;

    fn manifest() -> Manifest {
        Manifest {
            name: "Unit script".into(),
            subscribe: vec![topic::UNIT.into(), topic::CALL_START.into(), topic::CALL_END.into(), topic::CALL_CONCLUDED.into()],
            // A system sets these in its own settings (`Own`, on its card); a site of a
            // system with several may set them again for itself (SDK 0.14.0+). Systems
            // and Include conventional systems stay whole-recorder settings.
            site_overrides: vec!["script".into(), "workdir".into(), "actions".into(), "timeoutS".into()],
            ..trunk_recorder_plugin::manifest!()
        }
    }

    fn start(host: Host, setup: Setup<Config, Own>) -> Result<Self, String> {
        let chosen: HashSet<&str> = setup.config.systems.iter().map(String::as_str).collect();
        for name in &chosen {
            if setup.system_named(name).is_none() {
                host.warn(format!("{name} is ticked under Systems but isn't one of the recorder's systems"));
            }
        }
        let mut targets = HashMap::new();
        let mut sites_on = HashMap::new();
        let mut left_out = Vec::new();
        let mut lines = Vec::new();
        for s in &setup.systems {
            let wanted = if chosen.is_empty() { s.kind != "conventional" || setup.config.include_conventional } else { chosen.contains(s.short_name.as_str()) };
            if !wanted {
                left_out.push(s.short_name.clone());
                continue;
            }
            sites_on.insert(s.short_name.clone(), s.sites.iter().map(|site| site.id).collect::<HashSet<u32>>());
            // The system's: the plugin's with what the system set.
            let system = s.config.as_ref().map_or_else(|| setup.config.clone(), |own| own.over(&setup.config));
            let system_target = Target::of(&s.short_name, &system)?;
            if let Some(t) = &system_target {
                lines.push((s.short_name.clone(), t.clone()));
                targets.insert((s.short_name.clone(), 0), t.clone());
            }
            // A site with settings of its own: the system's with what the site set again.
            // `<shortName>` is the site's name only on a system with several sites.
            for site in &s.sites {
                let Some(own) = &site.config else { continue };
                let label = format!("{} site {}", s.short_name, site.name);
                // Only a site whose settings differ from its system's has its own.
                if let Some(t) = Target::of(&label, &own.over(&system))?.filter(|t| system_target.as_ref() != Some(t)) {
                    lines.push((label, t.clone()));
                    targets.insert((s.short_name.clone(), site.id), t);
                }
            }
        }
        if !left_out.is_empty() {
            left_out.sort();
            host.info(format!("not running the script for {}", left_out.join(", ")));
        }
        if targets.is_empty() {
            return Err(if setup.config.script.trim().is_empty() {
                "Set the script to run (Script).".into()
            } else {
                "No system runs the script: tick some under Systems, or turn on Include conventional systems.".into()
            });
        }
        lines.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, t) in &lines {
            let mut actions: Vec<&str> = t.actions.iter().map(|a| a.as_str()).collect();
            actions.sort_unstable();
            host.info(format!("{name}: {} in {} for {}", t.script.display(), t.workdir.display(), actions.join(", ")));
        }
        let systems: HashSet<&String> = targets.keys().map(|(name, _)| name).collect();
        let sites = targets.keys().filter(|(_, site)| *site != 0).count();
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        let stats = Arc::new(Stats::default());
        let (worker_host, worker_stats) = (host.clone(), stats.clone());
        let worker = thread::spawn(move || work(worker_host, rx, worker_stats));
        let own = if sites == 0 { String::new() } else { format!(", {sites} site(s) with their own") };
        host.status(State::Ok, format!("running the script for {} system(s){own}", systems.len()));
        host.metrics(&stats.metrics());
        Ok(UnitScript { host, targets, sites_on, calls: HashMap::new(), jobs: Some(tx), worker: Some(worker), stats })
    }

    fn unit(&mut self, e: UnitEvent) {
        if let Some(action) = Action::of_kind(&e.kind) {
            let from = Origin { short_name: &e.short_name, system_id: e.system_id, site: &e.site, site_id: e.site_id };
            self.queue(&from, e.unit, action, e.talkgroup);
        }
    }

    fn call_start(&mut self, call: CallInfo) {
        self.call_units(&call, false);
    }

    fn call_end(&mut self, call: CallInfo) {
        self.call_units(&call, true);
    }

    /// The recorder keeps a recorded call's files until every plugin taking recorded calls has
    /// reported on it, and counts those results on the plugin's card (Sent, Skipped, Last
    /// success, age at delivery). The radios heard were passed to the script as the call
    /// ended, so there is nothing more to wait for: say what became of the call now.
    fn call_concluded(&mut self, call: ConcludedCall) {
        // The settings of the site its kept copy is from, as for its radios as it ended.
        let (outcome, message) = match self.target(&call.call.short_name, call.call.site_id) {
            Some(t) if t.actions.contains(&Action::Call) => (Outcome::Ok, ""),
            Some(_) => (Outcome::Skipped, "the call action is off for this system or site"),
            None => (Outcome::Skipped, "this system or site doesn't run the script"),
        };
        self.host.call_result(&call.path, outcome, message, "");
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
        std::fs::write(&p, "#!/bin/sh\necho \"$(pwd) | $* | $TR_SHORT_NAME,$TR_SYSTEM_ID,$TR_SITE,$TR_SITE_ID\" >> calls.log\n").unwrap();
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

    #[cfg(unix)]
    #[test]
    fn a_system_can_set_its_own_actions() {
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let mut hello = testing::hello(&dir, json!({ "script": s }));
        // The system's own settings: empty ones are the plugin's.
        hello.systems[0].config = json!({ "actions": ["ackresp"] });
        let out = testing::run::<UnitScript>([HostMessage::Hello(hello), unit("affiliation", 1, Some(2)), unit("acknowledge", 3, None)]);
        assert!(out.ready());
        assert_eq!(log(&dir), ["sys1 3 ackresp "]);
    }

    /// Every setting a site may set again is one of a system's, and none is whole-recorder.
    #[test]
    fn site_overrides_are_settings_a_system_has() {
        let manifest = <UnitScript as Plugin>::manifest();
        assert!(manifest.system_overrides.is_empty(), "a system's are its own settings, not overrides of the plugin's");
        let schema = serde_json::to_value(schemars::schema_for!(Own)).unwrap();
        let props = schema["properties"].as_object().expect("properties");
        assert!(!manifest.site_overrides.is_empty());
        for name in &manifest.site_overrides {
            assert!(props.contains_key(name), "{name} is not a system setting");
        }
        for whole_recorder in ["systems", "includeConventional"] {
            assert!(!props.contains_key(whole_recorder), "{whole_recorder} must stay whole-recorder");
        }
    }

    /// The env the script ran with: `TR_SHORT_NAME,TR_SYSTEM_ID,TR_SITE,TR_SITE_ID` per run.
    fn envs(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default().lines().map(|l| l.split(" | ").nth(2).unwrap().to_string()).collect()
    }

    /// A hello whose `sys1` (id 7) has the sites `north` (11, as the system) and `south`
    /// (12, with `south` as its own settings).
    fn two_sites(dir: &Path, config: Value, south: Value) -> trunk_recorder_plugin::Hello {
        let mut hello = testing::hello(dir, config);
        hello.systems[0].id = 7;
        hello.systems[0].sites = vec![
            trunk_recorder_plugin::SiteInfo { id: 11, name: "north".into(), config: Value::Null },
            trunk_recorder_plugin::SiteInfo { id: 12, name: "south".into(), config: south },
        ];
        hello
    }

    fn unit_at(site: &str, site_id: u32, kind: &str, unit: u32) -> HostMessage {
        HostMessage::Unit(UnitEvent { short_name: "sys1".into(), system_id: 7, site: site.into(), site_id, kind: kind.into(), unit, ..Default::default() })
    }

    #[cfg(unix)]
    #[test]
    fn a_site_runs_its_own_script_named_for_the_site() {
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let south_dir = dir.join("south");
        std::fs::create_dir_all(&south_dir).unwrap();
        let s2 = script(&south_dir);
        let mut south_call = call(5, 100, &[3]);
        (south_call.system_id, south_call.site, south_call.site_id) = (7, "south".into(), 12);
        let out = testing::run::<UnitScript>([
            HostMessage::Hello(two_sites(&dir, json!({ "script": s }), json!({ "script": s2 }))),
            unit_at("north", 11, "registration", 1),
            unit_at("south", 12, "registration", 2),
            HostMessage::CallStart(south_call),
        ]);
        assert!(out.ready());
        assert_eq!(log(&dir), ["north 1 on "], "north has none of its own: the system's script, named for the site");
        assert_eq!(envs(&dir), ["sys1,7,north,11"]);
        assert_eq!(log(&south_dir), ["south 2 on ", "south 3 call 100"], "south's own script");
        assert_eq!(envs(&south_dir), ["sys1,7,south,12", "sys1,7,south,12"]);
        assert!(out.status().is_some_and(|(_, m)| m.contains("1 site(s) with their own")), "{:?}", out.status());
    }

    #[cfg(unix)]
    #[test]
    fn a_site_with_its_systems_settings_has_none_of_its_own() {
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let out = testing::run::<UnitScript>([HostMessage::Hello(two_sites(&dir, json!({ "script": s }), json!({ "script": s })))]);
        assert!(out.ready());
        assert!(out.status().is_some_and(|(_, m)| m == "running the script for 1 system(s)"), "{:?}", out.status());
    }

    #[cfg(unix)]
    #[test]
    fn a_system_of_one_site_is_named_by_its_short_name() {
        // Its one site is named for the system; an event with no site (conventional) is too.
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let mut hello = testing::hello(&dir, json!({ "script": s }));
        hello.systems[0].sites = vec![trunk_recorder_plugin::SiteInfo { id: 11, name: "sys1".into(), config: Value::Null }];
        let out = testing::run::<UnitScript>([HostMessage::Hello(hello), unit_at("sys1", 11, "registration", 1), unit("registration", 2, None)]);
        assert!(out.ready());
        assert_eq!(log(&dir), ["sys1 1 on ", "sys1 2 on "]);
    }

    #[cfg(unix)]
    #[test]
    fn a_site_the_plugin_is_off_for_runs_nothing() {
        // Calls as they go are the system's and still come from a site turned off (13).
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let mut off_call = call(5, 100, &[3]);
        (off_call.site, off_call.site_id) = ("east".into(), 13);
        let out = testing::run::<UnitScript>([HostMessage::Hello(two_sites(&dir, json!({ "script": s }), Value::Null)), HostMessage::CallStart(off_call)]);
        assert!(out.ready());
        assert!(log(&dir).is_empty(), "{:?}", log(&dir));
    }

    #[cfg(unix)]
    #[test]
    fn a_recorded_call_is_reported_by_its_site() {
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let mut north = testing::call(&dir, "sys1", 100);
        (north.call.site, north.call.site_id) = ("north".into(), 11);
        let mut south = testing::call(&dir, "sys1", 200);
        (south.call.site, south.call.site_id) = ("south".into(), 12);
        let out = testing::run::<UnitScript>([
            HostMessage::Hello(two_sites(&dir, json!({ "script": s }), json!({ "actions": ["on"] }))),
            HostMessage::CallConcluded(north),
            HostMessage::CallConcluded(south),
        ]);
        assert!(out.ready());
        let outcomes: Vec<Outcome> = out.results().into_iter().map(|r| r.1).collect();
        assert_eq!(outcomes, [Outcome::Ok, Outcome::Skipped], "south passes `on` only");
    }

    #[cfg(unix)]
    #[test]
    fn every_recorded_call_is_reported_on() {
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let out = testing::run::<UnitScript>([
            HostMessage::Hello(testing::hello(&dir, json!({ "script": s }))),
            HostMessage::CallConcluded(testing::call(&dir, "sys1", 100)),
        ]);
        assert!(out.ready());
        let results = out.results();
        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].1, Outcome::Ok, "{results:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_call_the_script_isnt_run_for_is_skipped() {
        // Conventional systems are left out unless asked for: skipped, not ok.
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let out = testing::run::<UnitScript>([
            HostMessage::Hello(with_conventional(&dir, json!({ "script": s }))),
            HostMessage::CallConcluded(testing::call(&dir, "conv", 1)),
        ]);
        assert!(out.ready());
        let results = out.results();
        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].1, Outcome::Skipped, "{results:?}");

        // The call action turned off: skipped too.
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let out = testing::run::<UnitScript>([
            HostMessage::Hello(testing::hello(&dir, json!({ "script": s, "actions": ["on"] }))),
            HostMessage::CallConcluded(testing::call(&dir, "sys1", 100)),
        ]);
        assert!(out.ready());
        assert_eq!(out.results()[0].1, Outcome::Skipped, "{:?}", out.results());
    }

    /// A hello with a conventional system beside the trunked `sys1`.
    fn with_conventional(dir: &Path, config: Value) -> trunk_recorder_plugin::Hello {
        let mut hello = testing::hello(dir, config);
        let mut conv = hello.systems[0].clone();
        conv.index = 1;
        conv.short_name = "conv".into();
        conv.kind = "conventional".into();
        hello.systems.push(conv);
        hello
    }

    fn conv_call(id: u32, units: &[u32]) -> HostMessage {
        HostMessage::CallStart(CallInfo { id, short_name: "conv".into(), talkgroup: 1, units: units.to_vec(), ..Default::default() })
    }

    #[cfg(unix)]
    #[test]
    fn conventional_systems_are_left_out_unless_asked_for() {
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let out = testing::run::<UnitScript>([
            HostMessage::Hello(with_conventional(&dir, json!({ "script": s }))),
            conv_call(1, &[77]),
            HostMessage::CallStart(call(2, 100, &[88])),
        ]);
        assert!(out.ready());
        assert_eq!(log(&dir), ["sys1 88 call 100"]);
        assert!(out.logs().iter().any(|(_, m)| m == "not running the script for conv"), "{:?}", out.logs());

        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let out =
            testing::run::<UnitScript>([HostMessage::Hello(with_conventional(&dir, json!({ "script": s, "includeConventional": true }))), conv_call(1, &[77])]);
        assert!(out.ready());
        assert_eq!(log(&dir), ["conv 77 call 1"]);
    }

    #[cfg(unix)]
    #[test]
    fn systems_ticked_are_the_only_ones() {
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let out = testing::run::<UnitScript>([
            HostMessage::Hello(with_conventional(&dir, json!({ "script": s, "systems": ["conv"] }))),
            conv_call(1, &[77]),
            unit("registration", 5, None),
        ]);
        assert!(out.ready());
        assert_eq!(log(&dir), ["conv 77 call 1"], "sys1 isn't ticked");
    }

    #[cfg(unix)]
    #[test]
    fn nothing_to_run_for_is_a_settings_error() {
        let dir = testing::temp_dir("unit-script");
        let s = script(&dir);
        let mut hello = testing::hello(&dir, json!({ "script": s }));
        hello.systems[0].kind = "conventional".into();
        let out = testing::run::<UnitScript>([HostMessage::Hello(hello)]);
        assert!(!out.ready());
        assert!(out.status().is_some_and(|(_, m)| m.contains("Include conventional")));
    }

    #[cfg(unix)]
    #[test]
    fn the_card_gets_its_figures() {
        let dir = testing::temp_dir("unit-script");
        let p = dir.join("bad.sh");
        std::fs::write(&p, "#!/bin/sh\n[ \"$3\" = on ] && exit 0\necho nope >&2\nexit 1\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = testing::run::<UnitScript>([
            HostMessage::Hello(testing::hello(&dir, json!({ "script": p }))),
            unit("registration", 1, None),
            unit("deregistration", 1, None),
        ]);
        let last = out.metrics().pop().expect("metrics sent");
        assert_eq!(last.extra["script runs"], 2);
        assert_eq!(last.extra["script failures"], 1);
        assert_eq!(last.queued, Some(0));
        assert!(last.last_ok.is_some() && last.last_error_text.contains("nope"), "{last:?}");
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
