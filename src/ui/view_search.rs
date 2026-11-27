//! Search over the whole app (Ctrl+K or the magnifier in the side bar): tabs, Settings
//! sections, modpacks, saves and the main actions (mods have their own search on the Mods
//! tab). Arrows pick, Enter opens, Esc closes.

use eframe::egui::{self, CornerRadius, Ui, Vec2};

use super::components::*;
use super::icons::*;
use super::theme::{tint, THEME};
use crate::app::{App, AppTab};
use crate::i18n::tr;

/// What a result does when it's picked.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Tab(AppTab),
    SettingsSection(usize),
    Modpack(String),
    Save(String),
    Action(Action),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    AutoSort,
    SaveOrder,
    Play,
    Rescan,
    AnalyzeLog,
    CheckUpdates,
}

struct Hit {
    icon: egui::ImageSource<'static>,
    title: String,
    kind: &'static str,
    target: Target,
    /// Lower is better: 0 = the name starts with the query, 1 = a word does, 2 = it's inside.
    rank: u8,
}

/// How well `text` matches the lowercase `query` (None: not at all).
fn rank(text: &str, query: &str) -> Option<u8> {
    let text = text.to_lowercase();
    if text.starts_with(query) {
        Some(0)
    } else if text
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.starts_with(query))
    {
        Some(1)
    } else if text.contains(query) {
        Some(2)
    } else {
        None
    }
}

/// Where the palette keeps its state between frames (egui's memory, not the app).
fn open_id() -> egui::Id {
    egui::Id::unique("app_search_open")
}
fn query_id() -> egui::Id {
    egui::Id::unique("app_search_query")
}
fn pick_id() -> egui::Id {
    egui::Id::unique("app_search_pick")
}

/// Opens the search (the side bar's magnifier).
pub fn open(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.insert_temp(open_id(), true);
        d.insert_temp(query_id(), String::new());
        d.insert_temp(pick_id(), 0usize);
    });
}

fn collect(app: &App, query: &str) -> Vec<Hit> {
    let mut hits: Vec<Hit> = Vec::new();
    let push = |hits: &mut Vec<Hit>, icon, title: String, kind, target, limit: usize| {
        if let Some(r) = rank(&title, query) {
            if hits.iter().filter(|h| h.kind == kind).count() < limit {
                hits.push(Hit {
                    icon,
                    title,
                    kind,
                    target,
                    rank: r,
                });
            }
        }
    };
    let tab_kind = tr!("Вкладка", "Tab", "标签页");
    for tab in AppTab::ALL {
        if app.tab_visible(tab) {
            push(
                &mut hits,
                tab_icon(tab),
                app.tab_name(tab).to_string(),
                tab_kind,
                Target::Tab(tab),
                8,
            );
        }
    }
    let settings_kind = tr!("Настройки", "Settings", "设置");
    for (i, name) in super::view_settings::search_sections() {
        push(
            &mut hits,
            ICON_SETTINGS,
            name.to_string(),
            settings_kind,
            Target::SettingsSection(i),
            9,
        );
    }
    let action_kind = tr!("Действие", "Action", "操作");
    for (action, name) in [
        (Action::AutoSort, tr!("Авто-сортировка", "Auto-sort", "自动排序")),
        (
            Action::SaveOrder,
            tr!("Сохранить порядок", "Save the order", "保存顺序"),
        ),
        (Action::Play, tr!("Играть", "Play", "开始游戏")),
        (
            Action::Rescan,
            tr!("Пересканировать моды", "Rescan mods", "重新扫描模组"),
        ),
        (
            Action::AnalyzeLog,
            tr!("Анализировать лог", "Analyze the log", "分析日志"),
        ),
        (
            Action::CheckUpdates,
            tr!("Проверить обновления модов", "Check mod updates", "检查模组更新"),
        ),
    ] {
        let icon = match action {
            Action::AutoSort => ICON_SORT,
            Action::SaveOrder => ICON_SAVE,
            Action::Play => ICON_PLAY,
            Action::Rescan => ICON_REFRESH,
            Action::AnalyzeLog => ICON_ANALYZER,
            Action::CheckUpdates => ICON_STEAM,
        };
        push(
            &mut hits,
            icon,
            name.to_string(),
            action_kind,
            Target::Action(action),
            6,
        );
    }
    // Modpacks and saves only for a typed query.
    if !query.is_empty() {
        let set_kind = tr!("Сборка", "Modpack", "整合包");
        for l in &app.modpacks.lists {
            push(
                &mut hits,
                ICON_COPY,
                l.name.clone(),
                set_kind,
                Target::Modpack(l.name.clone()),
                5,
            );
        }
        let save_kind = tr!("Сохранение", "Save", "存档");
        for s in &app.saves.list {
            push(
                &mut hits,
                ICON_SAVE,
                s.name.clone(),
                save_kind,
                Target::Save(s.name.clone()),
                5,
            );
        }
    }
    // Best matches first; an empty query keeps the natural order (tabs, settings, actions).
    hits.sort_by_key(|h| h.rank);
    hits.truncate(24);
    hits
}

fn tab_icon(tab: AppTab) -> egui::ImageSource<'static> {
    match tab {
        AppTab::Mods => ICON_MODS,
        AppTab::Workshop => ICON_STEAM,
        AppTab::Sets => ICON_COPY,
        AppTab::Saves => ICON_SAVE,
        AppTab::Translations => ICON_TRANSLATIONS,
        AppTab::Optimizer => ICON_OPTIMIZER,
        AppTab::Analyzer => ICON_ANALYZER,
        AppTab::Settings => ICON_SETTINGS,
    }
}

