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
    DefException,
    PrepatcherHarmonyError,
    HardCrash,
}

impl ErrorSeverity {
    pub fn badge_text(&self, is_ru: bool) -> &'static str {
        match self {
            ErrorSeverity::HardCrash => {
                if is_ru {
                    "ФАТАЛЬНЫЙ КРАШ (CTD)"
                } else {
                    "FATAL CRASH (CTD)"
                }
            }
            ErrorSeverity::PrepatcherHarmonyError => {
                if is_ru {
                    "СБОЙ PREPATCHER / HARMONY"
                } else {
                    "PREPATCHER / HARMONY ERROR"
                }
            }
            ErrorSeverity::DefException => {
                if is_ru {
                    "ОШИБКА ДЕФОВ (NULLREFERENCE)"
                } else {
                    "DEF EXCEPTION (NULLREF)"
                }
            }
            ErrorSeverity::MissingDependency => {
                if is_ru {
                    "НЕТ ЗАВИСИМОСТИ"
                } else {
                    "MISSING DEPENDENCY"
                }
            }
            ErrorSeverity::ModError => {
                if is_ru {
                    "ОШИБКА В МОДЕ"
                } else {
                    "MOD ERROR"
                }
            }
            ErrorSeverity::Warning => {
                if is_ru {
                    "ПРЕДУПРЕЖДЕНИЕ"
                } else {
                    "WARNING"
                }
            }
        }
    }

    pub fn badge_label(&self, is_ru: bool) -> &'static str {
        self.badge_text(is_ru)
    }

    pub fn badge_colors(&self) -> ([u8; 3], [u8; 3]) {
        match self {
            ErrorSeverity::HardCrash => ([180, 45, 45], [255, 255, 255]),
            ErrorSeverity::PrepatcherHarmonyError => ([195, 95, 25], [255, 255, 255]),
            ErrorSeverity::DefException => ([180, 115, 25], [255, 255, 255]),
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

#[derive(Debug, Clone, Default)]
pub struct LogReport {
    pub log_path: PathBuf,
    pub log_file_name: String,
    pub file_size_bytes: u64,
    pub has_hard_crash: bool,
    pub total_lines: usize,
    pub total_errors: usize,
    pub culprits: Vec<CrashCulprit>,
    pub summary_text: String,
    /// Log written by RimWorld 1.7+ (Waylith engine) instead of Unity/Mono.
    pub is_waylith: bool,
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
            matcher
                .package_id_map
                .insert(m.package_id.to_lowercase(), m);
            matcher.name_map.insert(m.name.to_lowercase(), m);

            if m.dir_path.exists() {
                for entry in walkdir::WalkDir::new(&m.dir_path)
                    .max_depth(5)
                    .into_iter()
                    .flatten()
                {
                    let path = entry.path();
                    let is_dll = path
                        .extension()
                        .map(|e| e.eq_ignore_ascii_case("dll"))
                        .unwrap_or(false);
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

/// Summary line in the requested language (the report's own summary_text is Russian).
pub fn summary_for(report: &LogReport, is_ru: bool) -> String {
    let n = report.culprits.len();
    let e = report.total_errors;
    match (is_ru, report.has_hard_crash, n > 0, e > 0) {
        (true, true, _, _) => format!("В логе зафиксирован фатальный краш процесса (CTD). Найдено виновников: {n}."),
        (false, true, _, _) => format!("Fatal process crash (CTD) detected in log. Found culprits: {n}."),
        (true, false, true, _) => format!("Обнаружены критические ошибки. Найдено проблемных модов: {n}."),
        (false, false, true, _) => format!("Critical errors detected. Found problematic mods: {n}."),
        (true, false, false, true) => format!("В логе найдено {e} ошибок, но прямых виновников не выявлено."),
        (false, false, false, true) => format!("Found {e} errors in the log, but no direct culprits identified."),
        (true, ..) => "Лог чист. Критических ошибок и вылетов не обнаружено.".to_string(),
        (false, ..) => "Log is clean. No critical errors or crashes detected.".to_string(),
    }
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
        ("Logs/Player.log (1.7+, последний запуск)", base.join("Logs").join("Player.log")),
        ("Logs/Player-prev.log (1.7+, предыдущий запуск)", base.join("Logs").join("Player-prev.log")),
        ("Player.log (1.6, последний запуск)", base.join("Player.log")),
        ("Player-prev.log (1.6, предыдущий запуск)", base.join("Player-prev.log")),
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
    lines.iter().take(20).any(|l| l.contains("=== Waylith log ===") || l.starts_with("Runtime:   .NET"))
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
        let is_fatal = matches!(entry.level, "FATAL" | "CRITICAL")
            || entry.message.contains("Unhandled exception");
        let is_error = is_fatal || entry.level == "ERROR";
        if !is_error {
            continue;
        }

        report.total_errors += 1;
        if is_fatal {
            report.has_hard_crash = true;
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
            explanation: format!(
                "Ошибка возникла в коде мода: `{}` (уровень {}).",
                frame, entry.level
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
            report.summary_text = "Не удалось прочитать файл лога.".to_string();
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

    let lines: Vec<&str> = content.lines().collect();
    report.total_lines = lines.len();

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
                        let excerpt = stack_slice
                            .iter()
                            .take(25)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("\n");

                        culprits_map.insert(
                            key,
                            CrashCulprit {
                                mod_name: matched_mod.name.clone(),
                                package_id: Some(matched_mod.package_id.clone()),
                                mod_dir: Some(matched_mod.dir_path.clone()),
                                severity: ErrorSeverity::HardCrash,
                                headline: format!(
                                    "Вызов метода `{}` привел к фатальному сбою игры (CTD)",
                                    method
                                ),
                                explanation: format!(
                                    "Метод `{}:{}` выполнялся непосредственно перед крашем игрового движка Unity.",
                                    full_class, method
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
                                    headline: format!(
                                        "Фатальный сбой в модуле `{}`",
                                        matched_mod.name
                                    ),
                                    explanation: format!(
                                        "Исключение в методе `{}:{}` привело к аварийному завершению процесса.",
                                        full_class, method
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
                            headline: format!(
                                "Ошибка препатчера Prepatcher в моде `{}`",
                                matched_mod.name
                            ),
                            explanation: format!(
                                "Патч `{}` не смог инъецироваться в сборку. Это часто вызывает черный экран или зависание загрузки.",
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
                        headline: format!("Отсутствует или не загружена зависимость `{}`", dep_id),
                        explanation: format!(
                            "Мод `{}` требует наличие активного мода с Package ID `{}`.",
                            target_name, dep_id
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
                            headline: format!(
                                "Ошибка DefDatabase / NullReference в объекте `{}`",
                                def_name
                            ),
                            explanation: format!(
                                "Критическая ошибка конфигурации объекта `{}` в моде `{}`.",
                                def_name, m.name
                            ),
                            stack_trace_excerpt: excerpt.clone(),
                            occurrences: 0,
                        });
                        entry.occurrences += 1;
                    }
                }
            }
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

    if report.has_hard_crash {
        report.summary_text = format!(
            "В логе зафиксирован фатальный краш процесса (CTD). Найдено виновников: {}.",
            report.culprits.len()
        );
    } else if !report.culprits.is_empty() {
        report.summary_text = format!(
            "Обнаружены критические ошибки инициализации. Найдено проблемных модов: {}.",
            report.culprits.len()
        );
    } else if report.total_errors > 0 {
        report.summary_text = format!(
            "В логе найдено {} ошибок, но прямых виновников фатального сбоя не выявлено.",
            report.total_errors
        );
    } else {
        report.summary_text = "Лог чист. Критических ошибок и вылетов не обнаружено.".to_string();
    }

    report
}

pub fn generate_ai_diagnostic_prompt(report: &LogReport, is_ru: bool) -> String {
    let mut out = String::new();

    if is_ru {
        out.push_str("Привет! Пожалуйста, помоги разобраться с ошибкой/вылетом игры RimWorld на основе системного лога.\n\n");
        out.push_str(&format!("📁 **Файл лога:** {}\n", report.log_file_name));
        out.push_str(&format!("📊 **Общий статус:** {}\n", summary_for(report, true)));

        out.push_str(if report.is_waylith {
            "🎮 **Версия игры:** RimWorld 1.7+ (движок Waylith, .NET 10)\n"
        } else {
            "🎮 **Версия игры:** RimWorld 1.6 или старше (движок Unity, Mono)\n"
        });

        if report.has_hard_crash {
            out.push_str(if report.is_waylith {
                "⚠️ **Внимание: Зафиксирован фатальный вылет игрового процесса (Crash to Desktop).**\n"
            } else {
                "⚠️ **Внимание: Зафиксирован фатальный вылет игрового процесса (Crash to Desktop / Unity engine fault).**\n"
            });
        }

        if !report.culprits.is_empty() {
            out.push_str("\n### НАЙДЕННЫЕ ПРОБЛЕМНЫЕ МОДЫ И ОШИБКИ:\n\n");
            for (idx, c) in report.culprits.iter().enumerate() {
                out.push_str(&format!(
                    "{}. Мод: **{}** (ID: `{}`)\n",
                    idx + 1,
                    c.mod_name,
                    c.package_id.as_deref().unwrap_or("неизвестен")
                ));
                out.push_str(&format!(
                    "   - Уровень серьезности: {}\n",
                    c.severity.badge_text(true)
                ));
                out.push_str(&format!("   - Ошибка: {}\n", c.headline));
                out.push_str(&format!("   - Описание проблемы: {}\n", c.explanation));

                if !c.stack_trace_excerpt.is_empty() {
                    out.push_str("   - Стек вызовов из лога:\n   ```text\n");
                    for line in c.stack_trace_excerpt.lines().take(20) {
                        out.push_str(&format!("     {}\n", line));
                    }
                    out.push_str("   ```\n");
                }
                out.push('\n');
            }
        }

        out.push_str("### ВОПРОСЫ К НЕЙРОСЕТИ:\n");
        out.push_str("1. В чем конкретно заключается причина данного сбоя или конфликта?\n");
        out.push_str("2. Какой мод является главным виновником, и с чем он конфликтует?\n");
        out.push_str("3. Какие конкретные действия предпринять: отключить определенный мод, изменить порядок загрузки, обновить версию или использовать патч совместимости?\n");
    } else {
        out.push_str("Hello! Please help diagnose a RimWorld crash/startup error based on this system log.\n\n");
        out.push_str(&format!("📁 **Log file:** {}\n", report.log_file_name));
        out.push_str(&format!("📊 **Status:** {}\n", summary_for(report, false)));

        out.push_str(if report.is_waylith {
            "🎮 **Game version:** RimWorld 1.7+ (Waylith engine, .NET 10)\n"
        } else {
            "🎮 **Game version:** RimWorld 1.6 or older (Unity engine, Mono)\n"
        });

        if report.has_hard_crash {
            out.push_str(if report.is_waylith {
                "⚠️ **Warning: A fatal game crash occurred (Crash to Desktop).**\n"
            } else {
                "⚠️ **Warning: A fatal game crash occurred (Crash to Desktop / Unity engine fault).**\n"
            });
        }

        if !report.culprits.is_empty() {
            out.push_str("\n### DETECTED PROBLEMATIC MODS & ERRORS:\n\n");
            for (idx, c) in report.culprits.iter().enumerate() {
                out.push_str(&format!(
                    "{}. Mod: **{}** (ID: `{}`)\n",
                    idx + 1,
                    c.mod_name,
                    c.package_id.as_deref().unwrap_or("unknown")
                ));
                out.push_str(&format!(
                    "   - Severity: {}\n",
                    c.severity.badge_text(false)
                ));
                out.push_str(&format!("   - Issue: {}\n", c.headline));
                out.push_str(&format!("   - Details: {}\n", c.explanation));

                if !c.stack_trace_excerpt.is_empty() {
                    out.push_str("   - Stack trace excerpt:\n   ```text\n");
                    for line in c.stack_trace_excerpt.lines().take(20) {
                        out.push_str(&format!("     {}\n", line));
                    }
                    out.push_str("   ```\n");
                }
                out.push('\n');
            }
        }

        out.push_str("### QUESTIONS FOR AI:\n");
        out.push_str("1. What is the root cause of this crash or failure?\n");
        out.push_str("2. Which mod is the primary culprit and what is it conflicting with?\n");
        out.push_str("3. What specific actions should I take (disable mod, change load order, update, or patch)?\n");
    }

    out
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
        let unity = ["Mono path[0] = 'C:/Games/RimWorld/RimWorldWin64_Data/Managed'", "Initialize engine version: 2022.3.35f1"];
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
        let mods = vec![test_mod(&a, "author.moda", "Mod A"), test_mod(&b, "author.modb", "Mod B")];
        let m = ModMatcher::build(&mods);
        assert!(m.find_mod("0Harmony").is_none(), "0Harmony.dll ships in both mods");
        assert_eq!(m.find_mod("ModA.SomeClass").map(|x| x.package_id.as_str()), Some("author.moda"));
        // Longest contained packageId wins, deterministically
        assert_eq!(m.find_mod("x.author.modb.y").map(|x| x.package_id.as_str()), Some("author.modb"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn prompt_summary_follows_language() {
        let report = LogReport { total_errors: 3, summary_text: "русский текст".into(), ..Default::default() };
        let en = generate_ai_diagnostic_prompt(&report, false);
        assert!(en.contains("Found 3 errors") && !en.contains("русский"));
        assert!(generate_ai_diagnostic_prompt(&report, true).contains("найдено 3 ошибок"));
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
}
