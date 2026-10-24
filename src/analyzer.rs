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
}

#[derive(Default)]
struct ModMatcher<'a> {
    dll_map: HashMap<String, &'a ModInfo>,
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
                    if path.is_file() {
                        if let Some(ext) = path.extension() {
                            if ext.eq_ignore_ascii_case("dll") {
                                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                                    matcher.dll_map.insert(stem.to_lowercase(), m);
                                }
                            }
                        }
                    }
                }
            }
        }

        matcher
    }

    fn find_mod(&self, token: &str) -> Option<&'a ModInfo> {
        let clean = token.trim().to_lowercase();
        if clean.is_empty() {
            return None;
        }

        if let Some(m) = self.dll_map.get(&clean) {
            return Some(m);
        }

        if let Some(m) = self.package_id_map.get(&clean) {
            return Some(m);
        }

        if let Some(m) = self.name_map.get(&clean) {
            return Some(m);
        }

        let parts: Vec<&str> = clean.split('.').collect();
        for part in &parts {
            if part.len() >= 4 {
                if let Some(m) = self.dll_map.get(*part) {
                    return Some(m);
                }
            }
        }

        for (pkg_id, m) in &self.package_id_map {
            if clean.contains(pkg_id) {
                return Some(m);
            }
        }

        None
    }
}

pub fn detect_rimworld_log_paths() -> Vec<(String, PathBuf)> {
    let mut results = Vec::new();

    let candidates = [
        std::env::var("USERPROFILE").ok().map(|p| {
            PathBuf::from(p)
                .join("AppData")
                .join("LocalLow")
                .join("Ludeon Studios")
                .join("RimWorld by Ludeon Studios")
        }),
        std::env::var("LOCALAPPDATA").ok().and_then(|p| {
            PathBuf::from(p).parent().map(|parent| {
                parent
                    .join("LocalLow")
                    .join("Ludeon Studios")
                    .join("RimWorld by Ludeon Studios")
            })
        }),
    ];

    for base_opt in candidates.iter().flatten() {
        if base_opt.exists() {
            let player_log = base_opt.join("Player.log");
            if player_log.exists() {
                results.push(("Player.log (Последний запуск)".to_string(), player_log));
            }

            let prev_log = base_opt.join("Player-prev.log");
            if prev_log.exists() {
                results.push(("Player-prev.log (Предыдущий запуск)".to_string(), prev_log));
            }
            break;
        }
    }

    results
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

    let re_mono_call = Regex::new(r"\(Mono JIT Code\)\s+([A-Za-z0-9_\.]+):([A-Za-z0-9_]+)").ok();
    let re_mono_at = Regex::new(r"^\s*at\s+([A-Za-z0-9_\.]+)\.([A-Za-z0-9_<>]+)").ok();
    let re_prepatcher = Regex::new(r"Prepatcher Error:.*?static\s+.*?\s+([A-Za-z0-9_\.]+)::").ok();
    let re_dependency = Regex::new(r"Mod\s+(.*?)\s+dependency\s+\((.*?)\)").ok();
    let re_config_error = Regex::new(r"Exception in ConfigErrors\(\) of ([A-Za-z0-9_]+):").ok();

    let mut crash_marker_idx = None;
    for (i, line) in lines.iter().enumerate().rev() {
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
            report.total_errors += 1;
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
            report.total_errors += 1;
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
            report.total_errors += 1;
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

        if line.contains("Exception:") || line.contains("Error in ") {
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
        out.push_str(&format!("📊 **Общий статус:** {}\n", report.summary_text));

        if report.has_hard_crash {
            out.push_str("⚠️ **Внимание: Зафиксирован фатальный вылет игрового процесса (Crash to Desktop / Unity engine fault).**\n");
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
        out.push_str(&format!("📊 **Status:** {}\n", report.summary_text));

        if report.has_hard_crash {
            out.push_str("⚠️ **Warning: A fatal game crash occurred (Crash to Desktop / Unity engine fault).**\n");
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