fn run(app: &mut App, ctx: &egui::Context, target: Target) {
    match target {
        Target::Tab(tab) => app.current_tab = tab,
        Target::SettingsSection(i) => {
            app.current_tab = AppTab::Settings;
            super::view_settings::open_section(ctx, i);
        }
        Target::Modpack(name) => {
            app.current_tab = AppTab::Sets;
            app.modpacks.selected = Some(name);
        }
        Target::Save(name) => {
            app.current_tab = AppTab::Saves;
            app.saves.selected = Some(name);
        }
        Target::Action(action) => match action {
            Action::AutoSort => {
                app.current_tab = AppTab::Mods;
                app.sort_active_mods();
            }
            Action::SaveOrder => {
                app.save_config();
            }
            Action::Play => app.launch_game(),
            Action::Rescan => app.rescan_mods(),
            Action::AnalyzeLog => {
                app.current_tab = AppTab::Analyzer;
                app.run_log_analysis();
            }
            Action::CheckUpdates => {
                app.current_tab = AppTab::Mods;
                app.check_mod_updates();
            }
        },
    }
}

/// The search window, when open; Ctrl+K opens it from anywhere.
pub fn render(app: &mut App, ctx: &egui::Context) {
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
        open(ctx);
    }
    if !ctx.data(|d| d.get_temp::<bool>(open_id()).unwrap_or(false)) {
        return;
    }
    let mut query: String = ctx.data(|d| d.get_temp(query_id()).unwrap_or_default());
    let mut pick: usize = ctx.data(|d| d.get_temp(pick_id()).unwrap_or(0));
    let q = query.trim().to_lowercase();
    let hits = collect(app, &q);
    let (up, down, enter, esc) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    if down && pick + 1 < hits.len() {
        pick += 1;
    }
    if up && pick > 0 {
        pick -= 1;
    }
    pick = pick.min(hits.len().saturating_sub(1));

    let mut chosen: Option<Target> = if enter {
        hits.get(pick).map(|h| h.target.clone())
    } else {
        None
    };
    let width = (ctx.content_rect().width() - 64.0).clamp(320.0, 620.0);
    let modal = egui::Modal::new(egui::Id::unique("app_search_modal"))
        .frame(egui::Frame {
            inner_margin: egui::Margin::same(14),
            ..raised_frame(12)
        })
        .show(ctx, |ui| {
            ui.set_width(width);
            // The field takes the keyboard as soon as the window opens.
            let field = ui.add(
                egui::TextEdit::singleline(&mut query)
                    .hint_text(tr!(
                        "Поиск: вкладки, настройки, сборки, сохранения, действия…",
                        "Search: tabs, settings, modpacks, saves, actions…",
                        "搜索：标签页、设置、整合包、存档、操作……"
                    ))
                    .desired_width(f32::INFINITY)
                    .font(egui::FontId::proportional(16.0))
                    .margin(egui::Margin::symmetric(10, 8)),
            );
            if !field.has_focus() {
                field.request_focus();
            }
            ui.add_space(8.0);
            if hits.is_empty() {
                ui.label(
                    egui::RichText::new(tr!("Ничего не найдено", "Nothing found", "未找到")).color(THEME.text_muted),
                );
                return;
            }
            egui::ScrollArea::vertical()
                .max_height((ui.ctx().content_rect().height() * 0.6).max(200.0))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, hit) in hits.iter().enumerate() {
                        if hit_row(ui, hit, i == pick).clicked() {
                            chosen = Some(hit.target.clone());
                        }
                    }
                });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(tr!(
                    "↑ ↓ — выбрать · Enter — открыть · Esc — закрыть",
                    "↑ ↓ to pick · Enter to open · Esc to close",
                    "↑ ↓ 选择 · Enter 打开 · Esc 关闭"
                ))
                .color(THEME.text_muted)
                .size(11.0),
            );
        });

    let close = esc || modal.should_close() || chosen.is_some();
    if let Some(target) = chosen {
        run(app, ctx, target);
    }
    // A new query starts the pick from the top.
    let old_query: String = ctx.data(|d| d.get_temp(query_id()).unwrap_or_default());
    if old_query != query {
        pick = 0;
    }
    ctx.data_mut(|d| {
        d.insert_temp(open_id(), !close);
        d.insert_temp(query_id(), query);
        d.insert_temp(pick_id(), pick);
    });
}

/// One result: icon, name, and what it is on the right; the picked one is lit.
fn hit_row(ui: &mut Ui, hit: &Hit, picked: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        if picked {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), tint(THEME.accent_cyan));
            ui.scroll_to_rect(rect, None);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), THEME.bg_surface_hover);
        }
        let color = if picked {
            THEME.accent_cyan
        } else {
            THEME.text_secondary
        };
        let icon = egui::Rect::from_center_size(egui::pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(16.0));
        egui::Image::new(hit.icon.clone()).tint(color).paint_at(ui, icon);
        let kind =
            ui.painter()
                .layout_no_wrap(hit.kind.to_string(), egui::FontId::proportional(11.0), THEME.text_muted);
        let kind_w = kind.size().x;
        ui.painter().galley(
            egui::pos2(rect.right() - 10.0 - kind_w, rect.center().y - kind.size().y / 2.0),
            kind,
            THEME.text_muted,
        );
        let mut job = egui::text::LayoutJob::simple_singleline(
            hit.title.clone(),
            egui::FontId::proportional(14.0),
            THEME.text_primary,
        );
        job.wrap.max_width = (rect.width() - 36.0 - kind_w - 24.0).max(40.0);
        job.wrap.max_rows = 1;
        job.wrap.overflow_character = Some('…');
        let title = ui.painter().layout_job(job);
        ui.painter().galley(
            egui::pos2(rect.left() + 36.0, rect.center().y - title.size().y / 2.0),
            title,
            THEME.text_primary,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
