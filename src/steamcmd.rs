//! SteamCMD: downloads Workshop mods without the Steam client (copies of the game without
//! Steam, or when the client won't subscribe). Valve's console tool runs hidden; its output
//! is read line by line and shown in the manager's own window. Mods are downloaded
//! anonymously into SteamCMD's folder, and a link (directory junction) to each one is put
//! into the game's Mods folder, so nothing is stored twice and a re-download updates the mod.

use regex::Regex;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

pub const INSTALLER_URL: &str = "https://steamcdn-a.akamaihd.net/client/installer/steamcmd.zip";
pub const RIMWORLD_APP_ID: &str = "294100";
/// Items per SteamCMD run: the command line has a length limit.
const CHUNK: usize = 150;
/// Log lines kept for the window.
const LOG_LINES: usize = 400;
/// The installer is about 3 MB; anything far bigger is not it.
const MAX_INSTALLER: u64 = 50 * 1024 * 1024;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Returned by a run that was stopped from the window.
pub const STOPPED: &str = "stopped";
/// Returned by a run the watchdog stopped: it printed nothing for too long.
const IDLE: &str = "idle";
/// The update step prints steadily; silence this long means the restarted copy hangs.
/// (Without a console, SteamCMD restarted after updating itself never quits; the update is
/// already in place by then.)
const UPDATE_IDLE: Duration = Duration::from_secs(45);

/// The running SteamCMD process, so the window can stop it.
pub type KillSwitch = Arc<Mutex<Option<Child>>>;

pub fn default_dir() -> PathBuf {
    crate::game_install::manager_data_dir().join("steamcmd")
}

pub fn exe_path(dir: &Path) -> PathBuf {
    dir.join("steamcmd.exe")
}

pub fn is_installed(dir: &Path) -> bool {
    exe_path(dir).is_file()
}

/// Where SteamCMD puts RimWorld's Workshop items: `<dir>\steamapps\workshop\content\294100`.
pub fn content_dir(dir: &Path) -> PathBuf {
    dir.join("steamapps")
        .join("workshop")
        .join("content")
        .join(RIMWORLD_APP_ID)
}

pub fn is_downloaded(dir: &Path, id: &str) -> bool {
    content_dir(dir).join(id).is_dir()
}

/// A mod folder inside some SteamCMD's download folder (`...\content\294100\<id>` with
/// `steamcmd.exe` five levels up): such files may be deleted with the mod.
pub fn is_steamcmd_item(path: &Path) -> bool {
    let path = plain_path(path);
    path.parent()
        .and_then(Path::file_name)
        .is_some_and(|n| n == RIMWORLD_APP_ID)
        && path.ancestors().nth(5).is_some_and(|d| exe_path(d).is_file())
}

/// `\\?\C:\x` -> `C:\x` (what links and `canonicalize` return on Windows).
pub fn plain_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

// ----- Output -----

/// What a line of SteamCMD output says about an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineEvent {
    Started(String),
    Downloaded(String),
    Failed(String, String),
}

static STARTED_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Downloading item (\d+)").unwrap());
static DONE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Success\. Downloaded item (\d+)").unwrap());
static FAILED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"ERROR! Download item (\d+) failed \(([^)]*)\)").unwrap());
static TIMEOUT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"ERROR! Timeout downloading item (\d+)").unwrap());
static ESCAPE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").unwrap());

pub fn parse_line(line: &str) -> Option<LineEvent> {
    if let Some(c) = DONE_RE.captures(line) {
        return Some(LineEvent::Downloaded(c[1].to_string()));
    }
    if let Some(c) = FAILED_RE.captures(line) {
        return Some(LineEvent::Failed(c[1].to_string(), c[2].to_string()));
    }
    if let Some(c) = TIMEOUT_RE.captures(line) {
        return Some(LineEvent::Failed(c[1].to_string(), "Timeout".to_string()));
    }
    STARTED_RE.captures(line).map(|c| LineEvent::Started(c[1].to_string()))
}

