use crate::i18n::{tr, Language, TranslationKey};
use crate::models::ModInfo;
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Severity level of a detected crash or issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ErrorSeverity {
    Warning,
    MissingDependency,
    /// ERROR entry whose stack trace points into a mod (RimWorld 1.7 log format).
    ModError,
    BrokenTexture,
    CrossReferenceError,
    InstantiationError,
    DefException,
    PrepatcherHarmonyError,
    HardCrash,
}

impl ErrorSeverity {
    /// Badge text in `lang`.
    pub fn badge_text(&self, lang: Language) -> &'static str {
        let key = match self {
            ErrorSeverity::HardCrash => TranslationKey::SevHardCrash,
            ErrorSeverity::PrepatcherHarmonyError => TranslationKey::SevPrepatcherHarmony,
            ErrorSeverity::DefException => TranslationKey::SevDefException,
            ErrorSeverity::InstantiationError => TranslationKey::SevInstantiation,
            ErrorSeverity::CrossReferenceError => TranslationKey::SevCrossReference,
            ErrorSeverity::BrokenTexture => TranslationKey::SevBrokenTexture,
            ErrorSeverity::MissingDependency => TranslationKey::SevMissingDependency,
            ErrorSeverity::ModError => TranslationKey::SevModError,
            ErrorSeverity::Warning => TranslationKey::SevWarning,
        };
        key.translate(lang)
    }

    pub fn badge_colors(&self) -> ([u8; 3], [u8; 3]) {
        match self {
            ErrorSeverity::HardCrash => ([180, 45, 45], [255, 255, 255]),
            ErrorSeverity::PrepatcherHarmonyError => ([195, 95, 25], [255, 255, 255]),
            ErrorSeverity::DefException => ([180, 115, 25], [255, 255, 255]),
            ErrorSeverity::InstantiationError => ([185, 105, 35], [255, 255, 255]),
            ErrorSeverity::CrossReferenceError => ([175, 120, 25], [255, 255, 255]),
            ErrorSeverity::BrokenTexture => ([165, 80, 50], [255, 255, 255]),
            ErrorSeverity::MissingDependency => ([155, 125, 20], [255, 255, 255]),
            ErrorSeverity::ModError => ([170, 100, 30], [255, 255, 255]),
            ErrorSeverity::Warning => ([80, 85, 95], [230, 230, 235]),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CrashCulprit {
    pub mod_name: String,
    pub package_id: Option<String>,
    pub mod_dir: Option<PathBuf>,
    pub severity: ErrorSeverity,
    pub headline: String,
    pub explanation: String,
    pub stack_trace_excerpt: String,
    pub occurrences: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemCrashIssue {
    /// D3D error 887a0005 / DXGI_ERROR_DEVICE_REMOVED (GPU reset / VRAM exhaustion).
    GpuReset,
    /// Out of memory (RAM exhaustion / could not allocate memory).
    OutOfMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogViewMode {
    #[default]
    Report,
    FullLog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevelFilter {
    #[default]
    All,
    Errors,
    Warnings,
    Info,
}

#[derive(Debug, Clone, Default)]
pub struct LogReport {
    pub log_path: PathBuf,
    pub log_file_name: String,
    pub file_size_bytes: u64,
    pub has_hard_crash: bool,
    pub system_issue: Option<SystemCrashIssue>,
    /// Total loading time in seconds, if detected from log timestamps or mod output (F11).
    pub load_time_secs: Option<f32>,
    pub total_lines: usize,
    pub total_errors: usize,
    pub culprits: Vec<CrashCulprit>,
    pub summary_text: String,
    /// Raw contents of the log file for viewing and report export (F13).
    pub raw_content: String,
    /// Log written by RimWorld 1.7+ (Waylith engine) instead of Unity/Mono.
    pub is_waylith: bool,
    /// The log's last lines before the crash (empty without a crash): for "Copy for AI",
    /// which needs the log itself when no mod could be blamed.
    pub crash_excerpt: String,
    /// The last different errors in the log, each with a few lines after it.
    pub recent_errors: Vec<String>,
}

/// Lines in a log excerpt at most, and characters per line.
const EXCERPT_LINES: usize = 40;
const EXCERPT_LINE_CHARS: usize = 300;
/// Errors kept for "Copy for AI".
const RECENT_ERRORS: usize = 5;

/// `lines[start..end]` without empty lines, long lines cut, at most the last `EXCERPT_LINES`.
fn log_excerpt(lines: &[&str], start: usize, end: usize) -> String {
    let end = end.min(lines.len());
    let kept: Vec<String> = lines[start.min(end)..end]
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().take(EXCERPT_LINE_CHARS).collect())
        .collect();
    kept[kept.len().saturating_sub(EXCERPT_LINES)..].join("\n")
}

/// The last `RECENT_ERRORS` different errors (newest last), each with up to 5 lines after it.
fn recent_errors(lines: &[&str], is_waylith: bool) -> Vec<String> {
    let starts: Vec<(usize, usize)> = if is_waylith {
        parse_waylith_entries(lines)
            .into_iter()
            .filter(|e| matches!(e.level, "ERROR" | "FATAL" | "CRITICAL"))
            .map(|e| (e.first_line, e.end_line.max(e.first_line + 1)))
            .collect()
    } else {
        lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.contains("Exception:") || l.contains("Error in ") || l.starts_with("Error "))
            .map(|(i, _)| (i, i + 6))
            .collect()
    };
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<String> = starts
        .into_iter()
        .rev()
        .filter(|(start, _)| seen.insert(lines[*start].trim().to_string()))
        .take(RECENT_ERRORS)
        .map(|(start, end)| log_excerpt(lines, start, end.min(start + 6)))
        .collect();
    out.reverse();
    out
}

#[derive(Default)]
struct ModMatcher<'a> {
    /// DLL file stem -> mods that ship it. A DLL found in several mods (0Harmony.dll,
    /// shared libraries) is ambiguous and never used to blame a mod.
    dll_map: HashMap<String, Vec<&'a ModInfo>>,
    package_id_map: HashMap<String, &'a ModInfo>,
    name_map: HashMap<String, &'a ModInfo>,
}

impl<'a> ModMatcher<'a> {
    fn build(mods: &'a [ModInfo]) -> Self {
        let mut matcher = Self::default();

        for m in mods {
            matcher.package_id_map.insert(m.package_id.to_lowercase(), m);
            matcher.name_map.insert(m.name.to_lowercase(), m);

            if m.dir_path.exists() {
                for entry in walkdir::WalkDir::new(&m.dir_path).max_depth(5).into_iter().flatten() {
                    let path = entry.path();
                    let is_dll = path.extension().map(|e| e.eq_ignore_ascii_case("dll")).unwrap_or(false);
                    if is_dll && entry.file_type().is_file() {
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            let owners = matcher.dll_map.entry(stem.to_lowercase()).or_default();
                            if !owners.iter().any(|o| std::ptr::eq(*o, m)) {
                                owners.push(m);
                            }
                        }
                    }
                }
            }
        }

        matcher
    }

    /// The single mod that ships this DLL, if exactly one does.
    fn unique_dll_owner(&self, stem: &str) -> Option<&'a ModInfo> {
        match self.dll_map.get(stem).map(Vec::as_slice) {
            Some([only]) => Some(*only),
            _ => None,
        }
    }

    fn find_mod(&self, token: &str) -> Option<&'a ModInfo> {
        let clean = token.trim().to_lowercase();
        if clean.is_empty() {
            return None;
        }

        if let Some(m) = self.unique_dll_owner(&clean) {
            return Some(m);
        }

        if let Some(m) = self.package_id_map.get(&clean) {
            return Some(m);
        }

        if let Some(m) = self.name_map.get(&clean) {
            return Some(m);
        }

        for part in clean.split('.') {
            if part.len() >= 4 {
                if let Some(m) = self.unique_dll_owner(part) {
                    return Some(m);
                }
            }
        }

        // Last resort: a packageId contained in the token. Deterministic: the longest
        // (most specific) match wins, short IDs are ignored to avoid accidental hits.
        self.package_id_map
            .iter()
            .filter(|(pkg_id, _)| pkg_id.len() >= 8 && clean.contains(pkg_id.as_str()))
            .max_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| b.0.cmp(a.0)))
            .map(|(_, m)| *m)
    }
}

