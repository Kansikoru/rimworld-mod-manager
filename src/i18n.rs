use serde::{Deserialize, Serialize};

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
            (TranslationKey::TransCleanNotice, Language::Russian) => "В папке модов нет лишних или установленных переводов.",
            (TranslationKey::TransCleanNotice, Language::English) => "No translation mods currently found.",
            (TranslationKey::TransWorkshopResultsTitle, Language::Russian) => "Результаты в Мастерской",
            (TranslationKey::TransWorkshopResultsTitle, Language::English) => "Steam Workshop Results",
            (TranslationKey::TransBtnSubscribe, Language::Russian) => "Подписаться",
            (TranslationKey::TransBtnSubscribe, Language::English) => "Subscribe",

            // Optimizer View
            (TranslationKey::OptTitle, Language::Russian) => "Оптимизатор текстур",
            (TranslationKey::OptTitle, Language::English) => "Texture Optimizer",
            (TranslationKey::OptSubtitle, Language::Russian) => "Конвертация текстур PNG/JPG в формат DDS (BC7) с мипмапами",
            (TranslationKey::OptSubtitle, Language::English) => "Convert PNG/JPG textures to DDS (BC7) format with mipmaps",
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
            (TranslationKey::SettingsBtnUpdateDb, Language::Russian) => "Обновить базу правил",
            (TranslationKey::SettingsBtnUpdateDb, Language::English) => "Update Rules DB",

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
            (TranslationKey::AppSortedMsg, Language::Russian) => "Порядок модов оптимизирован топологическим алгоритмом",
            (TranslationKey::AppSortedMsg, Language::English) => "Mod order optimized by topological algorithm",
            (TranslationKey::AppStatusBarStats, Language::Russian) => "Всего: {} | Активно: {}",
            (TranslationKey::AppStatusBarStats, Language::English) => "Total: {} | Active: {}",

            (TranslationKey::AnalyzerBrowseLog, Language::Russian) => "Выбрать файл лога вручную",
            (TranslationKey::AnalyzerBrowseLog, Language::English) => "Select log file manually",
            (TranslationKey::OptDdsTitle, Language::Russian) => "Формат DDS BC7 ускоряет игру",
            (TranslationKey::OptDdsTitle, Language::English) => "DDS (BC7) format accelerates the game",
            (TranslationKey::OptDdsDesc, Language::Russian) => "• Ускоряет загрузку в VRAM видеокарты без распаковки процессором на старте игры\n• Ускорение запуска с 5-10 минут до 40-60 секунд на крупных сборках (300+ модов)\n• Снижение потребления оперативной памяти RAM на 4-8 ГБ\n• Идеальная совместимость с модом 'Graphics Settings +' и 'Dubs Performance Analyzer'",
            (TranslationKey::OptDdsDesc, Language::English) => "• Speeds up loading into RAM/VRAM without CPU unpacking at game start\n• Speeds up launch from 5-10 minutes to 40-60 seconds on large modpacks (300+ mods)\n• Reduces RAM consumption by 4-8 GB\n• Perfect compatibility with 'Graphics Settings +' and 'Dubs Performance Analyzer'",
            (TranslationKey::TransSearchHint, Language::Russian) => "Нет результатов поиска. Введите название мода или выберите мод в списке.",
            (TranslationKey::TransSearchHint, Language::English) => "Search results: see mod name or select from list.",
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

            (TranslationKey::ModsBtnInfoPanel, Language::Russian) => "ℹ Панель информации",
            (TranslationKey::ModsBtnInfoPanel, Language::English) => "ℹ Info Panel",
            (TranslationKey::ModsCtxDeactivate, Language::Russian) => "✕ Деактивировать",
            (TranslationKey::ModsCtxDeactivate, Language::English) => "✕ Deactivate",
            (TranslationKey::ModsCtxDeactivateSelected, Language::Russian) => "✕ Деактивировать выделенные ({})",
            (TranslationKey::ModsCtxDeactivateSelected, Language::English) => "✕ Deactivate selected ({})",
            (TranslationKey::ModsCtxMoveUp, Language::Russian) => "▲ Переместить вверх",
            (TranslationKey::ModsCtxMoveUp, Language::English) => "▲ Move Up",
            (TranslationKey::ModsCtxMoveDown, Language::Russian) => "▼ Переместить вниз",
            (TranslationKey::ModsCtxMoveDown, Language::English) => "▼ Move Down",
            (TranslationKey::ModsCtxOpenFolder, Language::Russian) => "📁 Открыть папку мода",
            (TranslationKey::ModsCtxOpenFolder, Language::English) => "📁 Open Mod Folder",
            (TranslationKey::ModsCtxOpenWorkshop, Language::Russian) => "🌐 Открыть в Steam Workshop",
            (TranslationKey::ModsCtxOpenWorkshop, Language::English) => "🌐 Open in Steam Workshop",
            (TranslationKey::ModsCtxOptSkip, Language::Russian) => "Оптимизация: ПРОПУСКАТЬ (нажми чтобы вкл)",
            (TranslationKey::ModsCtxOptSkip, Language::English) => "Optimization: SKIP (click to enable)",
            (TranslationKey::ModsCtxOptAllow, Language::Russian) => "Оптимизация: РАЗРЕШЕНА (нажми чтобы откл)",
            (TranslationKey::ModsCtxOptAllow, Language::English) => "Optimization: ALLOWED (click to disable)",
            (TranslationKey::ModsDepNotInList, Language::Russian) => "Нет в списке",
            (TranslationKey::ModsDepNotInList, Language::English) => "Not in list",
            (TranslationKey::ModsDepEnable, Language::Russian) => "+ Включить",
            (TranslationKey::ModsDepEnable, Language::English) => "+ Enable",
            (TranslationKey::ModsDepNotFound, Language::Russian) => "Не найден",
            (TranslationKey::ModsDepNotFound, Language::English) => "Not found",
            (TranslationKey::ModsDepOrder, Language::Russian) => "Порядок",
            (TranslationKey::ModsDepOrder, Language::English) => "Order",
            (TranslationKey::ModsCtxActivate, Language::Russian) => "✚ Активировать",
            (TranslationKey::ModsCtxActivate, Language::English) => "✚ Activate",
            (TranslationKey::ModsCtxActivateSelected, Language::Russian) => "✚ Активировать выделенные ({})",
            (TranslationKey::ModsCtxActivateSelected, Language::English) => "✚ Activate selected ({})",
            (TranslationKey::ModsDetailsTitle, Language::Russian) => "Информация о моде",
            (TranslationKey::ModsDetailsTitle, Language::English) => "Mod Details",
            (TranslationKey::ModsHideInfoPanel, Language::Russian) => "Скрыть панель информации",
            (TranslationKey::ModsHideInfoPanel, Language::English) => "Hide info panel",
            (TranslationKey::ModsBadgeProtected, Language::Russian) => "Защищённый",
            (TranslationKey::ModsBadgeProtected, Language::English) => "Protected",
            (TranslationKey::ModsBtnFolder, Language::Russian) => "📁 Папка",
            (TranslationKey::ModsBtnFolder, Language::English) => "📁 Folder",
            (TranslationKey::ModsAuthor, Language::Russian) => "• Автор: {}",
            (TranslationKey::ModsAuthor, Language::English) => "• Author: {}",
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
            (TranslationKey::CrashAnalyzing, Language::Russian) => "Анализ лога {:?}...",
            (TranslationKey::CrashAnalyzing, Language::English) => "Analyzing log {:?}...",
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
            (TranslationKey::CategoriesMenu, Language::Russian) => "🏷 Категории",
            (TranslationKey::CategoriesMenu, Language::English) => "🏷 Categories",
            (TranslationKey::CategoriesReset, Language::Russian) => "↺ Определять автоматически",
            (TranslationKey::CategoriesReset, Language::English) => "↺ Detect automatically",
            (TranslationKey::CategoriesManual, Language::Russian) => "задано вручную",
            (TranslationKey::CategoriesManual, Language::English) => "set manually",
            (TranslationKey::CategoriesNone, Language::Russian) => "Категория не определена",
            (TranslationKey::CategoriesNone, Language::English) => "No category detected",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct I18n {
    pub lang: Language,
}

impl I18n {
    pub fn new(lang: Language) -> Self {
        Self { lang }
    }

    pub fn language(&self) -> Language {
        self.lang
    }

    pub fn set_language(&mut self, lang: Language) {
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
}