/// A line as shown in the log: no color codes or control characters, trimmed.
fn clean_line(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let text = ESCAPE_RE.replace_all(&text, "");
    text.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

/// Arguments for one run: anonymous login (RimWorld's Workshop allows it), the items, quit.
pub fn download_args(ids: &[String]) -> Vec<String> {
    let mut args = vec!["+login".to_string(), "anonymous".to_string()];
    for id in ids {
        args.extend([
            "+workshop_download_item".to_string(),
            RIMWORLD_APP_ID.to_string(),
            id.clone(),
        ]);
    }
    args.push("+quit".to_string());
    args
}

// ----- Running SteamCMD -----

/// Runs SteamCMD hidden and passes each output line on. Lines end with `\n` or `\r`
/// (progress lines are redrawn with `\r`). With `idle_limit`, a run that prints nothing for
/// that long is stopped (process tree included) and returns `IDLE`.
fn run(
    exe: &Path,
    args: &[String],
    kill: &KillSwitch,
    idle_limit: Option<Duration>,
    on_line: &mut dyn FnMut(String),
) -> Result<(), String> {
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let mut out = child.stdout.take().ok_or("no output")?;
    let pid = child.id();
    *lock(kill) = Some(child);

    // The watchdog: output is awaited with a blocking read, so a hung SteamCMD is stopped
    // from another thread (its whole tree: the pipe stays open while any copy runs).
    let last_output = Arc::new(Mutex::new(std::time::Instant::now()));
    let finished = Arc::new(AtomicBool::new(false));
    let idle_stopped = Arc::new(AtomicBool::new(false));
    if let Some(limit) = idle_limit {
        let (last_output, finished, idle_stopped) = (last_output.clone(), finished.clone(), idle_stopped.clone());
        std::thread::spawn(move || {
            while !finished.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(500));
                let idle = last_output.lock().map(|t| t.elapsed()).unwrap_or_default();
                if idle > limit && !finished.load(Ordering::Relaxed) {
                    idle_stopped.store(true, Ordering::Relaxed);
                    kill_tree(pid);
                    break;
                }
            }
        });
    }

    let mut buf = [0u8; 4096];
    let mut line = Vec::new();
    loop {
        let n = match out.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if let Ok(mut t) = last_output.lock() {
            *t = std::time::Instant::now();
        }
        for &b in &buf[..n] {
            if b == b'\n' || b == b'\r' {
                let text = clean_line(&line);
                if !text.is_empty() {
                    on_line(text);
                }
                line.clear();
            } else {
                line.push(b);
            }
        }
    }
    let text = clean_line(&line);
    if !text.is_empty() {
        on_line(text);
    }
    finished.store(true, Ordering::Relaxed);
    // Taken by `stop` = stopped from the window.
    let child = lock(kill).take();
    match child {
        Some(mut c) => {
            let _ = c.wait();
            if idle_stopped.load(Ordering::Relaxed) {
                Err(IDLE.to_string())
            } else {
                Ok(())
            }
        }
        None => Err(STOPPED.to_string()),
    }
}

fn lock(kill: &KillSwitch) -> std::sync::MutexGuard<'_, Option<Child>> {
    kill.lock().unwrap_or_else(|e| e.into_inner())
}

/// Ends a SteamCMD and the copies it started (after updating itself it restarts as a child).
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}