/// Summary line in the requested language (the report's own summary_text uses the
/// interface language at the time of the analysis).
pub fn summary_for(report: &LogReport, lang: Language) -> String {
    let n = report.culprits.len().to_string();
    let key = match report.system_issue {
        Some(SystemCrashIssue::GpuReset) => TranslationKey::SummaryGpuReset,
        Some(SystemCrashIssue::OutOfMemory) => TranslationKey::SummaryOutOfMemory,
        None if report.has_hard_crash => TranslationKey::SummaryHardCrash,
        None if !report.culprits.is_empty() => TranslationKey::SummaryCulprits,
        None if report.total_errors > 0 => {
            let e = report.total_errors.to_string();
            return crate::i18n::fmt(TranslationKey::SummaryErrorsOnly.translate(lang), &[&e]);
        }
        None => TranslationKey::SummaryClean,
    };
    crate::i18n::fmt(key.translate(lang), &[&n])
}

/// Finds Player.log files, newest first.
/// RimWorld 1.7+ writes them to `<user data>\Logs\`, 1.6 and older to `<user data>\`.
pub fn detect_rimworld_log_paths() -> Vec<(String, PathBuf)> {
    let Some(base) = crate::game_install::user_data_dir() else {
        return Vec::new();
    };
    log_paths_in(&base)
}

fn log_paths_in(base: &Path) -> Vec<(String, PathBuf)> {
    let candidates = [
        (
            tr!(
                "Logs/Player.log (1.7+, последний запуск)",
                "Logs/Player.log (1.7+, last run)",
                "Logs/Player.log（1.7+，上次运行）"
            ),
            base.join("Logs").join("Player.log"),
        ),
        (
            tr!(
                "Logs/Player-prev.log (1.7+, предыдущий запуск)",
                "Logs/Player-prev.log (1.7+, previous run)",
                "Logs/Player-prev.log（1.7+，再上一次运行）"
            ),
            base.join("Logs").join("Player-prev.log"),
        ),
        (
            tr!(
                "Player.log (1.6, последний запуск)",
                "Player.log (1.6, last run)",
                "Player.log（1.6，上次运行）"
            ),
            base.join("Player.log"),
        ),
        (
            tr!(
                "Player-prev.log (1.6, предыдущий запуск)",
                "Player-prev.log (1.6, previous run)",
                "Player-prev.log（1.6，再上一次运行）"
            ),
            base.join("Player-prev.log"),
        ),
    ];

    let mut found: Vec<(String, PathBuf, std::time::SystemTime)> = candidates
        .into_iter()
        .filter(|(_, p)| p.is_file())
        .map(|(label, p)| {
            let modified = fs::metadata(&p)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            (label.to_string(), p, modified)
        })
        .collect();

    found.sort_by_key(|f| std::cmp::Reverse(f.2));
    found.into_iter().map(|(label, p, _)| (label, p)).collect()
}

static WAYLITH_ENTRY_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+\s+([A-Z]+)\s+\[([^\]]*)\]\s?(.*)$").unwrap()
});

static WAYLITH_FRAME_RE: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"^\s+at\s+([^\s(]+)\(").unwrap());

/// Namespaces that belong to the engine, the runtime or the game itself — never a mod.
const ENGINE_FRAME_PREFIXES: &[&str] = &[
    "Brutal.",
    "Waylith.",
    "RimWorldWaylith.",
    "System.",
    "Microsoft.",
    "Verse.",
    "RimWorld.",
    "LudeonTK.",
    "HarmonyLib.",
    "UnityEngine.",
    "UnityStubs.",
    "MonoMod.",
    "Mono.",
];

/// A RimWorld 1.7+ log starts with "=== Waylith log ===" and uses "date time LEVEL [Category] text" lines.
pub fn is_waylith_log(lines: &[&str]) -> bool {
    lines
        .iter()
        .take(20)
        .any(|l| l.contains("=== Waylith log ===") || l.starts_with("Runtime:   .NET"))
        || lines.iter().take(200).filter(|l| WAYLITH_ENTRY_RE.is_match(l)).count() >= 3
}

/// One log entry of the 1.7 format: header line, message continuation lines and stack frames.
struct WaylithEntry<'a> {
    level: &'a str,
    category: &'a str,
    message: &'a str,
    first_line: usize,
    end_line: usize,
    frames: Vec<&'a str>,
}

fn parse_waylith_entries<'a>(lines: &[&'a str]) -> Vec<WaylithEntry<'a>> {
    let mut entries: Vec<WaylithEntry<'a>> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(caps) = WAYLITH_ENTRY_RE.captures(line) {
            if let Some(last) = entries.last_mut() {
                last.end_line = i;
            }
            entries.push(WaylithEntry {
                level: caps.get(1).map(|m| m.as_str()).unwrap_or(""),
                category: caps.get(2).map(|m| m.as_str()).unwrap_or(""),
                message: caps.get(3).map(|m| m.as_str()).unwrap_or(""),
                first_line: i,
                end_line: i + 1,
                frames: Vec::new(),
            });
        } else if let Some(last) = entries.last_mut() {
            last.end_line = i + 1;
            if let Some(caps) = WAYLITH_FRAME_RE.captures(line) {
                if let Some(m) = caps.get(1) {
                    last.frames.push(m.as_str());
                }
            }
        }
    }
    entries
}

/// "AchtungMod.Controller.HandleDrafting" -> "AchtungMod.Controller"
fn frame_class(frame: &str) -> &str {
    match frame.rfind('.') {
        Some(idx) if idx > 0 => &frame[..idx],
        _ => frame,
    }
}

fn is_engine_frame(frame: &str) -> bool {
    ENGINE_FRAME_PREFIXES.iter().any(|p| frame.starts_with(p))
}

fn analyze_waylith_entries<'a>(
    lines: &[&str],
    matcher: &ModMatcher<'a>,
    report: &mut LogReport,
    culprits_map: &mut HashMap<String, CrashCulprit>,
) {
    for entry in parse_waylith_entries(lines) {
        let is_fatal = matches!(entry.level, "FATAL" | "CRITICAL") || entry.message.contains("Unhandled exception");
        let is_error = is_fatal || entry.level == "ERROR";
        if !is_error {
            continue;
        }

        report.total_errors += 1;
        if is_fatal {
            report.has_hard_crash = true;
            if report.crash_excerpt.is_empty() {
                report.crash_excerpt = log_excerpt(lines, entry.first_line, entry.end_line.max(entry.first_line + 1));
            }
        }

        let culprit = entry
            .frames
            .iter()
            .filter(|f| !is_engine_frame(f))
            .find_map(|f| matcher.find_mod(frame_class(f)).map(|m| (*f, m)));

        let Some((frame, matched_mod)) = culprit else {
            continue;
        };

        let severity = if is_fatal {
            ErrorSeverity::HardCrash
        } else {
            ErrorSeverity::ModError
        };
        let excerpt_end = entry.end_line.min(entry.first_line + 16).min(lines.len());
        let excerpt = lines[entry.first_line..excerpt_end].join("\n");
        let message: String = entry.message.chars().take(200).collect();

        let key = matched_mod.package_id.to_lowercase();
        let item = culprits_map.entry(key).or_insert_with(|| CrashCulprit {
            mod_name: matched_mod.name.clone(),
            package_id: Some(matched_mod.package_id.clone()),
            mod_dir: Some(matched_mod.dir_path.clone()),
            severity,
            headline: format!("[{}] {}", entry.category, message),
            explanation: tr!(
                "Ошибка возникла в коде мода: `{}` (уровень {}).",
                "The error came from the mod's code: `{}` (level {}).",
                "错误来自模组代码：`{}`（级别 {}）。",
                frame,
                entry.level
            ),
            stack_trace_excerpt: excerpt,
            occurrences: 0,
        });
        item.occurrences += 1;
        if item.severity < severity {
            item.severity = severity;
        }
    }
}

