//! Frame composition: chrome, splash, dashboard, overlays and notifications.

mod modals;
mod screens;
mod widgets;

use std::cell::RefCell;
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Sparkline};
use ratatui::Frame;

use crate::tui::app::{App, ContentKind, JobView, LogLine, MouseAction, Screen, SearchFocus};
use crate::tui::i18n;
use crate::tui::jobs::LogLevel;
use crate::tui::theme;
use crate::tui::ui::widgets::{account_name, centered_rect, hint, panel};

#[derive(Clone, Copy)]
struct Hitbox {
    rect: Rect,
    action: MouseAction,
}

thread_local! {
    static MOUSE_HITBOXES: RefCell<Vec<Hitbox>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn reset_mouse_hitboxes() {
    MOUSE_HITBOXES.with(|boxes| boxes.borrow_mut().clear());
}

pub(crate) fn register_mouse(rect: Rect, action: MouseAction) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    MOUSE_HITBOXES.with(|boxes| boxes.borrow_mut().push(Hitbox { rect, action }));
}

/// Topmost registered action under a terminal cell, if any.
pub(crate) fn mouse_action_at(column: u16, row: u16) -> Option<MouseAction> {
    MOUSE_HITBOXES.with(|boxes| {
        boxes
            .borrow()
            .iter()
            .rev()
            .find(|hitbox| {
                let rect = hitbox.rect;
                column >= rect.x
                    && column < rect.x + rect.width
                    && row >= rect.y
                    && row < rect.y + rect.height
            })
            .map(|hitbox| hitbox.action)
    })
}

pub fn render(frame: &mut Frame, app: &mut App) {
    reset_mouse_hitboxes();
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(
        Block::default().style(Style::default().fg(theme::TEXT).bg(theme::BG)),
        area,
    );
    if area.width < 52 || area.height < 16 {
        render_too_small(frame, area, app);
        return;
    }
    if app.splash_active() {
        render_splash(frame, area, app);
        return;
    }

    let t = if app.settings.reduced_motion {
        0.0
    } else {
        app.anim_time()
    };
    let compact = area.width < 96;
    let job_visible = app.job.is_some() && app.show_job_panel;
    let mut constraints = vec![
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(6),
        Constraint::Length(2),
    ];
    if compact {
        constraints.insert(2, Constraint::Length(1));
    }
    if job_visible {
        constraints.insert(if compact { 4 } else { 3 }, Constraint::Length(7));
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut index = 0;
    let header_area = chunks[index];
    index += 1;
    let rule_area = chunks[index];
    index += 1;
    let tabs_area = if compact {
        let extra = chunks[index];
        index += 1;
        Some(extra)
    } else {
        None
    };
    let body_area = chunks[index];
    index += 1;
    let job_area = if job_visible {
        let extra = chunks[index];
        index += 1;
        Some(extra)
    } else {
        None
    };
    let footer_area = chunks[index];

    render_header(frame, app, header_area, t);
    render_rule(frame, rule_area, t);
    if let Some(tabs) = tabs_area {
        render_tabs(frame, app, tabs, t);
    }

    let content_area = if compact {
        body_area
    } else {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(22),
                Constraint::Length(1),
                Constraint::Min(24),
            ])
            .split(body_area);
        render_sidebar(frame, app, columns[0], t);
        columns[2]
    };

    if app.search.open {
        screens::render_search(frame, app, content_area, t);
    } else {
        screens::render_screen(frame, app, content_area, t);
    }

    if let Some(job_area) = job_area {
        render_job_panel(frame, app, job_area, t);
    }
    render_footer(frame, app, footer_area, t);
    render_particles(frame, app, content_area);
    render_toasts(frame, app, area, t);
    if app.modal.is_some() {
        modals::render(frame, app, area, t);
    }
}

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::SURFACE)),
        area,
    );
    let left_width = if area.width >= 100 {
        34
    } else if area.width >= 72 {
        30
    } else {
        area.width.saturating_sub(26).max(14)
    };
    let right_width = if area.width >= 100 {
        46
    } else {
        area.width.saturating_sub(left_width).saturating_sub(1)
    };
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(left_width),
            Constraint::Min(0),
            Constraint::Length(right_width),
        ])
        .split(area);

    let mut brand_spans = vec![
        Span::styled(
            " ◆ ",
            Style::default()
                .fg(theme::mix(theme::GREEN, theme::LIME, theme::pulse(t, 1.3)))
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Hot-RS",
            Style::default()
                .fg(theme::LIME)
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "-Launcher",
            Style::default()
                .fg(theme::mix(theme::GREEN, theme::CYAN, 0.35))
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if left_width >= 30 {
        brand_spans.push(Span::styled(
            format!("  v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        ));
    }
    let brand = Line::from(brand_spans);
    frame.render_widget(
        Paragraph::new(brand).style(Style::default().bg(theme::SURFACE)),
        chunks[0],
    );

    let width = area.width;
    let mut right: Vec<Span<'static>> = Vec::new();
    let lang = app.lang();
    let has_account = app.current_account().is_some();
    right.push(Span::styled(
        if has_account { "● " } else { "○ " },
        Style::default()
            .fg(if has_account {
                theme::mix(theme::GREEN, theme::BG, 0.25 * (1.0 - theme::pulse(t, 2.0)))
            } else {
                theme::RED
            })
            .bg(theme::SURFACE),
    ));
    right.push(Span::styled(
        if has_account {
            account_name(app)
        } else {
            lang.pick("no account", "无账号").to_string()
        },
        Style::default()
            .fg(if has_account {
                theme::TEXT
            } else {
                theme::TEXT_DIM
            })
            .bg(theme::SURFACE)
            .add_modifier(Modifier::BOLD),
    ));
    if width >= 78 {
        right.push(Span::styled(
            "  ·  ",
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        ));
        right.push(Span::styled(
            app.settings
                .active_version
                .clone()
                .unwrap_or_else(|| lang.pick("no version", "无版本").into()),
            Style::default()
                .fg(theme::mix(theme::CYAN, theme::TEXT, 0.2))
                .bg(theme::SURFACE),
        ));
        if let Some(loader) = app.active_loader() {
            right.push(Span::styled(
                format!(" · {loader}"),
                Style::default().fg(theme::PURPLE).bg(theme::SURFACE),
            ));
        }
        right.push(Span::styled(
            format!(
                "  · {}",
                match app.settings.source.as_str() {
                    "official" => lang.pick("official", "官方"),
                    "bmcl" => "BMCL",
                    _ => lang.pick("auto", "自动"),
                }
            ),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(right))
            .alignment(Alignment::Right)
            .style(Style::default().bg(theme::SURFACE)),
        chunks[2],
    );

    if width >= 96 {
        let middle = Line::from(vec![
            Span::styled(
                "◆",
                Style::default().fg(theme::GREEN_DARK).bg(theme::SURFACE),
            ),
            Span::styled(
                app.lang().pick(" cli-first ", " 命令行优先 "),
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(middle)
                .alignment(Alignment::Center)
                .style(Style::default().bg(theme::SURFACE)),
            chunks[1],
        );
    }
}

fn render_rule(frame: &mut Frame, area: Rect, t: f64) {
    let width = area.width as usize;
    let mut spans = Vec::with_capacity(width);
    for index in 0..width {
        let position = index as f64 / width.max(1) as f64;
        let base = if position < 0.33 {
            theme::mix(theme::GREEN, theme::CYAN, position / 0.33)
        } else if position < 0.66 {
            theme::mix(theme::CYAN, theme::PURPLE, (position - 0.33) / 0.33)
        } else {
            theme::mix(theme::PURPLE, theme::GOLD, (position - 0.66) / 0.34)
        };
        let shimmer = ((position * 9.0 - t * 1.4).sin() * 0.5 + 0.5).powf(9.0);
        let color = theme::mix(theme::BG, base, 0.28 + shimmer * 0.5);
        spans.push(Span::styled("─", Style::default().fg(color).bg(theme::BG)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_tabs(frame: &mut Frame, app: &App, area: Rect, _t: f64) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::SURFACE_ALT)),
        area,
    );
    let screens = Screen::ALL;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![
            Constraint::Ratio(1, screens.len() as u32);
            screens.len()
        ])
        .split(area);
    for (index, screen) in screens.iter().enumerate() {
        let active = *screen == app.screen;
        let style = if active {
            Style::default()
                .bg(theme::mix(theme::SURFACE_ALT, screen_color(*screen), 0.2))
                .fg(theme::mix(screen_color(*screen), theme::TEXT, 0.45))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme::TEXT_FAINT)
                .bg(theme::SURFACE_ALT)
        };
        let label = if area.width >= 74 {
            format!(" {} {} ", screen.icon(), screen.title(app.lang()))
        } else {
            format!(" {} ", screen.icon())
        };
        let spans = vec![Span::styled(label, style)];
        frame.render_widget(
            Paragraph::new(Line::from(spans))
                .alignment(Alignment::Center)
                .style(Style::default().bg(theme::SURFACE_ALT)),
            chunks[index],
        );
        register_mouse(chunks[index], MouseAction::Nav(*screen));
    }
}

fn render_sidebar(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER).bg(theme::SURFACE))
        .style(Style::default().fg(theme::TEXT).bg(theme::SURFACE));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 8 || inner.height < 8 {
        return;
    }
    let show_logo = inner.height >= 19;
    let logo_height = if show_logo { 8 } else { 0 };
    let footer_height = if inner.height >= 14 { 3 } else { 0 };
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(logo_height),
            Constraint::Min(6),
            Constraint::Length(footer_height),
        ])
        .split(inner);

    if show_logo {
        render_logo(frame, app, sections[0], t);
    }

    let nav_area = sections[1];
    let items = Screen::ALL;
    let row_step = if nav_area.height >= (items.len() * 2) as u16 {
        2
    } else {
        1
    };
    for (index, screen) in items.iter().enumerate() {
        let y = nav_area.y + (index * row_step) as u16;
        if y >= nav_area.y + nav_area.height {
            break;
        }
        let active = *screen == app.screen;
        let row = Rect {
            x: nav_area.x,
            y,
            width: nav_area.width,
            height: 1,
        };
        let color = screen_color(*screen);
        let bg = if active {
            theme::mix(theme::SURFACE, color, 0.15)
        } else {
            theme::SURFACE
        };
        frame.render_widget(Block::default().style(Style::default().bg(bg)), row);
        let mut spans = vec![Span::styled(
            if active { "▌" } else { " " },
            Style::default()
                .fg(theme::mix(color, theme::TEXT, theme::pulse(t, 2.0) * 0.3))
                .bg(bg),
        )];
        spans.push(Span::styled(
            format!(" {} ", screen.icon()),
            Style::default()
                .fg(if active { color } else { theme::TEXT_FAINT })
                .bg(bg)
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));
        spans.push(Span::styled(
            screen.title(app.lang()),
            Style::default()
                .fg(if active { theme::TEXT } else { theme::TEXT_DIM })
                .bg(bg)
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));
        if active {
            let right = format!("{} ", index + 1);
            let used =
                2 + screen.icon().chars().count() + 2 + screen.title(app.lang()).chars().count();
            if (used + right.len()) < row.width as usize {
                spans.push(Span::styled(
                    " ".repeat(row.width as usize - used - right.len()),
                    Style::default().bg(bg),
                ));
                spans.push(Span::styled(
                    right,
                    Style::default().fg(theme::TEXT_FAINT).bg(bg),
                ));
            }
        }
        let paragraph = Paragraph::new(Line::from(spans)).style(Style::default().bg(bg));
        frame.render_widget(paragraph, row);
        register_mouse(row, MouseAction::Nav(*screen));
    }

    if footer_height > 0 && sections[2].height > 0 {
        let footer = sections[2];
        let version = app
            .settings
            .active_version
            .clone()
            .unwrap_or_else(|| "—".into());
        let loader = app
            .active_loader()
            .unwrap_or_else(|| app.lang().pick("Vanilla", "原版").into());
        let lines = vec![
            Line::from(vec![
                Span::styled(" ◈ ", Style::default().fg(theme::GOLD).bg(theme::SURFACE)),
                Span::styled(
                    app.settings.ram_label(),
                    Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
                ),
                Span::styled(
                    format!(
                        "  {} {}",
                        app.settings.threads,
                        app.lang().pick("threads", "线程")
                    ),
                    Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                ),
            ]),
            Line::from(vec![
                Span::styled(" ◆ ", Style::default().fg(theme::CYAN).bg(theme::SURFACE)),
                Span::styled(
                    theme::truncate_chars(&version, 10),
                    Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
                ),
                Span::styled(
                    format!("  {}", theme::truncate_chars(&loader, 9)),
                    Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                ),
            ]),
            Line::from(vec![Span::styled(
                format!(" ↻ {}", theme::format_duration(app.session_start.elapsed())),
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            )]),
        ];
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(theme::SURFACE)),
            footer,
        );
    }
}