/// Stops the running SteamCMD, if any.
pub fn stop(kill: &KillSwitch) {
    if let Some(mut c) = lock(kill).take() {
        kill_tree(c.id());
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Downloads Valve's installer and unpacks `steamcmd.exe` into `dir`.
pub fn download_installer(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(120)).build();
    let response = agent.get(INSTALLER_URL).call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_INSTALLER)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut file = zip.by_name("steamcmd.exe").map_err(|e| e.to_string())?;
    let mut exe = Vec::new();
    file.read_to_end(&mut exe).map_err(|e| e.to_string())?;
    let target = exe_path(dir);
    let tmp = target.with_extension("exe.tmp");
    std::fs::write(&tmp, exe).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

// ----- Links in the game's Mods folder -----

/// What happened to a mod's place in the game's Mods folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Linked {
    New,
    Already,
    /// Mods already has a real folder with this name (a copy put there by hand): left alone.
    OwnCopy,
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let key = |p: &Path| {
        plain_path(&std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    key(a) == key(b)
}

/// Puts a link `<mods_dir>\<id>` -> the downloaded mod.
pub fn link_into_mods(source: &Path, mods_dir: &Path, id: &str) -> Result<Linked, String> {
    let target = mods_dir.join(id);
    if let Ok(points_to) = std::fs::read_link(&target) {
        if same_dir(&points_to, source) && target.is_dir() {
            return Ok(Linked::Already);
        }
        // A link to somewhere else (an old SteamCMD folder): replaced.
        std::fs::remove_dir(&target).map_err(|e| e.to_string())?;
    } else if target.exists() {
        return Ok(Linked::OwnCopy);
    }
    std::fs::create_dir_all(mods_dir).map_err(|e| e.to_string())?;
    make_link(source, &target)?;
    Ok(Linked::New)
}

/// A directory junction: needs no admin rights, works across local drives.
#[cfg(windows)]
fn make_link(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let output = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(target)
        .arg(plain_path(source))
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() && target.is_dir() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(not(windows))]
fn make_link(source: &Path, target: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(source, target).map_err(|e| e.to_string())
}

// ----- A job: install and/or download, step by step -----

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemState {
    Waiting,
    Downloading,
    Downloaded,
    Failed(String),
    Linked,
    OwnCopy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobItem {
    pub id: String,
    pub title: String,
    pub state: ItemState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Asking Steam for the mods' titles.
    Starting,
    /// Downloading Valve's installer.
    Installing,
    /// SteamCMD updates itself (first start: about 40 MB).
    Updating,
    Downloading,
    Linking,
    /// Running the user's console commands.
    Console,
    Done,
    Stopped,
    Failed(String),
}

impl Phase {
    pub fn is_finished(&self) -> bool {
        matches!(self, Phase::Done | Phase::Stopped | Phase::Failed(_))
    }
}

/// Messages from the job thread.
#[derive(Debug, Clone)]
pub enum Msg {
    Phase(Phase),
    Line(String),
    Titles(HashMap<String, String>),
    Item(String, ItemState),
}

/// What the window shows.
#[derive(Debug, Clone)]
pub struct Job {
    pub items: Vec<JobItem>,
    pub phase: Phase,
    pub log: VecDeque<String>,
}

impl Job {
    pub fn new(ids: &[String]) -> Self {
        Self {
            items: ids
                .iter()
                .map(|id| JobItem {
                    id: id.clone(),
                    title: String::new(),
                    state: ItemState::Waiting,
                })
                .collect(),
            phase: Phase::Starting,
            log: VecDeque::new(),
        }
    }

    pub fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Phase(p) => self.phase = p,
            Msg::Line(line) => {
                if self.log.len() >= LOG_LINES {
                    self.log.pop_front();
                }
                self.log.push_back(line);
            }
            Msg::Titles(titles) => {
                for item in &mut self.items {
                    if let Some(t) = titles.get(&item.id) {
                        item.title = t.clone();
                    }
                }
            }
            Msg::Item(id, state) => {
                if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
                    item.state = state;
                }
            }
        }
    }

    /// Items that reached the Mods folder (or were already there as a copy).
    pub fn finished_items(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.state, ItemState::Linked | ItemState::OwnCopy))
            .count()
    }

    pub fn failed_items(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.state, ItemState::Failed(_)))
            .count()
    }

    /// 0..1 for the progress bar: each item counts once downloaded or failed.
    pub fn progress(&self) -> f32 {
        if self.items.is_empty() {
            return if self.phase.is_finished() { 1.0 } else { 0.0 };
        }
        let done = self
            .items
            .iter()
            .filter(|i| !matches!(i.state, ItemState::Waiting | ItemState::Downloading))
            .count();
        done as f32 / self.items.len() as f32
    }
}