pub fn analyze_rimworld_log(log_path: &Path, mods: &[ModInfo]) -> LogReport {
    let mut report = LogReport {
        log_path: log_path.to_path_buf(),
        log_file_name: log_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("Player.log")
            .to_string(),
        ..Default::default()
    };

    let metadata = match fs::metadata(log_path) {
        Ok(m) => m,
        Err(_) => {
            report.summary_text = tr!(
                "Не удалось прочитать файл лога.",
                "Could not read the log file.",
                "无法读取日志文件。"
            )
            .to_string();
            return report;
        }
    };
    report.file_size_bytes = metadata.len();

    let content = match fs::read_to_string(log_path) {
        Ok(c) => c,
        Err(_) => {
            let bytes = fs::read(log_path).unwrap_or_default();
            String::from_utf8_lossy(&bytes).to_string()
        }
    };

    report.raw_content = content.clone();
    let lines: Vec<&str> = content.lines().collect();
    report.total_lines = lines.len();

    let lower_content = content.to_lowercase();
    let has_gpu_reset = lower_content.contains("887a0005")
        || lower_content.contains("device_removed")
        || lower_content.contains("device_reset");
    let has_oom = lower_content.contains("could not allocate memory")
        || lower_content.contains("out of memory")
        || lower_content.contains("outofmemoryexception");

    if has_gpu_reset {
        report.system_issue = Some(SystemCrashIssue::GpuReset);
        report.has_hard_crash = true;
    } else if has_oom {
        report.system_issue = Some(SystemCrashIssue::OutOfMemory);
        report.has_hard_crash = true;
    }

    let matcher = ModMatcher::build(mods);
    let mut culprits_map: HashMap<String, CrashCulprit> = HashMap::new();

    report.is_waylith = is_waylith_log(&lines);
    // In the 1.7 format errors are counted per log entry, not by keywords in lines.
    let count_keyword_errors = !report.is_waylith;
    if report.is_waylith {
        analyze_waylith_entries(&lines, &matcher, &mut report, &mut culprits_map);
    }

    let re_mono_call = Regex::new(r"\(Mono JIT Code\)\s+([A-Za-z0-9_\.]+):([A-Za-z0-9_]+)").ok();
    let re_mono_at = Regex::new(r"^\s*at\s+([A-Za-z0-9_\.]+)\.([A-Za-z0-9_<>]+)").ok();
    let re_prepatcher = Regex::new(r"Prepatcher Error:.*?static\s+.*?\s+([A-Za-z0-9_\.]+)::").ok();
    let re_dependency = Regex::new(r"Mod\s+(.*?)\s+dependency\s+\((.*?)\)").ok();
    let re_config_error = Regex::new(r"Exception in ConfigErrors\(\) of ([A-Za-z0-9_]+):").ok();
    let re_instantiate =
        Regex::new(r"(?:Error while instantiating class|Exception creating instance of)\s+([A-Za-z0-9_\.]+)").ok();
    // The game writes both "Could not resolve cross-reference: No Verse.SoundDef named X found
    // to give to Some.Class ..." and "Could not resolve cross-reference to Verse.ThingDef named X
    // (wanter=costList)".
    let re_cross_ref = Regex::new(
        r"Could not resolve cross-reference(?::\s*No|\s+to)\s+([A-Za-z0-9_\.]+)\s+named\s+(\S+?)(?:\s+found to give to\s+([A-Za-z0-9_\.]+))?(?:\s|$)",
    )
    .ok();
    let re_texture_err = Regex::new(r"(?:Could not load UnityEngine\.Texture2D at|Failed to load texture at|Could not find a texture named)\s+([^\s]+)").ok();
    let re_harmony_conflict = Regex::new(
        r"(?:Method not found:\s+'HarmonyLib|MissingMethodException:.*?HarmonyLib|Could not load type 'HarmonyLib)",
    )
    .ok();

    let mut crash_marker_idx = None;
    // Unity/Mono crash markers only exist in 1.6 and older logs.
    let skip_unity_markers = report.is_waylith;
    for (i, line) in lines.iter().enumerate().rev().filter(|_| !skip_unity_markers) {
        if line.contains("END OF STACKTRACE")
            || line.contains("A crash has been intercepted by the crash handler")
            || line.contains("Crash report generated in:")
        {
            crash_marker_idx = Some(i);
            report.has_hard_crash = true;
            break;
        }
    }

    if let Some(c_idx) = crash_marker_idx {
        // Unity's crash block starts with "Crash!!!" and goes on with hundreds of lines of
        // loaded DLLs: what led to the crash is right before "Crash!!!".
        let block_start = (c_idx.saturating_sub(600)..=c_idx)
            .rev()
            .find(|&i| lines[i].trim_start().starts_with("Crash!!!"))
            .unwrap_or(c_idx);
        report.crash_excerpt = log_excerpt(&lines, block_start.saturating_sub(EXCERPT_LINES), block_start + 1);
        let start = c_idx.saturating_sub(80);
        let stack_slice = &lines[start..=c_idx];
        let stack_text = stack_slice.join("\n");

        let mut found_culprit = false;

        for line in stack_slice {
            if let Some(ref re) = re_mono_call {
                if let Some(caps) = re.captures(line) {
                    let full_class = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let method = caps.get(2).map(|m| m.as_str()).unwrap_or("");

                    if full_class.starts_with("UnityEngine")
                        || full_class.starts_with("System.")
                        || full_class == "Verse.Root"
                        || full_class == "Verse.UIRoot_Entry"
                        || full_class == "RimWorld.MainMenuDrawer"
                        || full_class.starts_with("MonoMod.")
                    {
                        continue;
                    }

                    if let Some(matched_mod) = matcher.find_mod(full_class) {
                        let key = matched_mod.package_id.to_lowercase();
                        let excerpt = stack_slice.iter().take(25).cloned().collect::<Vec<_>>().join("\n");

                        culprits_map.insert(
                            key,
                            CrashCulprit {
                                mod_name: matched_mod.name.clone(),
                                package_id: Some(matched_mod.package_id.clone()),
                                mod_dir: Some(matched_mod.dir_path.clone()),
                                severity: ErrorSeverity::HardCrash,
                                headline: tr!(
                                    "Вызов метода `{}` привёл к вылету игры (CTD)",
                                    "Calling `{}` crashed the game (CTD)",
                                    "调用 `{}` 导致游戏崩溃 (CTD)",
                                    method
                                ),
                                explanation: tr!(
                                    "Метод `{}:{}` выполнялся прямо перед сбоем движка Unity.",
                                    "`{}:{}` was running right before the Unity engine crashed.",
                                    "Unity 引擎崩溃前正在运行 `{}:{}`。",
                                    full_class,
                                    method
                                ),
                                stack_trace_excerpt: excerpt,
                                occurrences: 1,
                            },
                        );
                        found_culprit = true;
                        break;
                    }
                }
            }
        }

        if !found_culprit {
            for line in stack_slice {
                if let Some(ref re) = re_mono_at {
                    if let Some(caps) = re.captures(line) {
                        let full_class = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                        let method = caps.get(2).map(|m| m.as_str()).unwrap_or("");

                        if full_class.starts_with("UnityEngine")
                            || full_class.starts_with("System.")
                            || full_class.starts_with("Verse.")
                            || full_class.starts_with("RimWorld.")
                        {
                            continue;
                        }

                        if let Some(matched_mod) = matcher.find_mod(full_class) {
                            let key = matched_mod.package_id.to_lowercase();
                            culprits_map.insert(
                                key,
                                CrashCulprit {
                                    mod_name: matched_mod.name.clone(),
                                    package_id: Some(matched_mod.package_id.clone()),
                                    mod_dir: Some(matched_mod.dir_path.clone()),
                                    severity: ErrorSeverity::HardCrash,
                                    headline: tr!(
                                        "Вылет игры в моде `{}`",
                                        "Game crash in mod `{}`",
                                        "模组 `{}` 导致游戏崩溃",
                                        matched_mod.name
                                    ),
                                    explanation: tr!(
                                        "Исключение в методе `{}:{}` завершило процесс игры.",
                                        "An exception in `{}:{}` ended the game process.",
                                        "`{}:{}` 中的异常终止了游戏进程。",
                                        full_class,
                                        method
                                    ),
                                    stack_trace_excerpt: stack_text.clone(),
                                    occurrences: 1,
                                },
                            );
                            break;
                        }
                    }
                }
            }
        }
    }

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        if line.contains("Prepatcher Error:") {
            if count_keyword_errors {
                report.total_errors += 1;
            }
            let mut excerpt_lines = vec![line];
            for k in 1..=6 {
                if i + k < lines.len() {
                    excerpt_lines.push(lines[i + k]);
                }
            }
            let excerpt = excerpt_lines.join("\n");

            if let Some(ref re) = re_prepatcher {
                if let Some(caps) = re.captures(line) {
                    let patch_class = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    if let Some(matched_mod) = matcher.find_mod(patch_class) {
                        let key = matched_mod.package_id.to_lowercase();
                        let entry = culprits_map.entry(key).or_insert_with(|| CrashCulprit {
                            mod_name: matched_mod.name.clone(),
                            package_id: Some(matched_mod.package_id.clone()),
                            mod_dir: Some(matched_mod.dir_path.clone()),
                            severity: ErrorSeverity::PrepatcherHarmonyError,
                            headline: tr!(
                                "Ошибка Prepatcher в моде `{}`",
                                "Prepatcher error in mod `{}`", "模组 `{}` 的 Prepatcher 错误",
                                matched_mod.name
                            ),
                            explanation: tr!(
                                "Патч `{}` не смог встроиться в сборку. Это часто даёт чёрный экран или зависание загрузки.",
                                "Patch `{}` could not be applied to the assembly. This often causes a black screen or a stuck loading screen.", "补丁 `{}` 无法应用到程序集。这常常导致黑屏或卡在加载界面。",
                                patch_class
                            ),
                            stack_trace_excerpt: excerpt.clone(),
                            occurrences: 0,
                        });
                        entry.occurrences += 1;
                        if entry.severity < ErrorSeverity::PrepatcherHarmonyError {
                            entry.severity = ErrorSeverity::PrepatcherHarmonyError;
                        }
                    }
                }
            }
        }

        if line.contains("dependency (") && line.contains("needs to have") {
            if count_keyword_errors {
                report.total_errors += 1;
            }
            if let Some(ref re) = re_dependency {
                if let Some(caps) = re.captures(line) {
                    let mod_name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let dep_id = caps.get(2).map(|m| m.as_str()).unwrap_or("");

                    let matched_mod = matcher.find_mod(mod_name);
                    let target_name = matched_mod
                        .map(|m| m.name.clone())
                        .unwrap_or_else(|| mod_name.to_string());
                    let key = matched_mod
                        .map(|m| m.package_id.to_lowercase())
                        .unwrap_or_else(|| mod_name.to_lowercase());

                    let entry = culprits_map.entry(key).or_insert_with(|| CrashCulprit {
                        mod_name: target_name.clone(),
                        package_id: matched_mod.map(|m| m.package_id.clone()),
                        mod_dir: matched_mod.map(|m| m.dir_path.clone()),
                        severity: ErrorSeverity::MissingDependency,
                        headline: tr!(
                            "Нет или не загружена зависимость `{}`",
                            "Dependency `{}` is missing or not loaded",
                            "前置 `{}` 缺失或未加载",
                            dep_id
                        ),
                        explanation: tr!(
                            "Мод `{}` требует включённый мод с Package ID `{}`.",
                            "Mod `{}` needs an enabled mod with Package ID `{}`.",
                            "模组 `{}` 需要启用 Package ID 为 `{}` 的模组。",
                            target_name,
                            dep_id
                        ),
                        stack_trace_excerpt: line.to_string(),
                        occurrences: 0,
                    });
                    entry.occurrences += 1;
                }
            }
        }

        if line.contains("Exception in ConfigErrors()") {
            if count_keyword_errors {
                report.total_errors += 1;
            }
            let mut excerpt_lines = vec![line];
            for k in 1..=6 {
                if i + k < lines.len() {
                    excerpt_lines.push(lines[i + k]);
                }
            }
            let excerpt = excerpt_lines.join("\n");

            if let Some(ref re) = re_config_error {
                if let Some(caps) = re.captures(line) {
                    let def_name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let matched_mod = matcher.find_mod(def_name);
                    if let Some(m) = matched_mod {
                        let key = m.package_id.to_lowercase();
                        let entry = culprits_map.entry(key).or_insert_with(|| CrashCulprit {
                            mod_name: m.name.clone(),
                            package_id: Some(m.package_id.clone()),
                            mod_dir: Some(m.dir_path.clone()),
                            severity: ErrorSeverity::DefException,
                            headline: tr!(
                                "Ошибка DefDatabase / NullReference в объекте `{}`",
                                "DefDatabase / NullReference error in `{}`",
                                "`{}` 中的 DefDatabase / NullReference 错误",
                                def_name
                            ),
                            explanation: tr!(
                                "Ошибка в настройках объекта `{}` в моде `{}`.",
                                "Broken definition of `{}` in mod `{}`.",
                                "模组 `{1}` 中 `{0}` 的定义有误。",
                                def_name,
                                m.name
                            ),
                            stack_trace_excerpt: excerpt.clone(),
                            occurrences: 0,
                        });
                        entry.occurrences += 1;
                    }
                }
            }
        }

        if line.contains("Error while instantiating") || line.contains("Exception creating instance of") {
            if count_keyword_errors {
                report.total_errors += 1;
            }
            if let Some(ref re) = re_instantiate {
                if let Some(caps) = re.captures(line) {
                    let class_name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    if let Some(matched_mod) = matcher.find_mod(class_name) {
                        let key = matched_mod.package_id.to_lowercase();
                        let entry = culprits_map.entry(key).or_insert_with(|| CrashCulprit {
                            mod_name: matched_mod.name.clone(),
                            package_id: Some(matched_mod.package_id.clone()),
                            mod_dir: Some(matched_mod.dir_path.clone()),
                            severity: ErrorSeverity::InstantiationError,
                            headline: tr!(
                                "Ошибка создания экземпляра класса `{}`",
                                "Error while instantiating class `{}`", "创建类 `{}` 的实例时出错",
                                class_name
                            ),
                            explanation: tr!(
                                "Движок не смог создать объект класса. Проверьте совместимость мода с версией игры.",
                                "The engine could not instantiate the class. Check mod compatibility with your game version.", "引擎无法创建该类的实例。请检查模组是否与你的游戏版本兼容。"
                            )
                            .to_string(),
                            stack_trace_excerpt: line.to_string(),
                            occurrences: 0,
                        });
                        entry.occurrences += 1;
                        if entry.severity < ErrorSeverity::InstantiationError {
                            entry.severity = ErrorSeverity::InstantiationError;
                        }
                    }
                }
            }
        }

        if line.contains("Could not resolve cross-reference") {
            if count_keyword_errors {
                report.total_errors += 1;
            }
            if let Some(ref re) = re_cross_ref {
                if let Some(caps) = re.captures(line) {
                    let def_type = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let def_type = def_type.rsplit('.').next().unwrap_or(def_type);
                    let def_name = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                    let wanter = caps.get(3).map(|m| m.as_str());
                    // The class that wants the def may belong to a mod; the def name rarely tells.
                    let matched_mod = wanter
                        .and_then(|w| matcher.find_mod(w))
                        .or_else(|| matcher.find_mod(def_name));
                    let key = matched_mod
                        .map(|m| m.package_id.to_lowercase())
                        .unwrap_or_else(|| format!("xref:{}", def_name.to_lowercase()));
                    let t = |k: TranslationKey| crate::i18n::text(k);

                    let entry = culprits_map.entry(key).or_insert_with(|| CrashCulprit {
                        mod_name: matched_mod
                            .map(|m| m.name.clone())
                            .unwrap_or_else(|| format!("{def_type} {def_name}")),
                        package_id: matched_mod.map(|m| m.package_id.clone()),
                        mod_dir: matched_mod.map(|m| m.dir_path.clone()),
                        severity: ErrorSeverity::CrossReferenceError,
                        headline: crate::i18n::fmt(t(TranslationKey::XrefHeadline), &[def_type, def_name]),
                        explanation: crate::i18n::fmt(t(TranslationKey::XrefExplanation), &[wanter.unwrap_or("?")]),
                        stack_trace_excerpt: line.to_string(),
                        occurrences: 0,
                    });
                    entry.occurrences += 1;
                    if entry.severity < ErrorSeverity::CrossReferenceError {
                        entry.severity = ErrorSeverity::CrossReferenceError;
                    }
                }
            }
        }

        if line.contains("Could not load UnityEngine.Texture2D")
            || line.contains("Failed to load texture at")
            || line.contains("Could not find a texture named")
        {
            if count_keyword_errors {
                report.total_errors += 1;
            }
            if let Some(ref re) = re_texture_err {
                if let Some(caps) = re.captures(line) {
                    let path = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let matched_mod = matcher.find_mod(path);
                    let mod_name = matched_mod
                        .as_ref()
                        .map(|m| m.name.clone())
                        .unwrap_or_else(|| "Textures".to_string());
                    let key = matched_mod
                        .as_ref()
                        .map(|m| m.package_id.to_lowercase())
                        .unwrap_or_else(|| "texture_error".to_string());

                    let entry = culprits_map.entry(key).or_insert_with(|| CrashCulprit {
                        mod_name,
                        package_id: matched_mod.as_ref().map(|m| m.package_id.clone()),
                        mod_dir: matched_mod.as_ref().map(|m| m.dir_path.clone()),
                        severity: ErrorSeverity::BrokenTexture,
                        headline: tr!(
                            "Не найдена или повреждена текстура `{}`",
                            "Missing or broken texture `{}`", "纹理 `{}` 缺失或损坏",
                            path
                        ),
                        explanation: tr!(
                            "Игра не смогла загрузить текстуру: файл повреждён, отсутствует или сохранён в неподдерживаемом формате.",
                            "The game failed to load the texture: file is missing, corrupt, or in an unsupported format.", "游戏无法加载该纹理：文件缺失、损坏或格式不受支持。"
                        )
                        .to_string(),
                        stack_trace_excerpt: line.to_string(),
                        occurrences: 0,
                    });
                    entry.occurrences += 1;
                    if entry.severity < ErrorSeverity::BrokenTexture {
                        entry.severity = ErrorSeverity::BrokenTexture;
                    }
                }
            }
        }

        if re_harmony_conflict.as_ref().is_some_and(|re| re.is_match(line)) {
            if count_keyword_errors {
                report.total_errors += 1;
            }
            let key = "brrainz.harmony".to_string();
            let entry = culprits_map.entry(key).or_insert_with(|| CrashCulprit {
                mod_name: crate::i18n::text(TranslationKey::HarmonyConflictName).to_string(),
                package_id: Some("brrainz.harmony".to_string()),
                mod_dir: None,
                severity: ErrorSeverity::PrepatcherHarmonyError,
                headline: tr!("Конфликт версий Harmony", "Harmony version conflict", "Harmony 版本冲突").to_string(),
                explanation: tr!(
                    "Один из модов загружает устаревшую версию 0Harmony.dll, что приводит к сбоям вызова методов Harmony.",
                    "A mod is loading an outdated 0Harmony.dll, causing method call failures in Harmony.", "某个模组加载了过时的 0Harmony.dll，导致 Harmony 方法调用失败。"
                )
                .to_string(),
                stack_trace_excerpt: line.to_string(),
                occurrences: 0,
            });
            entry.occurrences += 1;
        }

        if count_keyword_errors && (line.contains("Exception:") || line.contains("Error in ")) {
            report.total_errors += 1;
        }

        i += 1;
    }

    let mut culprits: Vec<CrashCulprit> = culprits_map.into_values().collect();
    culprits.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| b.occurrences.cmp(&a.occurrences))
    });

    report.culprits = culprits;
    report.recent_errors = recent_errors(&lines, report.is_waylith);

    report.summary_text = summary_for(&report, crate::i18n::current_language());
    report
}

