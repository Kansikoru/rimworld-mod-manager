use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

/// Current interface language for texts built outside the UI code (status lines,
/// errors from background tasks). Set by `I18n::new` / `I18n::set_language`.
static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn is_english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// Russian or English text by the current interface language.
/// With arguments it works like `format!`: `tr!("Найдено: {}", "Found: {}", n)`.
macro_rules! tr {
    ($ru:literal, $en:literal $(,)?) => {
        if $crate::i18n::is_english() { $en } else { $ru }
    };
    ($ru:literal, $en:literal, $($arg:tt)+) => {
        if $crate::i18n::is_english() { format!($en, $($arg)+) } else { format!($ru, $($arg)+) }
    };
}
pub(crate) use tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Language {
    #[default]
    Russian,
    English,
}

impl Language {
    pub fn label(&self) -> &'static str {
        match self {
            Language::Russian => "Русский",
            Language::English => "English",
        }
    }

    pub fn toggle(&self) -> Self {
        match self {
            Language::Russian => Language::English,
            Language::English => Language::Russian,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TranslationKey {
    // Tabs
    TabMods,
    TabTranslations,
    TabOptimizer,
    TabAnalyzer,
    TabSettings,

    // Mods View
    BtnAutoSort,
    BtnSaveConfig,
    BtnRescanMods,
    BtnEnableAll,
    BtnDisableAll,
    SearchPlaceholder,
    TabDependencies,
    DepMissing,
    DepDisabled,
    DepWrongOrder,
    BtnAutoFixDependencies,
    ActiveModsTitle,
    BtnDeactivate,
    AvailableModsTitle,
    BtnActivate,
    BtnOpenFolder,
    BtnOpenWorkshop,

    // Translations View
    TransTitle,
    TransSubtitle,
    TransTargetLang,
    TransBtnDeleteAll,
    TransBtnFindWorkshop,
    TransFoundTitle,
    TransCleanNotice,
    TransWorkshopResultsTitle,
    TransBtnSubscribe,

    // Optimizer View
    OptTitle,
    OptSubtitle,
    OptScope,
    OptScopeAll,
    OptScopeActive,
    OptScopeSelected,
    OptForceReencode,
    OptDeleteSource,
    OptThreads,
    OptBtnStart,
    OptBtnScan,
    OptBtnRollback,
    OptBtnCancel,

    // Analyzer View
    CrashTitle,
    CrashSubtitle,
    CrashBtnAnalyze,
    CrashBtnCopyPrompt,
    CrashNoErrors,

    // Settings View
    SettingsTitle,
    SettingsLanguage,
    SettingsPathsTitle,
    SettingsRimWorldPath,
    SettingsWorkshopPath,
    SettingsConfigPath,
    SettingsLaunchTitle,
    SettingsBtnLaunchRimWorld,
    SettingsDbTitle,
    SettingsBtnUpdateDb,

    // Dynamic & Extended keys
    SettingsSubtitle,
    SettingsNotFound,
    SettingsBrowse,
    SettingsDbCount,
    AppPlayBtn,
    AppSortedMsg,
    AppStatusBarStats,
    AnalyzerBrowseLog,
    OptDdsTitle,
    OptDdsDesc,
    TransSearchHint,
    TransBtnSearch,
    ModListSummary,
    OptThreadsCount,
    TransSearchPlaceholder,
    ModsSearchActive,
    ModsTotal,
    ModsSummaryHeader,
    StatusModsCount,
    ModsBtnInfoPanel,
    ModsCtxDeactivate,
    ModsCtxDeactivateSelected,
    ModsCtxMoveUp,
    ModsCtxMoveDown,
    ModsCtxOpenFolder,
    ModsCtxOpenWorkshop,
    ModsCtxOptSkip,
    ModsCtxOptAllow,
    ModsDepNotInList,
    ModsDepEnable,
    ModsDepNotFound,
    ModsDepOrder,
    ModsCtxActivate,
    ModsCtxActivateSelected,
    ModsDetailsTitle,
    ModsHideInfoPanel,
    ModsBadgeProtected,
    ModsBtnFolder,
    ModsAuthor,
    ModsDependencies,
    CrashSummaryHardCrash,
    CrashSummaryCulprits,
    CrashSummaryErrorsOnly,
    CrashSummaryClean,
    CrashFileStats,
    CrashAnalyzing,
    CrashFileNotFound,

    // Game version support (1.6 Unity / 1.7 Waylith)
    SettingsGameDetected,
    SettingsLaunchExtraArgs,
    SettingsLaunchUnverified,
    SettingsLaunchWaylithHint,
    OptWaylithWarning,

    // Safe deletion
    ConfirmDeleteTitle,
    ConfirmDeleteBody,
    ConfirmDeleteBtn,
    BtnCancel,
    TransNoneForLang,
    ModsNotOnDisk,
    ModsDuplicatesTitle,

    // Mod categories
    CategoriesAll,
    CategoriesMenu,
    CategoriesReset,
    CategoriesManual,
    CategoriesNone,

    // Game version checks
    ModsVersionBadge,
    ModsVersionHint,
    ModsVersionSummary,
    VersionPrepatcher,
    VersionHarmony,

    // Optimizer v2
    OptFlip,
    OptFlipHint,
    OptAllowOnWaylith,
    OptClearGameCache,
    OptClearGameCacheHint,

    // Incompatibilities
    ModsDepIncompatible,
    DepIncompatible,

    // Community rules
    SettingsBtnImportDb,
    SettingsDbHint,

    // Saved settings
    SettingsCheckSteam,
    SettingsAutoDetect,
    SettingsSavedHint,

    // Interface font
    SettingsFont,
    SettingsTextSize,
    SettingsFontHint,
}

/// Fills `{}` placeholders one by one: `fmt("A: {} | B: {}", &["1", "2"])` -> "A: 1 | B: 2".
/// (`str::replace("{}", ..)` would fill every placeholder with the first value.)
pub fn fmt(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    let mut args = args.iter();
    while let Some(pos) = rest.find("{}") {
        out.push_str(&rest[..pos]);
        match args.next() {
            Some(a) => out.push_str(a),
            None => out.push_str("{}"),
        }
        rest = &rest[pos + 2..];
    }
    out.push_str(rest);
    out
}

impl TranslationKey {
    pub fn translate(&self, lang: Language) -> &'static str {
        match (self, lang) {
            // Tabs
            (TranslationKey::TabMods, Language::Russian) => "Моды",
            (TranslationKey::TabMods, Language::English) => "Mods",
            (TranslationKey::TabTranslations, Language::Russian) => "Переводы",
            (TranslationKey::TabTranslations, Language::English) => "Translations",
            (TranslationKey::TabOptimizer, Language::Russian) => "Оптимизация",
            (TranslationKey::TabOptimizer, Language::English) => "Optimizer",
            (TranslationKey::TabAnalyzer, Language::Russian) => "Анализатор логов",
            (TranslationKey::TabAnalyzer, Language::English) => "Log Analyzer",
            (TranslationKey::TabSettings, Language::Russian) => "Настройки",
            (TranslationKey::TabSettings, Language::English) => "Settings",

            // Mods View
            (TranslationKey::BtnAutoSort, Language::Russian) => "Авто-сортировка",
            (TranslationKey::BtnAutoSort, Language::English) => "Auto-Sort",
            (TranslationKey::BtnSaveConfig, Language::Russian) => "Сохранить порядок",
            (TranslationKey::BtnSaveConfig, Language::English) => "Save Order",
            (TranslationKey::BtnRescanMods, Language::Russian) => "Пересканировать",
            (TranslationKey::BtnRescanMods, Language::English) => "Rescan Mods",
            (TranslationKey::BtnEnableAll, Language::Russian) => "Включить все",
            (TranslationKey::BtnEnableAll, Language::English) => "Enable All",
            (TranslationKey::BtnDisableAll, Language::Russian) => "Отключить все",
            (TranslationKey::BtnDisableAll, Language::English) => "Disable All",
            (TranslationKey::SearchPlaceholder, Language::Russian) => "Поиск модов...",
            (TranslationKey::SearchPlaceholder, Language::English) => "Search mods...",
            (TranslationKey::TabDependencies, Language::Russian) => "Зависимости",
            (TranslationKey::TabDependencies, Language::English) => "Dependencies",
            (TranslationKey::DepMissing, Language::Russian) => "отсутствует",
            (TranslationKey::DepMissing, Language::English) => "missing",
            (TranslationKey::DepDisabled, Language::Russian) => "выключено",
            (TranslationKey::DepDisabled, Language::English) => "disabled",
            (TranslationKey::DepWrongOrder, Language::Russian) => "неверный порядок",
            (TranslationKey::DepWrongOrder, Language::English) => "wrong order",
            (TranslationKey::BtnAutoFixDependencies, Language::Russian) => "Авто-исправление",
            (TranslationKey::BtnAutoFixDependencies, Language::English) => "Auto-fix",
            (TranslationKey::ActiveModsTitle, Language::Russian) => "Активные моды",
            (TranslationKey::ActiveModsTitle, Language::English) => "Active Mods",
            (TranslationKey::BtnDeactivate, Language::Russian) => "Отключить",
            (TranslationKey::BtnDeactivate, Language::English) => "Deactivate",
            (TranslationKey::AvailableModsTitle, Language::Russian) => "Доступные моды",
            (TranslationKey::AvailableModsTitle, Language::English) => "Available Mods",
            (TranslationKey::BtnActivate, Language::Russian) => "Включить",
            (TranslationKey::BtnActivate, Language::English) => "Activate",
            (TranslationKey::BtnOpenFolder, Language::Russian) => "Открыть папку",
            (TranslationKey::BtnOpenFolder, Language::English) => "Open Folder",
            (TranslationKey::BtnOpenWorkshop, Language::Russian) => "Мастерская Steam",
            (TranslationKey::BtnOpenWorkshop, Language::English) => "Steam Workshop",

            // Translations View
            (TranslationKey::TransTitle, Language::Russian) => "Менеджер переводов",
            (TranslationKey::TransTitle, Language::English) => "Translation Manager",
            (TranslationKey::TransSubtitle, Language::Russian) => "Поиск и управление локализациями модов Steam Workshop",
            (TranslationKey::TransSubtitle, Language::English) => "Search and manage Steam Workshop mod localizations",
            (TranslationKey::TransTargetLang, Language::Russian) => "Целевой язык",
            (TranslationKey::TransTargetLang, Language::English) => "Target Language",
            (TranslationKey::TransBtnDeleteAll, Language::Russian) => "Удалить все переводы",
            (TranslationKey::TransBtnDeleteAll, Language::English) => "Delete All Translations",
            (TranslationKey::TransBtnFindWorkshop, Language::Russian) => "Найти в Мастерской",
            (TranslationKey::TransBtnFindWorkshop, Language::English) => "Find on Workshop",
            (TranslationKey::TransFoundTitle, Language::Russian) => "Найденные переводы",
            (TranslationKey::TransFoundTitle, Language::English) => "Installed Translations",
            (TranslationKey::TransCleanNotice, Language::Russian) => "Переводов для выбранного языка не найдено.",
            (TranslationKey::TransCleanNotice, Language::English) => "No translations found for the selected language.",
            (TranslationKey::TransWorkshopResultsTitle, Language::Russian) => "Результаты в Мастерской",
            (TranslationKey::TransWorkshopResultsTitle, Language::English) => "Steam Workshop Results",
            (TranslationKey::TransBtnSubscribe, Language::Russian) => "Подписаться",
            (TranslationKey::TransBtnSubscribe, Language::English) => "Subscribe",

            // Optimizer View
            (TranslationKey::OptTitle, Language::Russian) => "Оптимизатор текстур",
            (TranslationKey::OptTitle, Language::English) => "Texture Optimizer",
            (TranslationKey::OptSubtitle, Language::Russian) => "PNG/JPG → DDS: BC7 для текстур с прозрачностью, BC1 для остальных, с мипмапами, без изменения размера",
            (TranslationKey::OptSubtitle, Language::English) => "PNG/JPG → DDS: BC7 for textures with transparency, BC1 otherwise, with mipmaps, original size",
            (TranslationKey::OptScope, Language::Russian) => "Область",
            (TranslationKey::OptScope, Language::English) => "Scope",
            (TranslationKey::OptScopeAll, Language::Russian) => "Все моды",
            (TranslationKey::OptScopeAll, Language::English) => "All Mods",
            (TranslationKey::OptScopeActive, Language::Russian) => "Только активные",
            (TranslationKey::OptScopeActive, Language::English) => "Active Only",
            (TranslationKey::OptScopeSelected, Language::Russian) => "Выбранный мод",
            (TranslationKey::OptScopeSelected, Language::English) => "Selected Only",
            (TranslationKey::OptForceReencode, Language::Russian) => "Перезаписывать существующие DDS",
            (TranslationKey::OptForceReencode, Language::English) => "Force re-encode existing DDS",
            (TranslationKey::OptDeleteSource, Language::Russian) => "Убирать исходные PNG/JPG в бэкап",
            (TranslationKey::OptDeleteSource, Language::English) => "Move source PNG/JPG to backup",
            (TranslationKey::OptThreads, Language::Russian) => "Нагрузка CPU",
            (TranslationKey::OptThreads, Language::English) => "CPU Threads",
            (TranslationKey::OptBtnStart, Language::Russian) => "Начать оптимизацию",
            (TranslationKey::OptBtnStart, Language::English) => "Start Optimization",
            (TranslationKey::OptBtnScan, Language::Russian) => "Сканировать",
            (TranslationKey::OptBtnScan, Language::English) => "Scan",
            (TranslationKey::OptBtnRollback, Language::Russian) => "Откатить (.dds)",
            (TranslationKey::OptBtnRollback, Language::English) => "Rollback (.dds)",
            (TranslationKey::OptBtnCancel, Language::Russian) => "Остановить",
            (TranslationKey::OptBtnCancel, Language::English) => "Stop",

            // Analyzer View
            (TranslationKey::CrashTitle, Language::Russian) => "Анализатор логов и сбоев",
            (TranslationKey::CrashTitle, Language::English) => "Crash & Log Analyzer",
            (TranslationKey::CrashSubtitle, Language::Russian) => "Диагностика ошибок RimWorld и подготовка запроса для ИИ",
            (TranslationKey::CrashSubtitle, Language::English) => "RimWorld crash analysis and AI diagnostic prompt generator",
            (TranslationKey::CrashBtnAnalyze, Language::Russian) => "Анализировать",
            (TranslationKey::CrashBtnAnalyze, Language::English) => "Analyze",
            (TranslationKey::CrashBtnCopyPrompt, Language::Russian) => "Копировать для ИИ",
            (TranslationKey::CrashBtnCopyPrompt, Language::English) => "Copy for AI",
            (TranslationKey::CrashNoErrors, Language::Russian) => "Критических ошибок в логе не обнаружено!",
            (TranslationKey::CrashNoErrors, Language::English) => "No critical errors found in the log!",

            // Settings View
            (TranslationKey::SettingsTitle, Language::Russian) => "Настройки",
            (TranslationKey::SettingsTitle, Language::English) => "Settings",
            (TranslationKey::SettingsLanguage, Language::Russian) => "Язык интерфейса",
            (TranslationKey::SettingsLanguage, Language::English) => "Language",
            (TranslationKey::SettingsPathsTitle, Language::Russian) => "Пути к файлам и каталогам",
            (TranslationKey::SettingsPathsTitle, Language::English) => "Paths Configuration",
            (TranslationKey::SettingsRimWorldPath, Language::Russian) => "Папка игры RimWorld",
            (TranslationKey::SettingsRimWorldPath, Language::English) => "RimWorld Game Path",
            (TranslationKey::SettingsWorkshopPath, Language::Russian) => "Папка модов Workshop",
            (TranslationKey::SettingsWorkshopPath, Language::English) => "Workshop Mods Path",
            (TranslationKey::SettingsConfigPath, Language::Russian) => "Файл конфигурации (ModsConfig.xml)",
            (TranslationKey::SettingsConfigPath, Language::English) => "ModsConfig.xml Path",
            (TranslationKey::SettingsLaunchTitle, Language::Russian) => "Параметры запуска RimWorld",
            (TranslationKey::SettingsLaunchTitle, Language::English) => "Launch Parameters",
            (TranslationKey::SettingsBtnLaunchRimWorld, Language::Russian) => "Запустить игру",
            (TranslationKey::SettingsBtnLaunchRimWorld, Language::English) => "Launch RimWorld",
            (TranslationKey::SettingsDbTitle, Language::Russian) => "База правил сообщества",
            (TranslationKey::SettingsDbTitle, Language::English) => "Community Rules Database",
            (TranslationKey::SettingsBtnUpdateDb, Language::Russian) => "Обновить из интернета",
            (TranslationKey::SettingsBtnUpdateDb, Language::English) => "Update from the internet",

            // Dynamic & Extended keys
            (TranslationKey::SettingsSubtitle, Language::Russian) => "Конфигурация путей, локализации и параметров запуска",
            (TranslationKey::SettingsSubtitle, Language::English) => "Configuration file paths and launch parameters",
            (TranslationKey::SettingsNotFound, Language::Russian) => "Не определено",
            (TranslationKey::SettingsNotFound, Language::English) => "Not found",
            (TranslationKey::SettingsBrowse, Language::Russian) => "Обзор...",
            (TranslationKey::SettingsBrowse, Language::English) => "Browse...",
            (TranslationKey::SettingsDbCount, Language::Russian) => "Правил сортировки и совместимости в базе: {}",
            (TranslationKey::SettingsDbCount, Language::English) => "Sorting and compatibility rules in DB: {}",
            (TranslationKey::AppPlayBtn, Language::Russian) => "Играть",
            (TranslationKey::AppPlayBtn, Language::English) => "Play",
            (TranslationKey::AppSortedMsg, Language::Russian) => "Порядок загрузки отсортирован",
            (TranslationKey::AppSortedMsg, Language::English) => "Load order sorted",
            (TranslationKey::AppStatusBarStats, Language::Russian) => "Всего: {} | Активно: {}",
            (TranslationKey::AppStatusBarStats, Language::English) => "Total: {} | Active: {}",

            (TranslationKey::AnalyzerBrowseLog, Language::Russian) => "Выбрать файл лога вручную",
            (TranslationKey::AnalyzerBrowseLog, Language::English) => "Select log file manually",
            (TranslationKey::OptDdsTitle, Language::Russian) => "Как работает оптимизатор (RimWorld 1.6 и старше)",
            (TranslationKey::OptDdsTitle, Language::English) => "How the optimizer works (RimWorld 1.6 and older)",
            (TranslationKey::OptDdsDesc, Language::Russian) => "• Игра грузит готовые сжатые текстуры без распаковки PNG — запуск больших сборок быстрее, памяти нужно меньше\n• Те же параметры, что у ToDDS: BC7/BC1, мипмапы, без переворота и без растягивания\n• Текстуры не кратные 4 и мельче 64 px остаются PNG; DDS автора мода не трогаются\n• Все созданные DDS записываются в журнал — «Откатить» удаляет их полностью\n• После обновления мода устаревшие DDS пересоздаются автоматически",
            (TranslationKey::OptDdsDesc, Language::English) => "• The game loads ready compressed textures instead of decoding PNG — faster start, less memory on big modpacks\n• Same settings as ToDDS: BC7/BC1, mipmaps, no flip, no stretching\n• Textures not a multiple of 4 or under 64 px stay PNG; the mod author's DDS are never touched\n• Every created DDS is recorded — \"Rollback\" removes all of them\n• After a mod update stale DDS are re-encoded automatically",
            (TranslationKey::TransSearchHint, Language::Russian) => "Результатов пока нет. Введите название мода или выберите мод в списке.",
            (TranslationKey::TransSearchHint, Language::English) => "No results yet. Type a mod name or pick a mod in the list.",
            (TranslationKey::TransBtnSearch, Language::Russian) => "Искать",
            (TranslationKey::TransBtnSearch, Language::English) => "Search",
            (TranslationKey::ModListSummary, Language::Russian) => "Активно: {} | Доступно: {} | Выберите мод в списке для деталей",
            (TranslationKey::ModListSummary, Language::English) => "Active: {} | Available: {} | Select a mod in the list for details",

            (TranslationKey::OptThreadsCount, Language::Russian) => "потоков (из {})",
            (TranslationKey::OptThreadsCount, Language::English) => "threads (of {})",
            (TranslationKey::TransSearchPlaceholder, Language::Russian) => "Поиск переводов в Мастерской Steam...",
            (TranslationKey::TransSearchPlaceholder, Language::English) => "Search translations in Steam Workshop...",
            (TranslationKey::ModsSearchActive, Language::Russian) => "Поиск в активных...",
            (TranslationKey::ModsSearchActive, Language::English) => "Search active...",
            (TranslationKey::ModsTotal, Language::Russian) => "Всего: {}",
            (TranslationKey::ModsTotal, Language::English) => "Total: {}",
            (TranslationKey::ModsSummaryHeader, Language::Russian) => "Сводка набора",
            (TranslationKey::ModsSummaryHeader, Language::English) => "Mod list summary",
            (TranslationKey::StatusModsCount, Language::Russian) => "Найдено модов: {} | Активных: {}",
            (TranslationKey::StatusModsCount, Language::English) => "Found mods: {} | Active: {}",

            (TranslationKey::ModsBtnInfoPanel, Language::Russian) => "Панель информации",
            (TranslationKey::ModsBtnInfoPanel, Language::English) => "Info Panel",
            (TranslationKey::ModsCtxDeactivate, Language::Russian) => "Отключить",
            (TranslationKey::ModsCtxDeactivate, Language::English) => "Deactivate",
            (TranslationKey::ModsCtxDeactivateSelected, Language::Russian) => "Отключить выделенные ({})",
            (TranslationKey::ModsCtxDeactivateSelected, Language::English) => "Deactivate selected ({})",
            (TranslationKey::ModsCtxMoveUp, Language::Russian) => "Переместить вверх",
            (TranslationKey::ModsCtxMoveUp, Language::English) => "Move up",
            (TranslationKey::ModsCtxMoveDown, Language::Russian) => "Переместить вниз",
            (TranslationKey::ModsCtxMoveDown, Language::English) => "Move down",
            (TranslationKey::ModsCtxOpenFolder, Language::Russian) => "Открыть папку мода",
            (TranslationKey::ModsCtxOpenFolder, Language::English) => "Open mod folder",
            (TranslationKey::ModsCtxOpenWorkshop, Language::Russian) => "Открыть в Steam Workshop",
            (TranslationKey::ModsCtxOpenWorkshop, Language::English) => "Open in Steam Workshop",
            (TranslationKey::ModsCtxOptSkip, Language::Russian) => "Разрешить оптимизацию текстур",
            (TranslationKey::ModsCtxOptSkip, Language::English) => "Allow texture optimization",
            (TranslationKey::ModsCtxOptAllow, Language::Russian) => "Исключить из оптимизации текстур",
            (TranslationKey::ModsCtxOptAllow, Language::English) => "Exclude from texture optimization",
            (TranslationKey::ModsDepNotInList, Language::Russian) => "Нет в списке",
            (TranslationKey::ModsDepNotInList, Language::English) => "Not in list",
            (TranslationKey::ModsDepEnable, Language::Russian) => "+ Включить",
            (TranslationKey::ModsDepEnable, Language::English) => "+ Enable",
            (TranslationKey::ModsDepNotFound, Language::Russian) => "Не найден",
            (TranslationKey::ModsDepNotFound, Language::English) => "Not found",
            (TranslationKey::ModsDepOrder, Language::Russian) => "Порядок",
            (TranslationKey::ModsDepOrder, Language::English) => "Order",
            (TranslationKey::ModsCtxActivate, Language::Russian) => "Включить",
            (TranslationKey::ModsCtxActivate, Language::English) => "Activate",
            (TranslationKey::ModsCtxActivateSelected, Language::Russian) => "Включить выделенные ({})",
            (TranslationKey::ModsCtxActivateSelected, Language::English) => "Activate selected ({})",
            (TranslationKey::ModsDetailsTitle, Language::Russian) => "Информация о моде",
            (TranslationKey::ModsDetailsTitle, Language::English) => "Mod Details",
            (TranslationKey::ModsHideInfoPanel, Language::Russian) => "Скрыть панель информации",
            (TranslationKey::ModsHideInfoPanel, Language::English) => "Hide info panel",
            (TranslationKey::ModsBadgeProtected, Language::Russian) => "Защищённый",
            (TranslationKey::ModsBadgeProtected, Language::English) => "Protected",
            (TranslationKey::ModsBtnFolder, Language::Russian) => "Папка",
            (TranslationKey::ModsBtnFolder, Language::English) => "Folder",
            (TranslationKey::ModsAuthor, Language::Russian) => "Автор: {}",
            (TranslationKey::ModsAuthor, Language::English) => "Author: {}",
            (TranslationKey::ModsDependencies, Language::Russian) => "Зависимости:",
            (TranslationKey::ModsDependencies, Language::English) => "Dependencies:",

            (TranslationKey::CrashSummaryHardCrash, Language::Russian) => "В логе зафиксирован фатальный краш процесса (CTD). Найдено виновников: {}.",
            (TranslationKey::CrashSummaryHardCrash, Language::English) => "Fatal process crash (CTD) detected in log. Found culprits: {}.",
            (TranslationKey::CrashSummaryCulprits, Language::Russian) => "Обнаружены критические ошибки инициализации. Найдено проблемных модов: {}.",
            (TranslationKey::CrashSummaryCulprits, Language::English) => "Critical initialization errors detected. Found problematic mods: {}.",
            (TranslationKey::CrashSummaryErrorsOnly, Language::Russian) => "В логе найдено {} ошибок, но прямых виновников фатального сбоя не выявлено.",
            (TranslationKey::CrashSummaryErrorsOnly, Language::English) => "Found {} errors in the log, but no direct fatal crash culprits identified.",
            (TranslationKey::CrashSummaryClean, Language::Russian) => "Лог чист. Критических ошибок и вылетов не обнаружено.",
            (TranslationKey::CrashSummaryClean, Language::English) => "Log is clean. No critical errors or crashes detected.",
            (TranslationKey::CrashFileStats, Language::Russian) => "Файл: {} | Строк: {} | Ошибок: {}",
            (TranslationKey::CrashFileStats, Language::English) => "File: {} | Lines: {} | Errors: {}",
            (TranslationKey::CrashAnalyzing, Language::Russian) => "Анализ лога...",
            (TranslationKey::CrashAnalyzing, Language::English) => "Analyzing log...",
            (TranslationKey::CrashFileNotFound, Language::Russian) => "Файл Player.log не найден в стандартных папках",
            (TranslationKey::CrashFileNotFound, Language::English) => "Player.log file not found in standard directories",

            (TranslationKey::SettingsGameDetected, Language::Russian) => "Обнаружено: RimWorld {} | движок {}",
            (TranslationKey::SettingsGameDetected, Language::English) => "Detected: RimWorld {} | engine {}",
            (TranslationKey::SettingsLaunchExtraArgs, Language::Russian) => "Дополнительные аргументы:",
            (TranslationKey::SettingsLaunchExtraArgs, Language::English) => "Extra arguments:",
            (TranslationKey::SettingsLaunchUnverified, Language::Russian) => "не проверено на этой версии игры",
            (TranslationKey::SettingsLaunchUnverified, Language::English) => "not verified on this game version",
            (TranslationKey::SettingsLaunchWaylithHint, Language::Russian) => "RimWorld 1.7: -mod=id1;id2 — запустить только с этими модами (ModsConfig не меняется), -NoFileLog — без лога, -Headless — без окна",
            (TranslationKey::SettingsLaunchWaylithHint, Language::English) => "RimWorld 1.7: -mod=id1;id2 runs with only these mods (ModsConfig untouched), -NoFileLog disables the log, -Headless runs without a window",
            (TranslationKey::OptWaylithWarning, Language::Russian) => "RimWorld 1.7 сам сжимает текстуры модов в BC7 и хранит их в кэше (TextureCache). Конвертация в DDS на 1.7 не нужна и может ломать моды.",
            (TranslationKey::OptWaylithWarning, Language::English) => "RimWorld 1.7 compresses mod textures to BC7 by itself and caches them (TextureCache). DDS conversion is not needed on 1.7 and may break mods.",

            (TranslationKey::ConfirmDeleteTitle, Language::Russian) => "Подтвердите удаление",
            (TranslationKey::ConfirmDeleteTitle, Language::English) => "Confirm deletion",
            (TranslationKey::ConfirmDeleteBody, Language::Russian) => "Эти моды будут отписаны в Steam и перемещены в Корзину ({} шт.). Их можно восстановить из Корзины.",
            (TranslationKey::ConfirmDeleteBody, Language::English) => "These mods will be unsubscribed in Steam and moved to the Recycle Bin ({}). You can restore them from the Recycle Bin.",
            (TranslationKey::ConfirmDeleteBtn, Language::Russian) => "Удалить в Корзину",
            (TranslationKey::ConfirmDeleteBtn, Language::English) => "Move to Recycle Bin",
            (TranslationKey::BtnCancel, Language::Russian) => "Отмена",
            (TranslationKey::BtnCancel, Language::English) => "Cancel",
            (TranslationKey::TransNoneForLang, Language::Russian) => "Нет установленных переводов для языка: {}",
            (TranslationKey::TransNoneForLang, Language::English) => "No installed translations for language: {}",
            (TranslationKey::ModsNotOnDisk, Language::Russian) => "Нет на диске",
            (TranslationKey::ModsNotOnDisk, Language::English) => "Not on disk",
            (TranslationKey::ModsDuplicatesTitle, Language::Russian) => "Найдены копии одного мода ({}). В игре такие моды видны дважды — оставьте одну копию:",
            (TranslationKey::ModsDuplicatesTitle, Language::English) => "Same mod found more than once ({}). The game lists such mods twice — keep one copy:",
            (TranslationKey::CategoriesAll, Language::Russian) => "Все",
            (TranslationKey::CategoriesAll, Language::English) => "All",
            (TranslationKey::CategoriesMenu, Language::Russian) => "Категории",
            (TranslationKey::CategoriesMenu, Language::English) => "Categories",
            (TranslationKey::CategoriesReset, Language::Russian) => "Определять автоматически",
            (TranslationKey::CategoriesReset, Language::English) => "Detect automatically",
            (TranslationKey::CategoriesManual, Language::Russian) => "задано вручную",
            (TranslationKey::CategoriesManual, Language::English) => "set manually",
            (TranslationKey::CategoriesNone, Language::Russian) => "Категория не определена",
            (TranslationKey::CategoriesNone, Language::English) => "No category detected",
            (TranslationKey::ModsVersionBadge, Language::Russian) => "до {}",
            (TranslationKey::ModsVersionBadge, Language::English) => "up to {}",
            (TranslationKey::ModsVersionHint, Language::Russian) => "Мод не заявляет поддержку RimWorld {}. По словам Ludeon, моды для 1.6 на 1.7 работают полностью или частично — обычно это не проблема.",
            (TranslationKey::ModsVersionHint, Language::English) => "The mod does not declare RimWorld {}. Per Ludeon, 1.6 mods work fully or partially on 1.7 — usually not a problem.",
            (TranslationKey::ModsVersionSummary, Language::Russian) => "Активных модов без заявленной поддержки {}: {} — игра покажет «potentially unsupported», обычно они работают.",
            (TranslationKey::ModsVersionSummary, Language::English) => "Active mods not declaring {}: {} — the game shows \"potentially unsupported\", they usually work.",
            (TranslationKey::VersionPrepatcher, Language::Russian) => "Prepatcher не работает на RimWorld 1.7, пока его не обновят (по отзывам — пустое меню). Отключите его и моды, которым он нужен.",
            (TranslationKey::VersionPrepatcher, Language::English) => "Prepatcher does not work on RimWorld 1.7 until it is updated (reports: empty menu). Disable it and the mods that need it.",
            (TranslationKey::VersionHarmony, Language::Russian) => "RimWorld 1.7 требует Harmony 3, а установленный Harmony не заявляет поддержку 1.7. Обновите Harmony в Steam Workshop.",
            (TranslationKey::VersionHarmony, Language::English) => "RimWorld 1.7 requires Harmony 3, but the installed Harmony does not declare 1.7. Update Harmony on Steam Workshop.",
            (TranslationKey::OptFlip, Language::Russian) => "Переворачивать по вертикали",
            (TranslationKey::OptFlip, Language::English) => "Flip vertically",
            (TranslationKey::OptFlipHint, Language::Russian) => "Как делает RimPy. По умолчанию выключено: ToDDS, который работает в 1.6, не переворачивает. Включайте только если текстуры в игре вверх ногами.",
            (TranslationKey::OptFlipHint, Language::English) => "What RimPy does. Off by default: ToDDS, which works on 1.6, does not flip. Turn on only if textures appear upside down in game.",
            (TranslationKey::OptAllowOnWaylith, Language::Russian) => "Всё равно разрешить конвертацию на 1.7 (не рекомендуется)",
            (TranslationKey::OptAllowOnWaylith, Language::English) => "Allow conversion on 1.7 anyway (not recommended)",
            (TranslationKey::OptClearGameCache, Language::Russian) => "Сбросить кэш текстур игры",
            (TranslationKey::OptClearGameCache, Language::English) => "Reset game texture cache",
            (TranslationKey::OptClearGameCacheHint, Language::Russian) => "TextureCache и ShaderCache уйдут в Корзину; игра пересоберёт их при запуске. Помогает при битых или старых текстурах.",
            (TranslationKey::OptClearGameCacheHint, Language::English) => "TextureCache and ShaderCache go to the Recycle Bin; the game rebuilds them on start. Helps with broken or stale textures.",
            (TranslationKey::ModsDepIncompatible, Language::Russian) => "Несовместим",
            (TranslationKey::ModsDepIncompatible, Language::English) => "Incompatible",
            (TranslationKey::DepIncompatible, Language::Russian) => "несовместимых",
            (TranslationKey::DepIncompatible, Language::English) => "incompatible",
            (TranslationKey::SettingsBtnImportDb, Language::Russian) => "Импорт из файла (RimPy / RimSort)",
            (TranslationKey::SettingsBtnImportDb, Language::English) => "Import from file (RimPy / RimSort)",
            (TranslationKey::SettingsDbHint, Language::Russian) => "«Обновить» скачивает базу правил сообщества RimSort с GitHub (лицензия CC0). Можно также взять communityRules.json из папки RimPy.",
            (TranslationKey::SettingsDbHint, Language::English) => "\"Update\" downloads the RimSort community rules from GitHub (CC0 license). You can also import communityRules.json from a RimPy folder.",
            (TranslationKey::SettingsCheckSteam, Language::Russian) => "Проверить Steam API",
            (TranslationKey::SettingsCheckSteam, Language::English) => "Check Steam API",
            (TranslationKey::SettingsAutoDetect, Language::Russian) => "Определить пути автоматически",
            (TranslationKey::SettingsAutoDetect, Language::English) => "Detect paths automatically",
            (TranslationKey::SettingsSavedHint, Language::Russian) => "Выбранные вручную пути, язык и параметры запуска сохраняются между запусками.",
            (TranslationKey::SettingsSavedHint, Language::English) => "Hand-picked paths, language and launch options are kept between runs.",
            (TranslationKey::SettingsFont, Language::Russian) => "Шрифт интерфейса",
            (TranslationKey::SettingsFont, Language::English) => "Interface font",
            (TranslationKey::SettingsTextSize, Language::Russian) => "Размер текста:",
            (TranslationKey::SettingsTextSize, Language::English) => "Text size:",
            (TranslationKey::SettingsFontHint, Language::Russian) => "Если текст кажется размытым — попробуйте другой шрифт или размер побольше. Применяется сразу.",
            (TranslationKey::SettingsFontHint, Language::English) => "If text looks blurry, try another font or a larger size. Applies immediately.",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct I18n {
    pub lang: Language,
}

impl I18n {
    pub fn new(lang: Language) -> Self {
        ENGLISH.store(lang == Language::English, Ordering::Relaxed);
        Self { lang }
    }

    pub fn language(&self) -> Language {
        self.lang
    }

    pub fn set_language(&mut self, lang: Language) {
        ENGLISH.store(lang == Language::English, Ordering::Relaxed);
        self.lang = lang;
    }

    pub fn get(&self, key: TranslationKey) -> &'static str {
        key.translate(self.lang)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_fills_placeholders_in_order() {
        assert_eq!(fmt("Всего: {} | Активно: {}", &["50", "7"]), "Всего: 50 | Активно: 7");
        assert_eq!(fmt("{} {} {}", &["a", "b"]), "a b {}");
        assert_eq!(fmt("no placeholders", &["x"]), "no placeholders");
    }

    #[test]
    fn tr_follows_interface_language() {
        let _ru = I18n::new(Language::Russian);
        assert_eq!(tr!("Да", "Yes"), "Да");
        assert_eq!(tr!("Найдено: {}", "Found: {}", 3), "Найдено: 3");
        let _en = I18n::new(Language::English);
        assert_eq!(tr!("Да", "Yes"), "Yes");
        assert_eq!(tr!("Найдено: {}", "Found: {}", 3), "Found: 3");
        ENGLISH.store(false, Ordering::Relaxed);
    }
}