/// Installs SteamCMD if needed, then lets it update itself in a run of its own, under the
/// watchdog: an update in the middle of a download could hang it with no way to tell.
/// Err = the phase the job ends with.
fn prepare(dir: &Path, kill: &KillSwitch, send: &dyn Fn(Msg)) -> Result<(), Phase> {
    if !is_installed(dir) {
        send(Msg::Phase(Phase::Installing));
        download_installer(dir).map_err(Phase::Failed)?;
    }
    send(Msg::Phase(Phase::Updating));
    match run(
        &exe_path(dir),
        &["+quit".to_string()],
        kill,
        Some(UPDATE_IDLE),
        &mut |l| send(Msg::Line(l)),
    ) {
        Err(e) if e == STOPPED => return Err(Phase::Stopped),
        // The restarted copy hung after the update: the update itself is in place.
        Err(e) if e == IDLE => {}
        Err(e) => return Err(Phase::Failed(e)),
        Ok(()) => {}
    }
    if is_installed(dir) {
        Ok(())
    } else {
        Err(Phase::Failed("steamcmd.exe".to_string()))
    }
}

/// Splits the console input into SteamCMD arguments: commands one per line or split by
/// `;`, each becoming `+command args...` ("quoted words" stay whole), after an anonymous
/// login. `login` lines are dropped (an account password would end up in the log), and so
/// are `quit`/`exit` (added at the end). Returns the arguments and whether a login was dropped.
pub fn console_args(input: &str) -> (Vec<String>, bool) {
    let mut args = vec!["+login".to_string(), "anonymous".to_string()];
    let mut dropped_login = false;
    for command in input.split(['\n', ';']).map(str::trim).filter(|c| !c.is_empty()) {
        let words = split_words(command.trim_start_matches('+'));
        let Some((first, rest)) = words.split_first() else {
            continue;
        };
        let lower = first.to_lowercase();
        if lower == "login" {
            dropped_login = true;
            continue;
        }
        if lower == "quit" || lower == "exit" {
            continue;
        }
        args.push(format!("+{first}"));
        args.extend(rest.iter().cloned());
    }
    args.push("+quit".to_string());
    (args, dropped_login)
}

/// Words split by spaces; "double quoted" parts stay one word (without the quotes).
fn split_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut any) = (false, false);
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    words.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            c => {
                word.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(word);
    }
    words
}

/// The console: prepares SteamCMD like a download does, then runs the user's commands.
pub fn run_console(dir: &Path, args: &[String], kill: &KillSwitch, tx: &Sender<Msg>, repaint: &dyn Fn()) {
    let send = |msg: Msg| {
        let _ = tx.send(msg);
        repaint();
    };
    if let Err(phase) = prepare(dir, kill, &send) {
        return send(Msg::Phase(phase));
    }
    send(Msg::Phase(Phase::Console));
    send(Msg::Line(format!("> {}", args.join(" "))));
    let phase = match run(&exe_path(dir), args, kill, None, &mut |l| send(Msg::Line(l))) {
        Err(e) if e == STOPPED => Phase::Stopped,
        Err(e) => Phase::Failed(e),
        Ok(()) => Phase::Done,
    };
    send(Msg::Phase(phase));
}