/// The log itself for "Copy for AI": the lines before the crash, and the last errors when
/// no mod could be blamed (otherwise the request would carry no data at all).
fn push_log_data(out: &mut String, report: &LogReport, lang: crate::i18n::Language) {
    use crate::i18n::TranslationKey;
    let text = |key: TranslationKey| key.translate(lang);
    out.push_str(&crate::i18n::fmt(
        text(TranslationKey::AiErrorsInLog),
        &[&report.total_errors.to_string()],
    ));
    out.push('\n');
    // Well-known causes, named outright.
    let excerpt = report.crash_excerpt.to_lowercase();
    if excerpt.contains("887a0005") || excerpt.contains("device_removed") {
        out.push_str(text(TranslationKey::AiHintGpuReset));
        out.push('\n');
    }
    if excerpt.contains("could not allocate memory") || excerpt.contains("out of memory") {
        out.push_str(text(TranslationKey::AiHintOutOfMemory));
        out.push('\n');
    }
    if !report.crash_excerpt.is_empty() {
        out.push('\n');
        out.push_str(text(TranslationKey::AiCrashExcerpt));
        out.push_str("\n```text\n");
        out.push_str(&report.crash_excerpt);
        out.push_str("\n```\n");
    }
    if report.culprits.is_empty() && !report.recent_errors.is_empty() {
        out.push('\n');
        out.push_str(text(TranslationKey::AiRecentErrors));
        out.push('\n');
        for e in &report.recent_errors {
            out.push_str("```text\n");
            out.push_str(e);
            out.push_str("\n```\n");
        }
    }
    out.push('\n');
}