fn render_logo(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let bitmap = [
        "..######..",
        ".########.",
        "##..##..##",
        "##..##..##",
        "##########",
        "####..####",
        "####..####",
        "####..####",
        "##########",
        "..######..",
    ];
    let glow = 0.55 + theme::pulse(t, 1.25) * 0.45;
    let mut lines = Vec::new();
    for pair in bitmap.chunks(2) {
        let top = pair[0].chars().collect::<Vec<_>>();
        let bottom = pair[1].chars().collect::<Vec<_>>();
        let mut spans = Vec::with_capacity(10);
        for index in 0..10 {
            let top_lit = top.get(index) == Some(&'#');
            let bottom_lit = bottom.get(index) == Some(&'#');
            let shade = 0.45 + index as f64 / 18.0;
            let color = theme::mix(theme::GREEN_DARK, theme::LIME, shade * glow);
            let (glyph, fg, bg) = match (top_lit, bottom_lit) {
                (true, true) => ("█", color, theme::SURFACE),
                (true, false) => ("▀", color, theme::SURFACE),
                (false, true) => ("▄", color, theme::SURFACE),
                (false, false) => (" ", theme::SURFACE, theme::SURFACE),
            };
            spans.push(Span::styled(glyph, Style::default().fg(fg).bg(bg)));
        }
        lines.push(Line::from(spans));
    }
    let logo = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .style(Style::default().bg(theme::SURFACE));
    let logo_height = 5.min(area.height);
    let logo_rect = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: logo_height,
    };
    frame.render_widget(logo, logo_rect);
    if area.height > 5 {
        let word = Line::from(vec![
            Span::styled(
                "Hot-RS",
                Style::default()
                    .fg(theme::mix(theme::LIME, theme::TEXT, 0.15))
                    .bg(theme::SURFACE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "-Launcher",
                Style::default()
                    .fg(theme::mix(theme::GREEN, theme::CYAN, 0.4))
                    .bg(theme::SURFACE),
            ),
        ]);
        let word_rect = Rect {
            x: area.x,
            y: area.y + 5,
            width: area.width,
            height: 1,
        };
        frame.render_widget(Paragraph::new(word).alignment(Alignment::Center), word_rect);
    }
    if area.height > 6 {
        let version_rect = Rect {
            x: area.x,
            y: area.y + 6,
            width: area.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("v{}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            )]))
            .alignment(Alignment::Center),
            version_rect,
        );
    }
    let _ = app;
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::SURFACE)),
        area,
    );
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let lang = app.lang();
    let mut spans: Vec<Span<'static>> = Vec::new();
    if app.search.open {
        match app.search.focus {
            SearchFocus::Input => {
                spans.extend(hint("type", lang.pick("query", "查询"), theme::SURFACE));
                spans.extend(hint("enter", lang.pick("search", "搜索"), theme::SURFACE));
                spans.extend(hint("tab", lang.pick("results", "结果"), theme::SURFACE));
            }
            SearchFocus::Results => {
                spans.extend(hint("↑↓", lang.pick("select", "选择"), theme::SURFACE));
                spans.extend(hint("enter", lang.pick("install", "安装"), theme::SURFACE));
                spans.extend(hint("o", lang.pick("open", "打开"), theme::SURFACE));
                spans.extend(hint("tab", lang.pick("edit", "编辑"), theme::SURFACE));
            }
        }
        spans.extend(hint("esc", lang.pick("close", "关闭"), theme::SURFACE));
    } else if app.modal.is_some() {
        spans.extend(hint("↑↓", lang.pick("move", "移动"), theme::SURFACE));
        spans.extend(hint("enter", lang.pick("confirm", "确认"), theme::SURFACE));
        spans.extend(hint("esc", lang.pick("close", "关闭"), theme::SURFACE));
    } else {
        let mut items: Vec<(&str, &str)> = Vec::new();
        if app.job.as_ref().is_some_and(JobView::is_running) {
            items.push(("c", "cancel"));
        } else if app.job.as_ref().is_some_and(|job| job.finished.is_some()) {
            items.push(("esc", "dismiss"));
        }
        items.extend(match app.screen {
            Screen::Dashboard => vec![
                ("enter", lang.pick("play", "启动")),
                ("n", lang.pick("new version", "新版本")),
                ("i", lang.pick("loader", "加载器")),
                ("s", lang.pick("search mods", "搜索模组")),
            ],
            Screen::Versions => vec![
                ("↑↓", lang.pick("select", "选择")),
                ("enter", lang.pick("play", "启动")),
                ("v", lang.pick("set active", "设为活跃")),
                ("i", lang.pick("loader", "加载器")),
                ("n", lang.pick("new version", "新版本")),
            ],
            Screen::Mods => vec![
                ("↑↓", lang.pick("select", "选择")),
                ("space", lang.pick("toggle", "启停")),
                ("x", lang.pick("remove", "卸载")),
                ("s", lang.pick("browse", "浏览")),
                ("O", lang.pick("optimize", "优化")),
            ],
            Screen::Content => match app.content_kind {
                ContentKind::Saves => vec![
                    ("t", lang.pick("tab", "切换")),
                    ("enter", lang.pick("backup", "备份")),
                    ("i", lang.pick("import", "导入")),
                    ("x", lang.pick("delete", "删除")),
                    ("o", lang.pick("folder", "目录")),
                    ("v", lang.pick("version", "版本")),
                ],
                ContentKind::ResourcePacks => vec![
                    ("t", lang.pick("tab", "切换")),
                    ("d", lang.pick("browse", "浏览")),
                    ("space", lang.pick("toggle", "启停")),
                    ("i", lang.pick("import", "导入")),
                    ("x", lang.pick("delete", "删除")),
                    ("o", lang.pick("folder", "目录")),
                ],
                ContentKind::Shaders => vec![
                    ("t", lang.pick("tab", "切换")),
                    ("d", lang.pick("browse", "浏览")),
                    ("space", lang.pick("toggle", "启停")),
                    ("i", lang.pick("import", "导入")),
                    ("x", lang.pick("delete", "删除")),
                    ("o", lang.pick("folder", "目录")),
                ],
            },
            Screen::Accounts => vec![
                ("↑↓", lang.pick("select", "选择")),
                ("a", lang.pick("offline", "离线")),
                ("m", lang.pick("login", "登录")),
                ("d", lang.pick("default", "默认")),
                ("x", lang.pick("logout", "登出")),
            ],
            Screen::Settings => vec![
                ("↑↓", lang.pick("field", "选项")),
                ("←→", lang.pick("change", "调整")),
                ("enter", lang.pick("edit", "编辑")),
                ("R", lang.pick("reset", "重置")),
            ],
            Screen::Logs => vec![
                ("↑↓", lang.pick("scroll", "滚动")),
                ("g / G", lang.pick("top / follow", "顶部 / 跟随")),
                ("f", lang.pick("follow", "跟随")),
                ("c", lang.pick("clear", "清空")),
            ],
            Screen::About => vec![
                ("g", lang.pick("github", "github")),
                ("b", lang.pick("codeberg", "codeberg")),
            ],
        });
        // Reserve room for the always-present help and quit hints (20 cells
        // plus a little breathing room).
        let budget = area.width.saturating_sub(22) as usize;
        let mut used = 0usize;
        for (key, what) in items {
            let cost = key.chars().count() + what.chars().count() + 5;
            if used + cost > budget {
                break;
            }
            spans.extend(hint(key, what, theme::SURFACE));
            used += cost;
        }
        if used + 14 <= budget {
            spans.extend(hint("tab", lang.pick("switch", "切换"), theme::SURFACE));
        }
        spans.extend(hint("?", lang.pick("help", "帮助"), theme::SURFACE));
        spans.extend(hint("q", lang.pick("quit", "退出"), theme::SURFACE));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::SURFACE)),
        chunks[0],
    );

    let status = app.job_status_label();
    let status_color = app.job_status_color();
    let mut left_spans = vec![
        Span::styled(
            if app.job.as_ref().is_some_and(JobView::is_running) {
                format!(" {} ", theme::spinner(app.frame))
            } else {
                " ● ".to_string()
            },
            Style::default()
                .fg(theme::mix(
                    status_color,
                    theme::TEXT,
                    theme::pulse(t, 1.6) * 0.2,
                ))
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            status,
            Style::default()
                .fg(status_color)
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if area.width >= 80 && !app.job.as_ref().is_some_and(JobView::is_running) {
        left_spans.push(Span::styled(
            format!("  {}", app.game_dir.display()),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        ));
    }
    let left = Line::from(left_spans);
    let clock = utc_clock();
    let right_spans = if area.width >= 80 {
        vec![
            Span::styled(
                format!(
                    "{} {}  ·  ",
                    lang.pick("src", "源"),
                    match app.settings.source.as_str() {
                        "official" => lang.pick("official", "官方"),
                        "bmcl" => "BMCL",
                        _ => lang.pick("auto", "自动"),
                    }
                ),
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            ),
            Span::styled(
                format!(
                    "↻ {}  ·  {clock}",
                    theme::format_duration(app.session_start.elapsed())
                ),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]
    } else {
        vec![Span::styled(
            clock,
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        )]
    };
    let right = Line::from(right_spans);
    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(12),
            Constraint::Length(if area.width >= 80 { 40 } else { 18 }),
        ])
        .split(chunks[1]);
    frame.render_widget(
        Paragraph::new(left).style(Style::default().bg(theme::SURFACE)),
        bottom[0],
    );
    frame.render_widget(
        Paragraph::new(right)
            .alignment(Alignment::Right)
            .style(Style::default().bg(theme::SURFACE)),
        bottom[1],
    );
}