/// Runs a job in the calling thread: installs SteamCMD if needed, lets it update itself,
/// downloads the items, retries the failed ones once, links them into
/// `mods_dir`. Every step is sent to `tx`; `repaint` wakes the window.
pub fn run_job(
    dir: &Path,
    ids: &[String],
    mods_dir: Option<&Path>,
    kill: &KillSwitch,
    tx: &Sender<Msg>,
    repaint: &dyn Fn(),
) {
    let send = |msg: Msg| {
        let _ = tx.send(msg);
        repaint();
    };
    let finish = |phase: Phase| send(Msg::Phase(phase));
    let exe = exe_path(dir);

    if !ids.is_empty() {
        if let Ok(details) = crate::mod_updates::fetch_details(ids) {
            send(Msg::Titles(
                details
                    .into_iter()
                    .filter(|(_, d)| !d.title.is_empty())
                    .map(|(id, d)| (id, d.title))
                    .collect(),
            ));
        }
    }

    if let Err(phase) = prepare(dir, kill, &send) {
        return finish(phase);
    }
    if ids.is_empty() {
        return finish(Phase::Done);
    }

    // 2. Download; the ones that failed get one more try.
    send(Msg::Phase(Phase::Downloading));
    let mut reasons: HashMap<String, String> = HashMap::new();
    for attempt in 0..2 {
        let todo: Vec<String> = if attempt == 0 {
            ids.to_vec()
        } else {
            ids.iter().filter(|id| !is_downloaded(dir, id)).cloned().collect()
        };
        if todo.is_empty() {
            break;
        }
        for chunk in todo.chunks(CHUNK) {
            let mut on_line = |line: String| {
                match parse_line(&line) {
                    Some(LineEvent::Started(id)) => send(Msg::Item(id, ItemState::Downloading)),
                    Some(LineEvent::Downloaded(id)) => send(Msg::Item(id, ItemState::Downloaded)),
                    Some(LineEvent::Failed(id, reason)) => {
                        reasons.insert(id.clone(), reason.clone());
                        send(Msg::Item(id, ItemState::Failed(reason)));
                    }
                    None => {}
                }
                send(Msg::Line(line));
            };
            // No watchdog: a big mod downloads for minutes without a line of output.
            match run(&exe, &download_args(chunk), kill, None, &mut on_line) {
                Err(e) if e == STOPPED => return finish(Phase::Stopped),
                Err(e) => return finish(Phase::Failed(e)),
                Ok(()) => {}
            }
        }
    }

    // 3. Links in the game's Mods folder.
    send(Msg::Phase(Phase::Linking));
    let content = content_dir(dir);
    for id in ids {
        if !is_downloaded(dir, id) {
            let reason = reasons.get(id).cloned().unwrap_or_default();
            send(Msg::Item(id.clone(), ItemState::Failed(reason)));
            continue;
        }
        let Some(mods_dir) = mods_dir else {
            send(Msg::Item(id.clone(), ItemState::Failed(String::new())));
            continue;
        };
        let state = match link_into_mods(&content.join(id), mods_dir, id) {
            Ok(Linked::New | Linked::Already) => ItemState::Linked,
            Ok(Linked::OwnCopy) => ItemState::OwnCopy,
            Err(e) => ItemState::Failed(e),
        };
        send(Msg::Item(id.clone(), state));
    }
    finish(Phase::Done);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_steamcmd_output() {
        assert_eq!(
            parse_line("Downloading item 818773962 ..."),
            Some(LineEvent::Started("818773962".into()))
        );
        assert_eq!(
            parse_line(
                r#"Success. Downloaded item 818773962 to "C:\s\steamapps\workshop\content\294100\818773962" (1234 bytes)"#
            ),
            Some(LineEvent::Downloaded("818773962".into()))
        );
        assert_eq!(
            parse_line("ERROR! Download item 123 failed (File Not Found)."),
            Some(LineEvent::Failed("123".into(), "File Not Found".into()))
        );
        assert_eq!(
            parse_line("ERROR! Timeout downloading item 456"),
            Some(LineEvent::Failed("456".into(), "Timeout".into()))
        );
        assert_eq!(parse_line("Connecting anonymously to Steam Public...OK"), None);
        assert_eq!(
            clean_line(b"\x1b[0m[  0%] Checking for available update...\x07  "),
            "[  0%] Checking for available update..."
        );
    }

    #[test]
    fn console_commands() {
        let (args, dropped) = console_args(
            "workshop_download_item 294100 1\n+app_info_print 294100; login me secret\nforce_install_dir \"C:\\My Dir\"\nquit",
        );
        assert_eq!(
            args,
            [
                "+login",
                "anonymous",
                "+workshop_download_item",
                "294100",
                "1",
                "+app_info_print",
                "294100",
                "+force_install_dir",
                "C:\\My Dir",
                "+quit"
            ]
        );
        assert!(dropped, "the password line never reaches SteamCMD or the log");
        assert_eq!(console_args("  ").0, ["+login", "anonymous", "+quit"]);
    }

    #[test]
    fn arguments_for_a_download() {
        assert_eq!(
            download_args(&["1".into(), "2".into()]),
            [
                "+login",
                "anonymous",
                "+workshop_download_item",
                "294100",
                "1",
                "+workshop_download_item",
                "294100",
                "2",
                "+quit"
            ]
        );
    }

    #[test]
    fn job_progress() {
        let mut job = Job::new(&["1".into(), "2".into()]);
        job.apply(Msg::Titles([("1".to_string(), "HugsLib".to_string())].into()));
        job.apply(Msg::Item("1".into(), ItemState::Linked));
        job.apply(Msg::Item("2".into(), ItemState::Failed("Timeout".into())));
        for i in 0..LOG_LINES + 5 {
            job.apply(Msg::Line(i.to_string()));
        }
        assert_eq!(job.items[0].title, "HugsLib");
        assert_eq!((job.finished_items(), job.failed_items()), (1, 1));
        assert_eq!(job.progress(), 1.0);
        assert_eq!(job.log.len(), LOG_LINES);
    }

    #[test]
    fn links_a_downloaded_mod_into_mods() {
        let root = std::env::temp_dir().join(format!("rwmm_steamcmd_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let steamcmd = root.join("steam cmd");
        let item = content_dir(&steamcmd).join("42");
        std::fs::create_dir_all(item.join("About")).unwrap();
        std::fs::write(item.join("About").join("About.xml"), "<ModMetaData/>").unwrap();
        std::fs::write(exe_path(&steamcmd), "").unwrap();
        let mods = root.join("Game Mods");

        assert_eq!(link_into_mods(&item, &mods, "42"), Ok(Linked::New));
        assert!(mods.join("42").join("About").join("About.xml").is_file());
        assert_eq!(link_into_mods(&item, &mods, "42"), Ok(Linked::Already));
        let link_target = std::fs::read_link(mods.join("42")).unwrap();
        assert!(is_steamcmd_item(&link_target), "{link_target:?}");
        assert!(!is_steamcmd_item(&mods.join("42")));

        // A real folder put into Mods by hand is left alone.
        std::fs::create_dir_all(mods.join("43")).unwrap();
        assert_eq!(link_into_mods(&item, &mods, "43"), Ok(Linked::OwnCopy));

        // Removing the link keeps the downloaded files.
        std::fs::remove_dir(mods.join("42")).unwrap();
        assert!(item.join("About").join("About.xml").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Installs SteamCMD into a temp folder and downloads HugsLib (run by hand:
    /// `cargo test --release live_steamcmd -- --ignored --nocapture`; ~100 MB of traffic).
    #[test]
    #[ignore]
    fn live_steamcmd() {
        let dir = std::env::temp_dir().join("rwmm_steamcmd_live");
        let mods = dir.join("Mods");
        let kill: KillSwitch = Arc::new(Mutex::new(None));
        let (tx, rx) = std::sync::mpsc::channel();
        let ids = vec!["818773962".to_string()];
        run_job(&dir.join("steamcmd"), &ids, Some(&mods), &kill, &tx, &|| {});
        drop(tx);
        let mut job = Job::new(&ids);
        for msg in rx {
            if let Msg::Line(l) = &msg {
                println!("| {l}");
            }
            job.apply(msg);
        }
        println!("{:?}\n{:?}", job.phase, job.items);
        assert_eq!(job.phase, Phase::Done);
        assert_eq!(job.items[0].state, ItemState::Linked);
        assert!(mods.join("818773962").join("About").is_dir());

        // The console: the same item, typed as a command.
        let (args, _) = console_args("workshop_download_item 294100 818773962");
        let (tx, rx) = std::sync::mpsc::channel();
        run_console(&dir.join("steamcmd"), &args, &kill, &tx, &|| {});
        drop(tx);
        let mut console = Job::new(&[]);
        for msg in rx {
            console.apply(msg);
        }
        println!(
            "console: {:?}\n{}",
            console.phase,
            console.log.iter().cloned().collect::<Vec<_>>().join("\n")
        );
        assert_eq!(console.phase, Phase::Done);
        assert!(console
            .log
            .iter()
            .any(|l| l.contains("Success. Downloaded item 818773962")));
    }
}