pub fn generate_ai_diagnostic_prompt(report: &LogReport, is_ru: bool) -> String {
    let mut out = String::new();

    if is_ru {
        out.push_str("Помоги разобраться с ошибкой или вылетом RimWorld. Ниже данные из Player.log.\n\n");
        out.push_str(&format!("Файл лога: {}\n", report.log_file_name));
        out.push_str(&format!("Итог анализа: {}\n", summary_for(report, Language::Russian)));

        out.push_str(if report.is_waylith {
            "Версия игры: RimWorld 1.7+ (движок Waylith, .NET 10)\n"
        } else {
            "Версия игры: RimWorld 1.6 или старше (движок Unity, Mono)\n"
        });

        if report.has_hard_crash {
            out.push_str(if report.is_waylith {
                "Игра вылетела на рабочий стол.\n"
            } else {
                "Игра вылетела на рабочий стол (сбой движка Unity).\n"
            });
        }

        if !report.culprits.is_empty() {
            out.push_str("\nМоды, на которые указывает лог:\n\n");
            for (idx, c) in report.culprits.iter().enumerate() {
                out.push_str(&format!(
                    "{}. {} ({})\n",
                    idx + 1,
                    c.mod_name,
                    c.package_id.as_deref().unwrap_or("неизвестен")
                ));
                out.push_str(&format!("   Тип: {}\n", c.severity.badge_text(Language::Russian)));
                out.push_str(&format!("   Ошибка: {}\n", c.headline));
                out.push_str(&format!("   Подробности: {}\n", c.explanation));

                if !c.stack_trace_excerpt.is_empty() {
                    out.push_str("   Стек вызовов:\n   ```text\n");
                    for line in c.stack_trace_excerpt.lines().take(20) {
                        out.push_str(&format!("     {}\n", line));
                    }
                    out.push_str("   ```\n");
                }
                out.push('\n');
            }
        }

        push_log_data(&mut out, report, crate::i18n::Language::Russian);
        out.push_str("Вопросы:\n");
        out.push_str("1. В чём причина сбоя?\n");
        out.push_str("2. Какой мод виноват и с чем он конфликтует?\n");
        out.push_str("3. Что сделать: отключить мод, поменять порядок загрузки, обновить мод или поставить патч совместимости?\n");
    } else {
        out.push_str("Help me find the cause of a RimWorld error or crash. Data from Player.log is below.\n\n");
        out.push_str(&format!("Log file: {}\n", report.log_file_name));
        out.push_str(&format!(
            "Analysis result: {}\n",
            summary_for(report, Language::English)
        ));

        out.push_str(if report.is_waylith {
            "Game version: RimWorld 1.7+ (Waylith engine, .NET 10)\n"
        } else {
            "Game version: RimWorld 1.6 or older (Unity engine, Mono)\n"
        });

        if report.has_hard_crash {
            out.push_str(if report.is_waylith {
                "The game crashed to desktop.\n"
            } else {
                "The game crashed to desktop (Unity engine fault).\n"
            });
        }

        if !report.culprits.is_empty() {
            out.push_str("\nMods the log points to:\n\n");
            for (idx, c) in report.culprits.iter().enumerate() {
                out.push_str(&format!(
                    "{}. {} ({})\n",
                    idx + 1,
                    c.mod_name,
                    c.package_id.as_deref().unwrap_or("unknown")
                ));
                out.push_str(&format!("   Type: {}\n", c.severity.badge_text(Language::English)));
                out.push_str(&format!("   Error: {}\n", c.headline));
                out.push_str(&format!("   Details: {}\n", c.explanation));

                if !c.stack_trace_excerpt.is_empty() {
                    out.push_str("   Stack trace:\n   ```text\n");
                    for line in c.stack_trace_excerpt.lines().take(20) {
                        out.push_str(&format!("     {}\n", line));
                    }
                    out.push_str("   ```\n");
                }
                out.push('\n');
            }
        }

        push_log_data(&mut out, report, crate::i18n::Language::English);
        out.push_str("Questions:\n");
        out.push_str("1. What caused the failure?\n");
        out.push_str("2. Which mod is at fault and what does it conflict with?\n");
        out.push_str(
            "3. What should I do: disable a mod, change the load order, update, or add a compatibility patch?\n",
        );
    }

    out
}