fn render_job_panel(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let Some(job) = app.job.as_ref() else {
        return;
    };
    let lang = app.lang();
    let (status_color, status_text) = match &job.finished {
        None => (theme::GOLD, lang.pick("RUNNING", "进行中")),
        Some(Ok(_)) => (theme::GREEN, lang.pick("COMPLETE", "完成")),
        Some(Err(_)) => (theme::RED, lang.pick("FAILED", "失败")),
    };
    let block = panel(
        format!("{} · {}", status_text, job.title),
        status_color,
        job.finished.is_none(),
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 3 || inner.width < 20 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    let phase_total = job.phase_total.max(1);
    let phase_index = job.phase_index.min(phase_total);
    let mut phase_spans = Vec::new();
    for step in 1..=phase_total.min(6) {
        let active = step == phase_index;
        let done = step < phase_index || job.finished.is_some();
        phase_spans.push(Span::styled(
            if done {
                "● "
            } else if active {
                "◉ "
            } else {
                "○ "
            },
            Style::default()
                .fg(if done {
                    theme::GREEN
                } else if active {
                    theme::mix(theme::GOLD, theme::TEXT, theme::pulse(t, 2.4) * 0.3)
                } else {
                    theme::TEXT_FAINT
                })
                .bg(theme::SURFACE),
        ));
    }
    phase_spans.push(Span::styled(
        format!(
            "  {}  ·  {}/{}  ·  {}",
            theme::truncate_chars(i18n::localize_job_phase(lang, &job.phase_label), 30),
            phase_index,
            phase_total,
            theme::format_duration(job.elapsed())
        ),
        Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
    ));
    frame.render_widget(
        Paragraph::new(Line::from(phase_spans)).style(Style::default().bg(theme::SURFACE)),
        rows[0],
    );

    let ratio = if job.phase_total > 1 || job.finished.is_some() {
        Some(job.display_ratio)
    } else {
        job.latest.as_ref().map(|_| job.display_ratio)
    };
    let bar_width = inner.width.saturating_sub(10).max(8);
    let mut bar_spans = widgets::progress_spans(
        ratio,
        bar_width,
        status_color,
        theme::mix(status_color, theme::CYAN, 0.45),
        t,
        theme::SURFACE,
    );
    if let Some(ratio) = ratio {
        bar_spans.push(Span::styled(
            format!("  {:>3.0}%", ratio * 100.0),
            Style::default()
                .fg(theme::mix(status_color, theme::TEXT, 0.25))
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        bar_spans.push(Span::styled(
            "  ···",
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(bar_spans)).style(Style::default().bg(theme::SURFACE)),
        rows[1],
    );

    let mut stats = vec![
        Span::styled(
            format!("  ↓ {} ", theme::format_bytes(job.total_bytes)),
            Style::default().fg(theme::CYAN).bg(theme::SURFACE),
        ),
        Span::styled(
            format!("· {} {} ", job.done_files, lang.pick("files", "个文件")),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        ),
    ];
    if job.rate > 1024.0 {
        stats.push(Span::styled(
            format!("· {}/s ", theme::format_bytes(job.rate as u64)),
            Style::default().fg(theme::GREEN).bg(theme::SURFACE),
        ));
    }
    if job.active_files > 0 {
        stats.push(Span::styled(
            format!("· {} {} ", job.active_files, lang.pick("active", "活跃")),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        ));
    }
    if let Some(latest) = job.current_transfer() {
        if let Some(total) = latest.total.filter(|total| *total > 0) {
            let percent = if latest.done {
                100.0
            } else {
                (latest.bytes as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
            };
            stats.push(Span::styled(
                format!("· {percent:.0}% "),
                Style::default()
                    .fg(theme::mix(theme::CYAN, theme::TEXT, 0.2))
                    .bg(theme::SURFACE),
            ));
        }
        stats.push(Span::styled(
            format!(
                "· {}",
                theme::truncate_chars(&file_label(lang, &latest.label), 28)
            ),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        ));
    }
    if job.finished.is_none() {
        stats.push(Span::styled(
            lang.pick("  [c] cancel", "  [c] 取消"),
            Style::default().fg(theme::GOLD).bg(theme::SURFACE),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(stats)).style(Style::default().bg(theme::SURFACE)),
        rows[2],
    );
    if job.finished.is_none() {
        let width = rows[2].width.min(18);
        register_mouse(
            Rect {
                x: rows[2].x + rows[2].width.saturating_sub(width),
                y: rows[2].y,
                width,
                height: 1,
            },
            MouseAction::JobCancel,
        );
    }

    if rows[3].height > 0 {
        let message = match &job.finished {
            Some(Ok(message)) => i18n::localize_job_result(lang, message),
            Some(Err(error)) => error.clone(),
            None => job
                .log_tail
                .back()
                .map(|(_, text)| text.clone())
                .unwrap_or_else(|| job.phase_label.clone()),
        };
        let color = match &job.finished {
            Some(Ok(_)) => theme::GREEN,
            Some(Err(_)) => theme::RED,
            None => theme::TEXT_DIM,
        };
        let show_spark = job.rate_history.len() > 1 && rows[3].width > 36;
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(if show_spark {
                vec![Constraint::Min(18), Constraint::Length(22)]
            } else {
                vec![Constraint::Min(18)]
            })
            .split(rows[3]);
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!(
                    "  {}{}",
                    if job.finished.is_some() {
                        lang.pick("esc dismiss  ·  ", "esc 关闭  ·  ")
                    } else {
                        ""
                    },
                    theme::truncate_chars(&message, split[0].width as usize)
                ),
                Style::default().fg(color).bg(theme::SURFACE),
            )]))
            .style(Style::default().bg(theme::SURFACE)),
            split[0],
        );
        if show_spark {
            let sparkline = Sparkline::default()
                .data(job.rate_history.iter().copied())
                .style(
                    Style::default()
                        .fg(theme::mix(theme::GREEN, theme::SURFACE, 0.15))
                        .bg(theme::SURFACE),
                );
            frame.render_widget(sparkline, split[1]);
        }
    }
}

fn file_label(lang: i18n::Lang, label: &str) -> String {
    match label {
        "" => lang.pick("download", "下载").into(),
        "Asset" => lang.pick("Asset", "资源").into(),
        "Asset index" => lang.pick("Asset index", "资源索引").into(),
        "Library" => lang.pick("Library", "库").into(),
        other => other.to_string(),
    }
}

fn render_toasts(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let width = area.width.saturating_sub(4).min(52);
    if width < 24 {
        return;
    }
    let job_height = if app.job.is_some() && app.show_job_panel {
        7
    } else {
        0
    };
    let bottom = area.y + area.height.saturating_sub(2 + job_height);
    for (slot, toast) in app.toasts.iter().rev().take(3).enumerate() {
        let age = toast.born.elapsed().as_secs_f64();
        let ttl = toast.ttl.as_secs_f64();
        let entry = (age / 0.22).clamp(0.0, 1.0);
        let exit = ((age - ttl) / 0.25).clamp(0.0, 1.0);
        let slide = (1.0 - theme::ease_out_back(entry)) * (width as f64 * 0.6);
        let x = (area.x + area.width).saturating_sub(width + 2) as i32 + slide as i32;
        let x = x.clamp(area.x as i32, (area.x + area.width) as i32 - width as i32) as u16;
        let y = bottom.saturating_sub(3 + slot as u16 * 4);
        if y < area.y || y + 3 > area.y + area.height {
            continue;
        }
        let rect = Rect {
            x,
            y,
            width,
            height: 3,
        };
        let (color, icon) = match toast.level {
            LogLevel::Ok => (theme::GREEN, "✓"),
            LogLevel::Warn => (theme::GOLD, "!"),
            LogLevel::Error => (theme::RED, "✕"),
            LogLevel::Debug => (theme::TEXT_FAINT, "·"),
            LogLevel::Info => (theme::CYAN, "i"),
        };
        let fade = exit * 0.85;
        let color = theme::mix(color, theme::BG, fade);
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(
                Style::default()
                    .fg(theme::mix(
                        color,
                        theme::BG,
                        0.25 + theme::pulse(t, 2.0) * 0.08,
                    ))
                    .bg(theme::SURFACE),
            )
            .style(Style::default().bg(theme::SURFACE));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let line = Line::from(vec![
            Span::styled(
                format!(" {icon} "),
                Style::default()
                    .fg(color)
                    .bg(theme::SURFACE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                theme::truncate_chars(&toast.text, inner.width.saturating_sub(3) as usize),
                Style::default()
                    .fg(theme::mix(theme::TEXT, color, 0.15))
                    .bg(theme::SURFACE),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(line).style(Style::default().bg(theme::SURFACE)),
            inner,
        );
        register_mouse(rect, MouseAction::Noop);
    }
}

fn render_particles(frame: &mut Frame, app: &App, area: Rect) {
    if app.particles.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    let buffer: &mut Buffer = frame.buffer_mut();
    for particle in &app.particles {
        let x = area.x as f64 + particle.x * area.width as f64;
        let y = area.y as f64 + particle.y * area.height as f64;
        if x < area.x as f64 || y < area.y as f64 {
            continue;
        }
        let x = x as u16;
        let y = y as u16;
        if x >= area.x + area.width || y >= area.y + area.height {
            continue;
        }
        let ratio = (particle.life / particle.max_life).clamp(0.0, 1.0);
        let color = theme::mix(theme::BG, particle.color, ratio);
        if let Some(cell) = buffer.cell_mut((x, y)) {
            cell.set_symbol(&particle.glyph.to_string());
            cell.set_fg(color);
        }
    }
}

fn render_splash(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(Block::default().style(Style::default().bg(theme::BG)), area);
    let width = area.width as usize;
    let height = area.height as usize;
    let buffer = frame.buffer_mut();
    for index in 0..70usize {
        let x = (index * 73 + 11) % width.max(1);
        let y = (index * 37 + 5) % height.max(1);
        let twinkle = ((index as f64 * 0.7 + app.anim_time() * 2.2).sin() * 0.5 + 0.5).powf(4.0);
        if twinkle < 0.08 {
            continue;
        }
        let color = theme::mix(theme::BG, theme::TEXT_FAINT, 0.2 + twinkle * 0.55);
        let glyph = if index % 11 == 0 { "✦" } else { "·" };
        if let Some(cell) = buffer.cell_mut((area.x + x as u16, area.y + y as u16)) {
            cell.set_symbol(glyph);
            cell.set_fg(color);
        }
    }

    let panel = centered_rect(area, 90, 58, 15);
    if panel.width < 24 || panel.height < 10 {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::mix(
            theme::GREEN,
            theme::CYAN,
            0.4 + theme::pulse(app.anim_time(), 2.0) * 0.2,
        )))
        .style(Style::default().bg(theme::SURFACE));
    let inner = block.inner(panel);
    frame.render_widget(block, panel);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);
    let logo = Paragraph::new(creeper_lines(app.anim_time()))
        .alignment(Alignment::Center)
        .style(Style::default().bg(theme::SURFACE));
    frame.render_widget(logo, rows[0]);
    frame.render_widget(
        Paragraph::new(Line::from(theme::gradient_text_bg(
            "MINECRAFT",
            theme::GREEN,
            theme::CYAN,
            theme::SURFACE,
            app.anim_time() * 1.3,
            true,
        )))
        .alignment(Alignment::Center)
        .style(Style::default().bg(theme::SURFACE)),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            app.lang()
                .pick("a terminal-native launcher", "原生终端 Minecraft 启动器"),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]))
        .alignment(Alignment::Center)
        .style(Style::default().bg(theme::SURFACE)),
        rows[2],
    );

    let total = crate::tui::app::SPLASH_MS as f64 / 1000.0;
    let remaining = app
        .splash_until
        .map(|until| until.saturating_duration_since(std::time::Instant::now()))
        .unwrap_or_default()
        .as_secs_f64();
    let ratio = (1.0 - remaining / total).clamp(0.0, 1.0);
    let bar_width = inner.width.saturating_sub(8).clamp(10, 34);
    let mut spans = widgets::progress_spans(
        Some(ratio),
        bar_width,
        theme::GREEN,
        theme::CYAN,
        app.anim_time(),
        theme::SURFACE,
    );
    let dots = ".".repeat((app.frame as usize / 8) % 4);
    spans.push(Span::styled(
        format!("  {}{dots}", app.lang().pick("loading", "加载中")),
        Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
    ));
    frame.render_widget(
        Paragraph::new(Line::from(spans))
            .alignment(Alignment::Center)
            .style(Style::default().bg(theme::SURFACE)),
        rows[3],
    );
    if rows[4].height > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                app.lang()
                    .pick("press any key or click to skip", "按任意键或点击跳过"),
                Style::default()
                    .fg(theme::dim(theme::TEXT_FAINT, 0.25))
                    .bg(theme::SURFACE),
            )]))
            .alignment(Alignment::Center)
            .style(Style::default().bg(theme::SURFACE)),
            rows[4],
        );
    }
}