/// Graphics card / memory failure: title, description and advice in `lang`.
fn system_issue_texts(issue: SystemCrashIssue, lang: Language) -> (&'static str, &'static str, Vec<&'static str>) {
    let keys = match issue {
        SystemCrashIssue::GpuReset => (
            TranslationKey::CrashGpuResetTitle,
            TranslationKey::CrashGpuResetDesc,
            vec![
                TranslationKey::CrashGpuAdviceOptimizer,
                TranslationKey::CrashGpuAdviceHighQuality,
                TranslationKey::CrashGpuAdviceDriver,
                TranslationKey::CrashGpuAdviceEventLog,
            ],
        ),
        SystemCrashIssue::OutOfMemory => (
            TranslationKey::CrashOomTitle,
            TranslationKey::CrashOomDesc,
            vec![
                TranslationKey::CrashOomAdviceCloseApps,
                TranslationKey::CrashOomAdvicePagingFile,
                TranslationKey::CrashOomAdviceReduceMods,
            ],
        ),
    };
    (
        keys.0.translate(lang),
        keys.1.translate(lang),
        keys.2.iter().map(|k| k.translate(lang)).collect(),
    )
}

fn engine_label(report: &LogReport) -> &'static str {
    if report.is_waylith {
        "RimWorld 1.7+ (Waylith)"
    } else {
        "RimWorld 1.6 (Unity)"
    }
}

/// Generates a plain-text diagnostic report in the interface language (F13).
pub fn generate_text_report(report: &LogReport) -> String {
    let lang = crate::i18n::current_language();
    let t = |k: TranslationKey| k.translate(lang);
    let mut out = String::new();
    let sep = "=".repeat(78);
    let subsep = "-".repeat(78);

    out.push_str(&sep);
    out.push('\n');
    out.push_str(&format!("RimWorld Mod Manager — {}\n", t(TranslationKey::ReportTitle)));
    out.push_str(&format!(
        "{}: {}\n",
        t(TranslationKey::ReportFile),
        report.log_path.display()
    ));
    out.push_str(&format!(
        "{}: {} | {}: {} | {}: {}\n",
        t(TranslationKey::ReportLines),
        report.total_lines,
        t(TranslationKey::ReportSize),
        crate::i18n::fmt(t(TranslationKey::ReportBytes), &[&report.file_size_bytes.to_string()]),
        t(TranslationKey::ReportErrors),
        report.total_errors
    ));
    out.push_str(&format!(
        "{}: {}\n",
        t(TranslationKey::ReportEngine),
        engine_label(report)
    ));
    if let Some(load_time) = report.load_time_secs {
        out.push_str(&format!(
            "{}: {}\n",
            t(TranslationKey::LoadTimeTitle),
            crate::load_time::format_duration_in(load_time, lang)
        ));
    }
    out.push_str(&sep);
    out.push_str("\n\n");

    out.push_str(&format!("[{}]\n", t(TranslationKey::ReportSummary)));
    out.push_str(&summary_for(report, lang));
    out.push_str("\n\n");

    if let Some(issue) = report.system_issue {
        let (title, desc, advice) = system_issue_texts(issue, lang);
        out.push_str(&subsep);
        out.push('\n');
        out.push_str(&format!("{title}\n{desc}\n"));
        for a in advice {
            out.push_str(&format!("• {a}\n"));
        }
        out.push('\n');
    }

    if !report.culprits.is_empty() {
        out.push_str(&subsep);
        out.push('\n');
        out.push_str(&format!("{}:\n\n", t(TranslationKey::ReportCulprits)));
        for (i, c) in report.culprits.iter().enumerate() {
            out.push_str(&format!(
                "{}. {} [{}] ({})\n",
                i + 1,
                c.mod_name,
                c.package_id.as_deref().unwrap_or("—"),
                crate::i18n::fmt(t(TranslationKey::ReportOccurrences), &[&c.occurrences.to_string()])
            ));
            out.push_str(&format!(
                "   {}: {}\n",
                t(TranslationKey::ReportStatus),
                c.severity.badge_text(lang)
            ));
            out.push_str(&format!("   {}: {}\n", t(TranslationKey::ReportCause), c.headline));
            out.push_str(&format!(
                "   {}: {}\n",
                t(TranslationKey::ReportExplanation),
                c.explanation
            ));
            if !c.stack_trace_excerpt.is_empty() {
                out.push_str(&format!("   {}:\n", t(TranslationKey::ReportStack)));
                for l in c.stack_trace_excerpt.lines().take(15) {
                    out.push_str(&format!("     {}\n", l));
                }
            }
            out.push('\n');
        }
    }

    if !report.recent_errors.is_empty() {
        out.push_str(&subsep);
        out.push('\n');
        out.push_str(&format!("{}:\n\n", t(TranslationKey::ReportRecentErrors)));
        for (idx, err) in report.recent_errors.iter().enumerate() {
            out.push_str(&format!("--- [#{}] ---\n{}\n\n", idx + 1, err));
        }
    }

    out
}