fn creeper_lines(t: f64) -> Vec<Line<'static>> {
    let bitmap = [
        "..######..",
        ".########.",
        "##..##..##",
        "##..##..##",
        "##########",
        "####..####",
        "####..####",
        "####..####",
        "##########",
        "..######..",
    ];
    let glow = 0.55 + theme::pulse(t, 1.3) * 0.45;
    let mut lines = Vec::new();
    for pair in bitmap.chunks(2) {
        let top = pair[0].chars().collect::<Vec<_>>();
        let bottom = pair[1].chars().collect::<Vec<_>>();
        let mut spans = Vec::new();
        for index in 0..10 {
            let top_lit = top.get(index) == Some(&'#');
            let bottom_lit = bottom.get(index) == Some(&'#');
            let shade = 0.4 + index as f64 / 16.0;
            let color = theme::mix(theme::GREEN_DARK, theme::LIME, shade * glow);
            let (glyph, fg) = match (top_lit, bottom_lit) {
                (true, true) => ("█", color),
                (true, false) => ("▀", color),
                (false, true) => ("▄", color),
                (false, false) => (" ", theme::SURFACE),
            };
            spans.push(Span::styled(
                glyph,
                Style::default().fg(fg).bg(theme::SURFACE),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    lines
}

fn render_too_small(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.anim_time();
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            "Hot-RS-Launcher",
            Style::default()
                .fg(theme::GREEN)
                .bg(theme::BG)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            app.lang().pick("terminal too small", "终端窗口太小"),
            Style::default()
                .fg(theme::GOLD)
                .bg(theme::BG)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            format!(
                "{} 52×16 · {} {}×{}",
                app.lang().pick("need at least", "至少需要"),
                app.lang().pick("now", "当前"),
                area.width,
                area.height
            ),
            Style::default().fg(theme::TEXT_DIM).bg(theme::BG),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            format!(
                "{} {}",
                theme::spinner(app.frame),
                app.lang().pick("resize the window", "请调整窗口大小")
            ),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::BG),
        )]),
    ];
    let _ = t;
    lines.push(Line::from(""));
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(Style::default().bg(theme::BG)),
        area,
    );
}

pub fn screen_color(screen: Screen) -> Color {
    match screen {
        Screen::Dashboard => theme::GREEN,
        Screen::Versions => theme::CYAN,
        Screen::Mods => theme::PURPLE,
        Screen::Content => theme::ORANGE,
        Screen::Accounts => theme::GOLD,
        Screen::Settings => theme::BLUE,
        Screen::Logs => theme::PINK,
        Screen::About => theme::LIME,
    }
}

pub fn level_color(level: LogLevel) -> Color {
    match level {
        LogLevel::Info => theme::CYAN,
        LogLevel::Ok => theme::GREEN,
        LogLevel::Warn => theme::GOLD,
        LogLevel::Error => theme::RED,
        LogLevel::Debug => theme::TEXT_FAINT,
    }
}

pub fn level_icon(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "·",
        LogLevel::Ok => "✓",
        LogLevel::Warn => "!",
        LogLevel::Error => "✕",
        LogLevel::Debug => "·",
    }
}

fn utc_clock() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!(
        "{:02}:{:02} UTC",
        (seconds / 3600) % 24,
        (seconds / 60) % 60
    )
}