/// Generates a standalone, dark-themed HTML diagnostic report in the interface language (F13).
pub fn generate_html_report(report: &LogReport) -> String {
    let lang = crate::i18n::current_language();
    let t = |k: TranslationKey| k.translate(lang);
    let title = format!("RimWorld Mod Manager — {}", t(TranslationKey::ReportTitle));

    let summary = summary_for(report, lang);
    let mut culprits_html = String::new();
    for (i, c) in report.culprits.iter().enumerate() {
        let (bg, fg) = c.severity.badge_colors();
        let stack = if c.stack_trace_excerpt.is_empty() {
            String::new()
        } else {
            format!("<pre class=\"stack\">{}</pre>", html_escape(&c.stack_trace_excerpt))
        };
        culprits_html.push_str(&format!(
            "<div class=\"card culprit\">\
                <div class=\"culprit-header\">\
                    <span class=\"num\">#{}.</span> \
                    <span class=\"mod-name\">{}</span> \
                    <span class=\"pkg-id\">({})</span> \
                    <span class=\"badge\" style=\"background:rgb({},{},{});color:rgb({},{},{});\">{}</span>\
                </div>\
                <div class=\"headline\">{}</div>\
                <div class=\"explanation\">{}</div>\
                {}\
            </div>",
            i + 1,
            html_escape(&c.mod_name),
            html_escape(c.package_id.as_deref().unwrap_or("—")),
            bg[0],
            bg[1],
            bg[2],
            fg[0],
            fg[1],
            fg[2],
            html_escape(c.severity.badge_text(lang)),
            html_escape(&c.headline),
            html_escape(&c.explanation),
            stack
        ));
    }

    let mut system_html = String::new();
    if let Some(issue) = report.system_issue {
        let (issue_title, issue_desc, advice) = system_issue_texts(issue, lang);
        let items: String = advice.iter().map(|a| format!("<li>{}</li>", html_escape(a))).collect();
        system_html = format!(
            "<div class=\"card system-issue\">\
                <h3>⚠️ {}</h3>\
                <p>{}</p>\
                <ul>{}</ul>\
            </div>",
            html_escape(issue_title),
            html_escape(issue_desc),
            items
        );
    }

    let mut errors_html = String::new();
    for err in &report.recent_errors {
        errors_html.push_str(&format!("<pre class=\"log-block\">{}</pre>", html_escape(err)));
    }

    let load_time_meta = match report.load_time_secs {
        Some(secs) => format!(
            " | {}: {}",
            t(TranslationKey::LoadTimeTitle),
            crate::load_time::format_duration_in(secs, lang)
        ),
        None => String::new(),
    };
    let html_lang = match lang {
        Language::Russian => "ru",
        Language::English => "en",
        Language::Chinese => "zh-CN",
    };
    let section = |key: TranslationKey, body: &str| {
        if body.is_empty() {
            String::new()
        } else {
            format!("<h2>{}</h2>{}", t(key), body)
        }
    };

    format!(
        "<!DOCTYPE html>\n\
        <html lang=\"{}\">\n\
        <head>\n\
            <meta charset=\"UTF-8\">\n\
            <title>{}</title>\n\
            <style>\n\
                body {{ background: #0F1117; color: #E2E8F0; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Microsoft YaHei', sans-serif; margin: 0; padding: 24px; line-height: 1.5; }}\n\
                .container {{ max-width: 900px; margin: 0 auto; }}\n\
                h1 {{ font-size: 22px; color: #FFFFFF; margin-bottom: 6px; }}\n\
                .meta {{ font-size: 13px; color: #94A3B8; margin-bottom: 20px; }}\n\
                .card {{ background: #1A1D27; border: 1px solid #282C3D; border-radius: 8px; padding: 16px; margin-bottom: 14px; }}\n\
                .summary {{ font-size: 15px; font-weight: bold; color: #6EE7B7; }}\n\
                .system-issue {{ border-color: #F47F7F; background: rgba(244, 127, 127, 0.08); }}\n\
                .system-issue h3 {{ margin: 0 0 6px 0; color: #F47F7F; font-size: 16px; }}\n\
                .culprit-header {{ display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }}\n\
                .mod-name {{ font-weight: bold; color: #FFFFFF; font-size: 16px; }}\n\
                .pkg-id {{ color: #94A3B8; font-size: 12px; }}\n\
                .badge {{ border-radius: 4px; padding: 2px 8px; font-size: 11px; font-weight: 600; }}\n\
                .headline {{ font-weight: 600; color: #CBD5E1; margin-bottom: 4px; }}\n\
                .explanation {{ font-size: 13px; color: #94A3B8; margin-bottom: 8px; }}\n\
                pre {{ background: #0A0C10; padding: 12px; border-radius: 6px; overflow-x: auto; font-size: 12px; font-family: Consolas, monospace; color: #FCA5A5; border: 1px solid #222634; }}\n\
                pre.log-block {{ color: #CBD5E1; }}\n\
            </style>\n\
        </head>\n\
        <body>\n\
            <div class=\"container\">\n\
                <h1>{}</h1>\n\
                <div class=\"meta\">{}: {} | {} | {}: {}{}</div>\n\
                <div class=\"card\">\n\
                    <div class=\"summary\">{}</div>\n\
                </div>\n\
                {}\
                {}\
                {}\
            </div>\n\
        </body>\n\
        </html>",
        html_lang,
        html_escape(&title),
        html_escape(&title),
        t(TranslationKey::ReportFile),
        html_escape(&report.log_file_name),
        engine_label(report),
        t(TranslationKey::ReportLines),
        report.total_lines,
        load_time_meta,
        html_escape(&summary),
        system_html,
        section(TranslationKey::ReportCulprits, &culprits_html),
        section(TranslationKey::ReportRecentErrors, &errors_html)
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ModInfo, ModOrigin};

    fn test_mod(dir: &Path, id: &str, name: &str) -> ModInfo {
        ModInfo {
            package_id: id.to_string(),
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            author: String::new(),
            description: String::new(),
            dir_path: dir.to_path_buf(),
            preview_path: None,
            supported_versions: vec![],
            load_after: vec![],
            force_load_after: vec![],
            load_before: vec![],
            force_load_before: vec![],
            dependencies: vec![],
            origin: ModOrigin::Workshop,
            languages: vec![],
            is_translation_mod: false,
            parent_package_id: None,
            steam_item_id: None,
            has_defs: true,
            has_assemblies: true,
            has_other_content: true,
            ..Default::default()
        }
    }

    const WAYLITH_LOG: &str = "=== Waylith log ===
Started:   2026-10-24 12:17:36.522 +02:00
Version:   1.7.4985.1269
Runtime:   .NET 10.0.13-waylithclr.dev

2026-10-24 12:17:41.695  INFO [Verse] Harmony Mod v3.0.1.0; Harmony library v3.0.0.0-rc.3
    at Brutal.Logging.LogCategory.Info(LogString`1 message) [0x0]
    at Verse.Log.Message(String text) [0x10]
2026-10-24 12:18:20.001  ERROR [Verse] Exception ticking Pawn: System.NullReferenceException: Object reference not set
    at Brutal.Logging.LogCategory.Error(LogString`1 message) [0x0]
    at Verse.Log.Error(String text) [0x10]
    at AchtungMod.Controller.HandleDrafting(Pawn pawn) [0x2A]
    at Verse.Pawn.Tick() [0x55]
2026-10-24 12:18:21.001  WARN [Verse] Something harmless
    at Verse.Log.Warning(String text) [0x10]
2026-10-24 12:18:22.001  ERROR [Verse] Engine-only error
    at Waylith.Graphics.GraphicsDevice.Present() [0x10]
";

    #[test]
    fn detects_waylith_format() {
        let lines: Vec<&str> = WAYLITH_LOG.lines().collect();
        assert!(is_waylith_log(&lines));
        let unity = [
            "Mono path[0] = 'C:/Games/RimWorld/RimWorldWin64_Data/Managed'",
            "Initialize engine version: 2022.3.35f1",
        ];
        assert!(!is_waylith_log(&unity));
    }

    #[test]
    fn finds_mod_culprit_in_waylith_log() {
        let base = std::env::temp_dir().join(format!("rwmm_log_test_{}", std::process::id()));
        let mod_dir = base.join("730936602");
        std::fs::create_dir_all(mod_dir.join("Assemblies")).unwrap();
        std::fs::write(mod_dir.join("Assemblies").join("AchtungMod.dll"), b"").unwrap();
        let log_path = base.join("Player.log");
        std::fs::write(&log_path, WAYLITH_LOG).unwrap();

        let mods = vec![test_mod(&mod_dir, "brrainz.achtung", "Achtung!")];
        let report = analyze_rimworld_log(&log_path, &mods);

        assert!(report.is_waylith);
        assert!(!report.has_hard_crash);
        assert_eq!(report.total_errors, 2, "two ERROR entries, WARN/INFO ignored");
        assert_eq!(report.culprits.len(), 1, "engine-only error has no mod culprit");
        let c = &report.culprits[0];
        assert_eq!(c.package_id.as_deref(), Some("brrainz.achtung"));
        assert_eq!(c.severity, ErrorSeverity::ModError);
        assert!(c.explanation.contains("AchtungMod.Controller.HandleDrafting"));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// A Unity crash inside the engine (no mod code in the stack): no culprit, but "Copy for
    /// AI" still carries the log's last lines and the errors before the crash.
    #[test]
    fn engine_crash_without_culprit_still_gives_the_ai_the_log() {
        let base = std::env::temp_dir().join(format!("rwmm_crash_test_{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let log_path = base.join("Player.log");
        // The crash block: "Crash!!!", then many DLL lines, then the end marker.
        let dlls: String = (0..300)
            .map(|i| format!("C:\\WINDOWS\\System32\\lib{i}.dll: size 1\n"))
            .collect();
        let log = format!(
            "Initialize engine version: 2022.3.35f1\n\
             Exception: Texture is not readable\n  at Verse.ContentFinder.Get\n\
             d3d11: failed to create 2D texture id=40508 [D3D error was 887a0005]\n\
             Crash!!!\n{dlls}\
             ========== END OF STACKTRACE ===========\n"
        );
        std::fs::write(&log_path, log).unwrap();

        let report = analyze_rimworld_log(&log_path, &[]);
        assert!(report.has_hard_crash);
        assert!(report.culprits.is_empty());
        assert!(
            report.crash_excerpt.contains("887a0005"),
            "what led to the crash, not the DLL list"
        );
        assert!(!report.crash_excerpt.contains("lib299.dll"));
        assert_eq!(report.recent_errors.len(), 1);

        let prompt = generate_ai_diagnostic_prompt(&report, true);
        assert!(prompt.contains("DXGI_ERROR_DEVICE_REMOVED"), "{prompt}");
        assert!(prompt.contains("Texture is not readable"));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// "Copy for AI" text for the user's own Player.log (run by hand:
    /// `cargo test --release live_ai_prompt -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn live_ai_prompt() {
        let path = detect_rimworld_log_paths()
            .into_iter()
            .map(|(_, p)| p)
            .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
            .expect("no Player.log");
        let report = analyze_rimworld_log(&path, &[]);
        println!("{}", generate_ai_diagnostic_prompt(&report, true));
    }

    #[test]
    fn shared_dll_does_not_blame_a_random_mod() {
        let base = std::env::temp_dir().join(format!("rwmm_dll_test_{}", std::process::id()));
        let a = base.join("a");
        let b = base.join("b");
        for d in [&a, &b] {
            std::fs::create_dir_all(d.join("Assemblies")).unwrap();
            std::fs::write(d.join("Assemblies").join("0Harmony.dll"), b"").unwrap();
        }
        std::fs::write(a.join("Assemblies").join("ModA.dll"), b"").unwrap();
        let mods = vec![
            test_mod(&a, "author.moda", "Mod A"),
            test_mod(&b, "author.modb", "Mod B"),
        ];
        let m = ModMatcher::build(&mods);
        assert!(m.find_mod("0Harmony").is_none(), "0Harmony.dll ships in both mods");
        assert_eq!(
            m.find_mod("ModA.SomeClass").map(|x| x.package_id.as_str()),
            Some("author.moda")
        );
        // Longest contained packageId wins, deterministically
        assert_eq!(
            m.find_mod("x.author.modb.y").map(|x| x.package_id.as_str()),
            Some("author.modb")
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn prompt_summary_follows_language() {
        let report = LogReport {
            total_errors: 3,
            summary_text: "русский текст".into(),
            ..Default::default()
        };
        let en = generate_ai_diagnostic_prompt(&report, false);
        assert!(en.contains("has 3 errors") && !en.contains("русский"));
        assert!(generate_ai_diagnostic_prompt(&report, true).contains("найдено ошибок: 3"));
    }

    #[test]
    fn prefers_newest_log_location() {
        let base = std::env::temp_dir().join(format!("rwmm_logpaths_{}", std::process::id()));
        std::fs::create_dir_all(base.join("Logs")).unwrap();
        std::fs::write(base.join("Player.log"), b"old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(base.join("Logs").join("Player.log"), b"new").unwrap();

        let found = log_paths_in(&base);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].1, base.join("Logs").join("Player.log"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn pattern_instantiation_and_cross_ref() {
        let base = std::env::temp_dir().join(format!("rwmm_patterns_{}", std::process::id()));
        let mod_dir = base.join("test_mod");
        std::fs::create_dir_all(mod_dir.join("Assemblies")).unwrap();
        std::fs::write(mod_dir.join("Assemblies").join("CustomMod.dll"), b"").unwrap();
        let log_path = base.join("Player.log");

        let log = "Initialize engine version: 2022.3.35f1\n\
             Error while instantiating class author.moda.CustomComponent\n\
             Could not resolve cross-reference: No Verse.SoundDef named BrokenSound found to give to author.modb.CompProperties_X author.modb.CompProperties_X (using undefined sound instead)\n\
             Could not resolve cross-reference to Verse.ThingDef named MissingSteel (wanter=costList)\n\
             Could not load UnityEngine.Texture2D at author.modc/Textures/Missing\n\
             Method not found: 'HarmonyLib.Harmony.CreateAndPatchAll'\n";
        std::fs::write(&log_path, log).unwrap();

        let moda_dir = base.join("moda");
        let modb_dir = base.join("modb");
        let modc_dir = base.join("modc");
        std::fs::create_dir_all(&moda_dir).unwrap();
        std::fs::create_dir_all(&modb_dir).unwrap();
        std::fs::create_dir_all(&modc_dir).unwrap();

        let mods = vec![
            test_mod(&moda_dir, "author.moda", "Mod A"),
            test_mod(&modb_dir, "author.modb", "Mod B"),
            test_mod(&modc_dir, "author.modc", "Mod C"),
        ];
        let report = analyze_rimworld_log(&log_path, &mods);

        assert_eq!(report.total_errors, 5);
        assert!(!report.raw_content.is_empty());

        // Cross-references in both real formats: the wanting class names the mod; a def
        // nobody claims gets a card of its own.
        let xref: Vec<&CrashCulprit> = report
            .culprits
            .iter()
            .filter(|c| c.severity == ErrorSeverity::CrossReferenceError)
            .collect();
        assert_eq!(xref.len(), 2, "{xref:?}");
        assert!(xref
            .iter()
            .any(|c| c.mod_name == "Mod B" && c.headline.contains("BrokenSound")));
        assert!(xref.iter().any(|c| c.mod_name == "ThingDef MissingSteel"));

        let has_inst = report
            .culprits
            .iter()
            .any(|c| c.severity == ErrorSeverity::InstantiationError);
        let has_cross = report
            .culprits
            .iter()
            .any(|c| c.severity == ErrorSeverity::CrossReferenceError);
        let has_tex = report
            .culprits
            .iter()
            .any(|c| c.severity == ErrorSeverity::BrokenTexture);
        let has_harm = report.culprits.iter().any(|c| c.mod_name.contains("Harmony"));

        assert!(has_inst, "instantiation pattern matched");
        assert!(has_cross, "cross-reference pattern matched");
        assert!(has_tex, "texture pattern matched");
        assert!(has_harm, "harmony conflict pattern matched");

        let txt = generate_text_report(&report);
        assert!(txt.contains("Mod A") && txt.contains("Mod B"));
        assert!(txt.contains("RimWorld 1.6 (Unity)"));

        let html = generate_html_report(&report);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Mod A"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