pub fn log_line(log: &LogLine, width: u16) -> Line<'static> {
    let mins = log.at.as_secs() / 60;
    let secs = log.at.as_secs() % 60;
    let color = level_color(log.level);
    let text_width = width.saturating_sub(13) as usize;
    Line::from(vec![
        Span::styled(
            format!("{mins:02}:{secs:02} "),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        ),
        Span::styled(
            format!("{} ", level_icon(log.level)),
            Style::default().fg(color).bg(theme::SURFACE),
        ),
        Span::styled(
            theme::truncate_chars(&log.text, text_width),
            Style::default()
                .fg(if log.level == LogLevel::Debug {
                    theme::TEXT_FAINT
                } else {
                    theme::TEXT_DIM
                })
                .bg(theme::SURFACE),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::DiscoverKind;
    use crate::progress::TransferEvent;
    use crate::tui::app::{
        Bootstrap, ConfirmAction, ContentKind, JobView, Modal, MouseAction, PickerItem, PickerKind,
        PickerState, Screen, SearchFocus, SearchState,
    };
    use crate::tui::input::TextInput;
    use crate::tui::jobs::LogLevel;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::time::{Duration, Instant};

    fn app_with_data() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("minecraft");
        let version = game.join("versions").join("1.21.4");
        std::fs::create_dir_all(version.join("mods")).unwrap();
        std::fs::write(
            version.join("1.21.4.json"),
            br#"{"id":"1.21.4","type":"release","mainClass":"net.minecraft.client.main.Main","libraries":[],"downloads":{"client":{"url":"https://example.org/client.jar"}}}"#,
        )
        .unwrap();
        std::fs::write(version.join("1.21.4.jar"), vec![0u8; 2048]).unwrap();
        std::fs::write(version.join("mods/sodium.jar"), vec![0u8; 1536]).unwrap();
        std::fs::write(version.join("mods/lithium.jar.disabled"), vec![0u8; 700]).unwrap();
        std::fs::create_dir_all(version.join("saves").join("World One")).unwrap();
        std::fs::write(
            version.join("saves").join("World One").join("level.dat"),
            b"fixture",
        )
        .unwrap();
        std::fs::write(
            version.join("saves").join("World One").join("icon.png"),
            b"png",
        )
        .unwrap();
        std::fs::create_dir_all(version.join("resourcepacks")).unwrap();
        {
            use std::io::Write;
            let file =
                std::fs::File::create(version.join("resourcepacks").join("Pack.zip")).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("pack.mcmeta", zip::write::FileOptions::default())
                .unwrap();
            zip.write_all(br#"{"pack":{"pack_format":15,"description":"Test pack"}}"#)
                .unwrap();
            zip.finish().unwrap();
        }
        std::fs::create_dir_all(version.join("shaderpacks")).unwrap();
        {
            use std::io::Write;
            let file =
                std::fs::File::create(version.join("shaderpacks").join("TestShader.zip")).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file(
                "shaders/shaders.properties",
                zip::write::FileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"profile.MEDIUM=\n").unwrap();
            zip.finish().unwrap();
        }
        std::fs::write(
            version.join("options.txt"),
            b"renderDistance:12\nresourcePacks:[\"file/Pack.zip\",\"vanilla\"]\nshaderPack:TestShader.zip\n",
        )
        .unwrap();
        let fabric_profile = game
            .join("libraries")
            .join("fabric")
            .join("fabric-profile-1.21.4.json");
        std::fs::create_dir_all(fabric_profile.parent().unwrap()).ok();
        std::fs::write(fabric_profile, b"{}").ok();
        crate::account::AccountManager::set_offline_metadata(&game, "Steve").unwrap();
        let launcher = crate::launcher::MinecraftLauncher::new(&game, 2).unwrap();
        let (tx, _rx) = crate::tui::jobs::channel();
        let mut app = App::new(
            launcher,
            tx,
            Bootstrap {
                no_splash: true,
                ..Bootstrap::default()
            },
        );
        app.settings.active_version = Some("1.21.4".into());
        app.refresh_local();
        for (level, text) in [
            (LogLevel::Info, "Version 1.21.4 detected"),
            (LogLevel::Ok, "Fabric profile ready"),
            (LogLevel::Warn, "A mod is disabled"),
            (LogLevel::Error, "Example error entry"),
            (LogLevel::Debug, "internal detail"),
        ] {
            app.add_log(level, text);
        }
        app.toast(LogLevel::Ok, "Ready to play");
        (app, dir)
    }

    fn dump_terminal(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer.cell((x, y)).map(|cell| cell.symbol()).unwrap_or(" "));
            }
            out.push('\n');
        }
        out
    }

    fn render_at(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        dump_terminal(&terminal)
    }

    #[test]
    fn about_screen_shows_author_and_both_repositories() {
        let (mut app, _dir) = app_with_data();
        app.splash_until = None;
        app.screen = Screen::About;
        let text = render_at(&mut app, 110, 34);
        assert!(text.contains("Hotsteel2901"), "{text}");
        assert!(
            text.contains("github.com/Hotsteel2901/mc_rs_cli_launcher"),
            "{text}"
        );
        assert!(
            text.contains("codeberg.org/hotsteel/mc_rs_cli_launcher"),
            "{text}"
        );
        assert!(text.contains("GPL-3.0-only"), "{text}");
        assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
    }

    #[test]
    fn renders_every_screen_at_common_sizes() {
        let (mut app, _dir) = app_with_data();
        assert_eq!(app.versions.len(), 1);
        assert_eq!(app.mods.len(), 2);
        assert_eq!(app.accounts.len(), 1);
        for (width, height) in [(120, 40), (100, 32), (80, 24), (60, 20), (52, 16)] {
            for screen in Screen::ALL {
                app.screen = screen;
                let text = render_at(&mut app, width, height);
                assert!(!text.trim().is_empty());
            }
        }
        app.screen = Screen::Dashboard;
        let tiny = render_at(&mut app, 40, 12);
        assert!(tiny.contains("too small"));
    }

    #[test]
    fn renders_java_picker_settings_rows_and_update_badges() {
        let (mut app, _dir) = app_with_data();
        app.splash_until = None;
        app.screen = Screen::Settings;
        app.settings_cursor = 7;
        let text = render_at(&mut app, 120, 40);
        assert!(text.contains("JVM arguments"), "{text}");
        assert!(text.contains("Garbage collector"), "{text}");
        assert!(
            text.contains("PROFILE PREVIEW") || text.contains("JVM"),
            "{text}"
        );

        app.settings_cursor = 6;
        app.settings_activate();
        let text = render_at(&mut app, 120, 40);
        assert!(text.contains("Auto-detect"), "{text}");
        assert!(
            text.contains("Java runtime") || text.contains("Java"),
            "{text}"
        );
        app.modal = None;

        app.mod_updates = vec![crate::update::ModUpdate {
            file: app.game_dir.join("versions/1.21.4/mods/sodium.jar"),
            filename: "sodium-2.jar".into(),
            enabled: true,
            project: "Sodium".into(),
            current: "0.5.0".into(),
            latest: "0.6.0".into(),
            download_url: "https://example.invalid/sodium.jar".into(),
            download_name: "sodium-2.jar".into(),
            download_sha1: None,
        }];
        app.refresh_mods();
        app.screen = Screen::Mods;
        for (width, height) in [(120, 40), (80, 24), (60, 20)] {
            let text = render_at(&mut app, width, height);
            assert!(text.contains("↑"), "{width}x{height}: {text}");
        }

        app.screen = Screen::Dashboard;
        let text = render_at(&mut app, 120, 40);
        assert!(
            text.contains("press u") || text.contains("可更新"),
            "{text}"
        );
    }

    #[test]
    fn discover_overlay_defaults_to_browse_and_shows_loader_badges() {
        let (mut app, _dir) = app_with_data();
        app.splash_until = None;
        app.search = SearchState::default();
        app.search.open = true;
        app.search.kind = DiscoverKind::Shaders;
        app.search.browse = true;
        app.search.total_hits = 1;
        app.search.results = crate::tui::app::parse_search_hits(&serde_json::json!({
            "hits": [{
                "slug": "bsl-shaders",
                "title": "BSL Shaders",
                "description": "Bright and colorful",
                "author": "CaptTatsu",
                "downloads": 42,
                "categories": ["iris", "optifine"],
                "project_type": "shader"
            }]
        }));
        app.search.list_state.select(Some(0));
        let text = render_at(&mut app, 110, 34);
        assert!(text.contains("BROWSE"), "{text}");
        assert!(text.contains("BSL Shaders"), "{text}");
        assert!(
            text.contains("Iris + OptiFine") || text.contains("shader loaders"),
            "{text}"
        );
    }

    #[test]
    fn discover_marks_installed_mods_and_optimization_category() {
        let (mut app, _dir) = app_with_data();
        app.splash_until = None;
        app.search = SearchState::default();
        app.search.open = true;
        app.search.kind = DiscoverKind::Mods;
        app.search.category = Some("optimization".into());
        app.search.total_hits = 2;
        app.search.results = crate::tui::app::parse_search_hits(&serde_json::json!({
            "hits": [
                {
                    "slug": "sodium", "title": "Sodium", "description": "Fast",
                    "author": "jellysquid3", "downloads": 10,
                    "categories": ["fabric", "optimization"], "project_type": "mod",
                    "_installed": true, "_installed_version": "0.6.0"
                },
                {
                    "slug": "lithium", "title": "Lithium", "description": "Fast logic",
                    "author": "jellysquid3", "downloads": 9,
                    "categories": ["fabric", "optimization"], "project_type": "mod"
                }
            ]
        }));
        app.search.list_state.select(Some(0));
        let text = render_at(&mut app, 110, 34);
        assert!(text.contains("Sodium"), "{text}");
        assert!(
            text.contains("installed") || text.contains("已安装"),
            "{text}"
        );
        assert!(
            text.contains("category optimization") || text.contains("分类 optimization"),
            "{text}"
        );
    }

    #[test]
    fn discover_pages_categories_and_browse_reset() {
        let (mut app, _dir) = app_with_data();
        app.search = SearchState::default();
        app.search.open = true;
        app.search.kind = DiscoverKind::Mods;
        app.search.focus = SearchFocus::Results;
        app.search.results = crate::tui::app::parse_search_hits(&serde_json::json!({
            "hits": [{
                "slug": "sodium", "title": "Sodium", "description": "Fast",
                "author": "jellysquid3", "downloads": 10,
                "categories": ["fabric", "optimization"], "project_type": "mod"
            }]
        }));
        app.search.list_state.select(Some(0));

        let _ = app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        assert_eq!(app.search.page, 1);
        app.search.focus = SearchFocus::Results;
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        assert_eq!(app.search.page, 0);

        app.search.focus = SearchFocus::Results;
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_eq!(app.search.category.as_deref(), Some("optimization"));
        app.search.focus = SearchFocus::Results;
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(app.search.category.is_none());

        app.search.input.set("sodium");
        app.search.browse = false;
        app.search.focus = SearchFocus::Results;
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert!(app.search.input.value().is_empty());
        assert!(app.search.browse);
    }

    #[test]
    fn mods_optimization_shortcut_opens_confirmation() {
        let (mut app, _dir) = app_with_data();
        app.splash_until = None;
        app.screen = Screen::Mods;
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT));
        assert!(matches!(
            app.modal,
            Some(Modal::Confirm {
                action: ConfirmAction::InstallOptimizations,
                ..
            })
        ));
    }

    #[test]
    fn settings_cursor_reaches_every_row() {
        let (mut app, _dir) = app_with_data();
        app.splash_until = None;
        app.screen = Screen::Settings;
        app.settings_cursor = 0;
        for _ in 0..20 {
            let _ = app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        assert_eq!(app.settings_cursor, crate::tui::app::SETTINGS_ROWS - 1);
    }

    #[test]
    fn content_screen_toggles_packs_and_deletes_worlds() {
        let (mut app, _dir) = app_with_data();
        app.splash_until = None;
        app.screen = Screen::Content;
        assert_eq!(app.content_kind, ContentKind::Saves);
        assert_eq!(app.saves.len(), 1);
        assert_eq!(app.saves[0].folder, "World One");
        assert!(app.saves[0].has_icon);

        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert_eq!(app.content_kind, ContentKind::ResourcePacks);
        assert_eq!(app.resourcepacks.len(), 1);
        assert!(app.resourcepacks[0].enabled);

        let _ = app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(!app.resourcepacks[0].enabled);
        let options =
            std::fs::read_to_string(app.game_dir.join("versions/1.21.4/options.txt")).unwrap();
        assert!(!options.contains("file/Pack.zip"));
        assert!(options.contains("renderDistance:12"));

        let _ = app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(app.resourcepacks[0].enabled);

        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert_eq!(app.content_kind, ContentKind::Shaders);
        assert_eq!(app.shaders.len(), 1);
        assert!(app.shaders[0].enabled);
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(!app.shaders[0].enabled);
        let options =
            std::fs::read_to_string(app.game_dir.join("versions/1.21.4/options.txt")).unwrap();
        assert!(options.contains("shaderPack:OFF"));
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(app.shaders[0].enabled);

        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert_eq!(app.content_kind, ContentKind::Saves);
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(matches!(
            app.modal,
            Some(Modal::Confirm {
                action: ConfirmAction::DeleteSave(_),
                ..
            })
        ));
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.saves.is_empty());
        assert!(!app
            .game_dir
            .join("versions/1.21.4/saves/World One")
            .exists());
    }

    #[test]
    fn renders_content_screen_with_saves_and_packs() {
        let (mut app, _dir) = app_with_data();
        app.splash_until = None;
        app.screen = Screen::Content;
        let text = render_at(&mut app, 110, 34);
        assert!(text.contains("World One"), "{text}");
        assert!(text.contains("SAVES") || text.contains("存档"), "{text}");

        app.content_kind = ContentKind::ResourcePacks;
        let text = render_at(&mut app, 110, 34);
        assert!(text.contains("Pack.zip"), "{text}");
        assert!(
            text.contains("ENABLED") || text.contains("已启用"),
            "{text}"
        );
        let text = render_at(&mut app, 80, 24);
        assert!(text.contains("Pack.zip"), "{text}");
        app.content_kind = ContentKind::Shaders;
        let text = render_at(&mut app, 110, 34);
        assert!(text.contains("TestShader.zip"), "{text}");
        let text = render_at(&mut app, 60, 20);
        assert!(!text.trim().is_empty(), "{text}");
    }

    #[test]
    fn mouse_switches_content_tabs() {
        let (mut app, _dir) = app_with_data();
        app.splash_until = None;
        app.screen = Screen::Content;
        let (x, y, action) = find_action(&mut app, 110, 34, |action| {
            matches!(action, MouseAction::ContentTab(1))
        })
        .expect("content tab hitbox");
        let _ = app.handle_mouse(left_click(x, y), Some(action));
        assert_eq!(app.content_kind, ContentKind::ResourcePacks);
        let (x, y, action) = find_action(&mut app, 110, 34, |action| {
            matches!(action, MouseAction::ContentTab(0))
        })
        .expect("content tab hitbox");
        let _ = app.handle_mouse(left_click(x, y), Some(action));
        assert_eq!(app.content_kind, ContentKind::Saves);
    }

    #[test]
    fn renders_overlays_jobs_splash_and_effects() {
        let (mut app, _dir) = app_with_data();
        app.screen = Screen::Mods;

        let mut job = JobView::new(1, "Download Minecraft 1.24");
        job.on_phase(3, 4, "Downloading assets".into());
        for (index, bytes) in [128u64, 512, 900].into_iter().enumerate() {
            job.on_transfer(TransferEvent {
                id: format!("asset-{index}"),
                label: "Asset".into(),
                bytes,
                total: Some(1024),
                done: index == 2,
            });
        }
        app.job = Some(job);
        let running = render_at(&mut app, 100, 30);
        assert!(running.contains("RUNNING"));
        if let Some(job) = app.job.as_mut() {
            job.finished = Some(Ok("done".into()));
            job.finished_at = Some(Instant::now());
        }
        let complete = render_at(&mut app, 100, 30);
        assert!(complete.contains("COMPLETE"));
        if let Some(job) = app.job.as_mut() {
            job.finished = Some(Err("network failed".into()));
            job.finished_at = Some(Instant::now());
        }
        let failed = render_at(&mut app, 100, 30);
        assert!(failed.contains("FAILED"));
        app.job = None;

        app.search.open = true;
        app.search.focus = SearchFocus::Results;
        app.search.results = crate::tui::app::parse_search_hits(&serde_json::json!({
            "hits": [{"slug":"sodium","title":"Sodium","description":"Fast renderer","author":"jelly","downloads":4200000,"categories":["fabric","optimization"]}]
        }));
        app.search.list_state.select(Some(0));
        assert!(render_at(&mut app, 100, 30).contains("Sodium"));
        app.search.open = false;

        app.modal = Some(Modal::Help { scroll: 0 });
        assert!(render_at(&mut app, 100, 30).contains("GLOBAL"));
        app.modal = Some(Modal::Confirm {
            title: "Uninstall mod".into(),
            lines: vec!["Remove sodium.jar?".into()],
            action: crate::tui::app::ConfirmAction::Quit,
            danger: true,
        });
        assert!(render_at(&mut app, 100, 30).contains("confirm"));
        app.modal = Some(Modal::Input {
            title: "Add offline account".into(),
            hint: "letters only".into(),
            input: TextInput::new("St"),
            action: crate::tui::app::InputAction::OfflineName,
            error: Some("Name too short".into()),
        });
        assert!(render_at(&mut app, 100, 30).contains("Name too short"));
        app.modal = Some(Modal::Picker(PickerState::new(
            "Download Minecraft",
            "enter select",
            PickerKind::RemoteVersions,
            vec![
                PickerItem {
                    id: "1.21.4".into(),
                    label: "1.21.4".into(),
                    meta: "release · 2024-12-03".into(),
                    tag: "release".into(),
                    installed: true,
                },
                PickerItem {
                    id: "24w01a".into(),
                    label: "24w01a".into(),
                    meta: "snapshot · 2024-01-03".into(),
                    tag: "snapshot".into(),
                    installed: false,
                },
            ],
        )));
        assert!(render_at(&mut app, 100, 30).contains("1.21.4"));
        if let Some(Modal::Picker(picker)) = app.modal.as_mut() {
            picker.loading = true;
            picker.items.clear();
            picker.filtered.clear();
        }
        let _ = render_at(&mut app, 100, 30);
        if let Some(Modal::Picker(picker)) = app.modal.as_mut() {
            picker.loading = false;
            picker.error = Some("offline".into());
        }
        assert!(render_at(&mut app, 100, 30).contains("offline"));
        app.modal = Some(Modal::Login { selected: 0 });
        assert!(render_at(&mut app, 100, 30).contains("Device code"));

        app.modal = None;
        app.splash_until = Some(Instant::now() + Duration::from_millis(1200));
        assert!(render_at(&mut app, 100, 30).contains("MINECRAFT"));
        app.splash_until = None;

        app.particles.push(crate::tui::app::Particle {
            x: 0.5,
            y: 0.5,
            vx: 0.0,
            vy: 0.0,
            life: 1.0,
            max_life: 1.0,
            glyph: '✦',
            color: theme::GOLD,
        });
        assert!(render_at(&mut app, 100, 30).contains('✦'));
    }

    fn find_action(
        app: &mut App,
        width: u16,
        height: u16,
        predicate: impl Fn(MouseAction) -> bool,
    ) -> Option<(u16, u16, MouseAction)> {
        let _ = render_at(app, width, height);
        for y in 0..height {
            for x in 0..width {
                if let Some(action) = mouse_action_at(x, y) {
                    if predicate(action) {
                        return Some((x, y, action));
                    }
                }
            }
        }
        None
    }

    fn left_click(x: u16, y: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::empty(),
        }
    }

    #[test]
    fn language_setting_switches_chinese_and_back() {
        let (mut app, _dir) = app_with_data();
        app.screen = Screen::Settings;
        let (x, y, action) = find_action(&mut app, 100, 30, |action| {
            matches!(action, MouseAction::SettingsIncrease(0))
        })
        .expect("language value hitbox");
        let _ = app.handle_mouse(left_click(x, y), Some(action));
        assert_eq!(app.settings.language, "zh");
        let chinese = render_at(&mut app, 100, 30);
        let compact: String = chinese.chars().filter(|ch| !ch.is_whitespace()).collect();
        assert!(compact.contains("偏好设置"), "{chinese}");
        assert!(compact.contains("界面语言"), "{chinese}");

        let (x, y, action) = find_action(&mut app, 100, 30, |action| {
            matches!(action, MouseAction::SettingsDecrease(0))
        })
        .expect("language value hitbox");
        let _ = app.handle_mouse(left_click(x, y), Some(action));
        assert_eq!(app.settings.language, "en");
        let english = render_at(&mut app, 100, 30);
        assert!(english.contains("PREFERENCES"), "{english}");

        app.screen = Screen::Settings;
        app.settings_cursor = 0;
        let _ = app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::empty()));
        assert_eq!(app.settings.language, "zh");
        let _ = app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::empty()));
        assert_eq!(app.settings.language, "en");
    }

    #[test]
    fn chinese_mode_localizes_all_main_screens_and_modals() {
        let (mut app, _dir) = app_with_data();
        app.settings.language = "zh".into();
        app.toasts.clear();
        let compact =
            |text: &str| -> String { text.chars().filter(|ch| !ch.is_whitespace()).collect() };
        for (screen, expected) in [
            (Screen::Versions, "版本详情"),
            (Screen::Mods, "模组详情"),
            (Screen::Content, "内容详情"),
            (Screen::Accounts, "账号详情"),
            (Screen::Logs, "活动"),
            (Screen::Settings, "偏好设置"),
            (Screen::About, "关于"),
        ] {
            app.screen = screen;
            app.modal = None;
            let text = compact(&render_at(&mut app, 110, 34));
            assert!(text.contains(expected), "missing {expected}: {text}");
        }

        app.screen = Screen::About;
        let text = compact(&render_at(&mut app, 110, 34));
        assert!(text.contains("两个托管平台上的仓库同名"), "{text}");
        assert!(text.contains("与Mojang/Microsoft无隶属关系"), "{text}");

        app.screen = Screen::Mods;
        app.search.open = true;
        app.search.results = crate::tui::app::parse_search_hits(&serde_json::json!({
            "hits": [{"slug":"sodium","title":"Sodium","description":"Fast renderer","author":"jelly","downloads":42,"categories":["fabric"]}]
        }));
        let text = compact(&render_at(&mut app, 110, 34));
        assert!(text.contains("结果"), "{text}");
        assert!(text.contains("作者"), "{text}");
        app.search.open = false;

        app.modal = Some(Modal::Help { scroll: 0 });
        let text = compact(&render_at(&mut app, 110, 34));
        assert!(text.contains("帮助"), "{text}");
        assert!(text.contains("全局"), "{text}");

        app.modal = Some(Modal::Login { selected: 0 });
        let text = compact(&render_at(&mut app, 110, 34));
        assert!(text.contains("Microsoft登录"), "{text}");
        assert!(text.contains("浏览器"), "{text}");
        app.modal = None;
    }

    #[test]
    fn mouse_can_navigate_click_rows_and_handle_modals() {
        let (mut app, _dir) = app_with_data();
        app.screen = Screen::Dashboard;
        let (x, y, action) = find_action(&mut app, 100, 30, |action| {
            matches!(action, MouseAction::Nav(Screen::Versions))
        })
        .expect("versions nav hitbox");
        let _ = app.handle_mouse(left_click(x, y), Some(action));
        assert_eq!(app.screen, Screen::Versions);

        app.screen = Screen::Mods;
        let (x, y, action) = find_action(&mut app, 100, 30, |action| {
            matches!(action, MouseAction::ModsRow(1))
        })
        .expect("mod row hitbox");
        let _ = app.handle_mouse(left_click(x, y), Some(action));
        assert_eq!(app.mod_sel, 1);

        app.modal = Some(Modal::Confirm {
            title: "Clear activity".into(),
            lines: vec!["Clear all entries?".into()],
            action: ConfirmAction::ClearLogs,
            danger: false,
        });
        let (x, y, action) = find_action(&mut app, 100, 30, |action| {
            matches!(action, MouseAction::ConfirmNo)
        })
        .expect("confirm no hitbox");
        let _ = app.handle_mouse(left_click(x, y), Some(action));
        assert!(app.modal.is_none());

        app.modal = Some(Modal::Picker(PickerState::new(
            "Download Minecraft",
            "enter select",
            PickerKind::RemoteVersions,
            vec![
                PickerItem {
                    id: "1.21.4".into(),
                    label: "1.21.4".into(),
                    meta: "release".into(),
                    tag: "release".into(),
                    installed: true,
                },
                PickerItem {
                    id: "1.21.3".into(),
                    label: "1.21.3".into(),
                    meta: "release".into(),
                    tag: "release".into(),
                    installed: false,
                },
            ],
        )));
        let (x, y, action) = find_action(&mut app, 100, 30, |action| {
            matches!(action, MouseAction::PickerRow(1))
        })
        .expect("picker row hitbox");
        let _ = app.handle_mouse(left_click(x, y), Some(action));
        match app.modal.as_ref() {
            Some(Modal::Picker(picker)) => assert_eq!(picker.selected, 1),
            _ => panic!("picker modal missing"),
        }
        let _ = app.handle_mouse(left_click(x, y), Some(action));
        assert!(matches!(app.modal, Some(Modal::Confirm { .. })));
    }

    #[test]
    fn mouse_click_skips_the_splash() {
        let (mut app, _dir) = app_with_data();
        app.splash_until = Some(Instant::now() + Duration::from_secs(30));
        assert!(app.splash_active());
        let _ = app.handle_mouse(left_click(1, 1), None);
        assert!(!app.splash_active());
    }

    #[test]
    fn mouse_wheel_moves_settings_cursor() {
        let (mut app, _dir) = app_with_data();
        app.screen = Screen::Settings;
        let _ = render_at(&mut app, 100, 30);
        let scroll = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        let _ = app.handle_mouse(scroll, None);
        assert_eq!(app.settings_cursor, 1);
        let scroll = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        let _ = app.handle_mouse(scroll, None);
        assert_eq!(app.settings_cursor, 0);
    }

    #[test]
    fn renders_empty_workspace_and_onboarding() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = crate::launcher::MinecraftLauncher::new(dir.path(), 2).unwrap();
        let (tx, _rx) = crate::tui::jobs::channel();
        let mut app = App::new(
            launcher,
            tx,
            Bootstrap {
                no_splash: true,
                ..Bootstrap::default()
            },
        );
        let dashboard = render_at(&mut app, 100, 30);
        assert!(dashboard.contains("No instance yet"), "{dashboard}");
        assert!(dashboard.contains("No account"), "{dashboard}");
        for screen in Screen::ALL {
            app.screen = screen;
            let _ = render_at(&mut app, 80, 24);
        }
    }

    #[test]
    fn survives_extreme_content_and_minimum_sizes() {
        let (mut app, _dir) = app_with_data();
        let long = "A".repeat(400);
        if let Some(row) = app.mods.first_mut() {
            row.display = format!("{long}.jar");
            row.file = format!("{long}.jar");
        }
        if let Some(account) = app.accounts.first_mut() {
            account.username = "VeryLongFallbackName".into();
            account.uuid = "0123456789abcdef0123456789abcdef".into();
        }
        app.logs.push(LogLine {
            at: Duration::from_secs(9),
            level: LogLevel::Error,
            text: long.repeat(20),
        });
        app.toast(LogLevel::Warn, long.repeat(5));
        app.search.open = true;
        app.search.focus = SearchFocus::Results;
        app.search.input.set(long.clone());
        app.search.kind = crate::discover::DiscoverKind::Shaders;
        app.search.total_hits = u64::MAX;
        app.search.results = vec![crate::tui::app::SearchHit {
            title: long.clone(),
            slug: "extremely-long-slug".into(),
            description: long.repeat(4),
            author: long.clone(),
            downloads: u64::MAX,
            categories: long.repeat(3),
            loaders: vec!["iris".into(), "optifine".into()],
            installed: false,
            installed_version: None,
        }];
        app.search.list_state.select(Some(0));

        let mut job = JobView::new(9, long.repeat(3));
        job.on_phase(3, 4, long.repeat(2));
        job.on_transfer(TransferEvent {
            id: "x".into(),
            label: long.repeat(2),
            bytes: u64::MAX,
            total: Some(u64::MAX),
            done: false,
        });
        app.job = Some(job);

        for (width, height) in [(52, 16), (60, 20), (80, 24)] {
            for screen in Screen::ALL {
                app.search.open = screen == Screen::Mods;
                app.screen = screen;
                let _ = render_at(&mut app, width, height);
            }
        }
        app.search.open = false;

        app.modal = Some(Modal::Confirm {
            title: long.clone(),
            lines: vec![long.repeat(4); 20],
            action: crate::tui::app::ConfirmAction::Quit,
            danger: true,
        });
        let _ = render_at(&mut app, 52, 16);
        app.modal = Some(Modal::Input {
            title: long.clone(),
            hint: long.repeat(3),
            input: TextInput::new(long.repeat(2)),
            action: crate::tui::app::InputAction::JavaPath,
            error: Some(long.repeat(4)),
        });
        let _ = render_at(&mut app, 52, 16);
        let items = (0..500)
            .map(|index| PickerItem {
                id: format!("item-{index}"),
                label: long.clone(),
                meta: long.repeat(2),
                tag: "snapshot".into(),
                installed: index % 2 == 0,
            })
            .collect();
        let mut picker = PickerState::new(
            long.clone(),
            long.repeat(2),
            PickerKind::RemoteVersions,
            items,
        );
        picker.selected = 499;
        picker.list_state.select(Some(499));
        app.modal = Some(Modal::Picker(picker));
        let _ = render_at(&mut app, 52, 16);
        app.modal = Some(Modal::Login { selected: 1 });
        let _ = render_at(&mut app, 52, 16);
        app.modal = Some(Modal::Help { scroll: usize::MAX });
        let _ = render_at(&mut app, 52, 16);
        app.modal = None;
        app.splash_until = Some(Instant::now() + Duration::from_millis(500));
        let _ = render_at(&mut app, 52, 16);
        app.splash_until = None;
        let _ = render_at(&mut app, 20, 5);
    }

    #[ignore]
    #[test]
    fn preview_dashboard_for_manual_review() {
        let (mut app, _dir) = app_with_data();
        app.toasts.clear();
        app.screen = Screen::Dashboard;
        println!("=== DASHBOARD ===\n{}", render_at(&mut app, 110, 34));
        app.screen = Screen::Versions;
        println!("=== VERSIONS ===\n{}", render_at(&mut app, 110, 34));
        app.screen = Screen::Mods;
        println!("=== MODS ===\n{}", render_at(&mut app, 110, 34));
        app.screen = Screen::Accounts;
        println!("=== ACCOUNTS ===\n{}", render_at(&mut app, 110, 34));
        app.screen = Screen::Settings;
        println!("=== SETTINGS ===\n{}", render_at(&mut app, 110, 34));
        app.screen = Screen::Logs;
        println!("=== LOGS ===\n{}", render_at(&mut app, 110, 34));
        app.screen = Screen::About;
        println!("=== ABOUT ===\n{}", render_at(&mut app, 110, 34));
        app.screen = Screen::Mods;
        app.search.open = true;
        app.search.focus = SearchFocus::Input;
        app.search.input.set("sodium");
        app.search.results = crate::tui::app::parse_search_hits(&serde_json::json!({
            "hits": [
                {"slug":"sodium","title":"Sodium","description":"Modern rendering engine that greatly improves performance","author":"jelly","downloads":4200000,"categories":["fabric","optimization"]},
                {"slug":"sodium-extra","title":"Sodium Extra","description":"Features that shouldn't be in Sodium","author":"FlashyReese","downloads":2000000,"categories":["fabric"]}
            ]
        }));
        app.search.list_state.select(Some(0));
        println!("=== SEARCH ===\n{}", render_at(&mut app, 110, 34));
        app.search.open = false;
        app.modal = Some(Modal::Picker(PickerState::new(
            "Download Minecraft",
            "Type to filter · enter select · r refresh · esc close",
            PickerKind::RemoteVersions,
            vec![
                PickerItem {
                    id: "1.21.4".into(),
                    label: "1.21.4".into(),
                    meta: "release · 2024-12-03".into(),
                    tag: "release".into(),
                    installed: true,
                },
                PickerItem {
                    id: "1.21.3".into(),
                    label: "1.21.3".into(),
                    meta: "release · 2024-10-23".into(),
                    tag: "release".into(),
                    installed: false,
                },
                PickerItem {
                    id: "24w45a".into(),
                    label: "24w45a".into(),
                    meta: "snapshot · 2024-11-06".into(),
                    tag: "snapshot".into(),
                    installed: false,
                },
            ],
        )));
        println!("=== PICKER ===\n{}", render_at(&mut app, 110, 34));
        app.modal = Some(Modal::Help { scroll: 0 });
        println!("=== HELP ===\n{}", render_at(&mut app, 110, 34));
        app.modal = None;
        app.screen = Screen::Dashboard;
        println!("=== 80x24 ===\n{}", render_at(&mut app, 80, 24));
        println!("=== 60x20 ===\n{}", render_at(&mut app, 60, 20));
        app.toasts.clear();
        app.java_probing = false;
        app.java_major = Some(21);
        app.java_path = Some("/usr/lib/jvm/java-21/bin/java".into());
        app.screen = Screen::Dashboard;
        println!("=== CLEAN 100x30 ===\n{}", render_at(&mut app, 100, 30));

        app.settings.language = "zh".into();
        app.toasts.clear();
        for (name, screen) in [
            ("DASHBOARD", Screen::Dashboard),
            ("VERSIONS", Screen::Versions),
            ("MODS", Screen::Mods),
            ("ACCOUNTS", Screen::Accounts),
            ("SETTINGS", Screen::Settings),
            ("LOGS", Screen::Logs),
        ] {
            app.screen = screen;
            println!("=== ZH {name} ===\n{}", render_at(&mut app, 110, 34));
        }
        app.screen = Screen::Mods;
        app.search.open = true;
        app.search.focus = SearchFocus::Input;
        app.search.input.set("sodium");
        app.search.results = crate::tui::app::parse_search_hits(&serde_json::json!({
            "hits": [
                {"slug":"sodium","title":"Sodium","description":"Modern rendering engine that greatly improves performance","author":"jelly","downloads":4200000,"categories":["fabric","optimization"]}
            ]
        }));
        app.search.list_state.select(Some(0));
        println!("=== ZH SEARCH ===\n{}", render_at(&mut app, 110, 34));
        app.search.open = false;
        app.modal = Some(Modal::Help { scroll: 0 });
        println!("=== ZH HELP ===\n{}", render_at(&mut app, 110, 34));
        app.modal = Some(Modal::Confirm {
            title: "卸载模组".into(),
            lines: vec!["从当前版本移除 sodium.jar？".into()],
            action: ConfirmAction::ClearLogs,
            danger: true,
        });
        println!("=== ZH CONFIRM ===\n{}", render_at(&mut app, 110, 34));
        app.modal = None;
        app.settings.language = "en".into();

        for screen in [
            Screen::Versions,
            Screen::Mods,
            Screen::Accounts,
            Screen::Settings,
        ] {
            app.screen = screen;
            println!(
                "=== {:?} 80x24 ===\n{}",
                screen,
                render_at(&mut app, 80, 24)
            );
        }

        let mut job = JobView::new(42, "Download Minecraft 1.21.4");
        job.on_phase(3, 4, "Libraries and natives".into());
        for index in 0..8 {
            job.on_transfer(TransferEvent {
                id: format!("asset-{index}"),
                label: if index % 2 == 0 {
                    "Library".into()
                } else {
                    "Asset".into()
                },
                bytes: 512 * 1024 * (index + 1) as u64,
                total: Some(8 * 1024 * 1024),
                done: index == 7,
            });
        }
        job.tick(0.5);
        job.tick(0.5);
        app.job = Some(job);
        app.screen = Screen::Dashboard;
        println!("=== JOB 110x32 ===\n{}", render_at(&mut app, 110, 32));
        app.job = None;

        app.modal = Some(Modal::Confirm {
            title: "Uninstall mod".into(),
            lines: vec![
                "Remove sodium.jar from the active version?".into(),
                "The jar file is deleted from disk.".into(),
            ],
            action: crate::tui::app::ConfirmAction::Quit,
            danger: true,
        });
        println!("=== CONFIRM ===\n{}", render_at(&mut app, 90, 26));
        app.modal = Some(Modal::Login { selected: 1 });
        println!("=== LOGIN ===\n{}", render_at(&mut app, 90, 26));
        app.modal = Some(Modal::Input {
            title: "Add offline account".into(),
            hint: "1–16 letters, digits or underscores · enter save · esc cancel".into(),
            input: TextInput::new("Steve"),
            action: crate::tui::app::InputAction::OfflineName,
            error: None,
        });
        println!("=== INPUT ===\n{}", render_at(&mut app, 80, 24));
        app.modal = None;
        app.screen = Screen::Mods;
        app.search.open = true;
        app.search.focus = SearchFocus::Results;
        app.search.input.set("sodium");
        app.search.results = crate::tui::app::parse_search_hits(&serde_json::json!({
            "hits": [
                {"slug":"sodium","title":"Sodium","description":"Modern rendering engine that greatly improves performance","author":"jelly","downloads":4200000,"categories":["fabric","optimization"]},
                {"slug":"sodium-extra","title":"Sodium Extra","description":"Features that shouldn't be in Sodium","author":"FlashyReese","downloads":2000000,"categories":["fabric"]}
            ]
        }));
        app.search.list_state.select(Some(0));
        assert_eq!(app.search.input.value(), "sodium");
        let search_text = render_at(&mut app, 80, 24);
        assert!(
            search_text.contains("sodium"),
            "search input lost: {search_text}"
        );
        println!("=== SEARCH 80x24 ===\n{search_text}");
        app.search.open = false;
        app.screen = Screen::Dashboard;
        app.toast(LogLevel::Warn, "A background task was cancelled safely.");
        println!("=== TOAST ===\n{}", render_at(&mut app, 90, 26));
    }
}
