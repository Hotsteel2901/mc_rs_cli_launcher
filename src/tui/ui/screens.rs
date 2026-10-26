//! Individual screens: dashboard, versions, mods, accounts, settings, logs,
//! about and the Modrinth search overlay.

use std::path::Path;
use std::time::Duration;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::content::{game_mode_label, ShaderSupport};
use crate::discover::DiscoverKind;
use crate::tui::app::{App, ContentKind, MouseAction, Screen, SearchFocus};
use crate::tui::i18n::Lang;
use crate::tui::theme;
use crate::tui::ui::log_line;
use crate::tui::ui::register_mouse;
use crate::tui::ui::widgets::{badge, empty_state, input_line, mini_panel, panel, soft_badge};

pub fn render_screen(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    match app.screen {
        Screen::Dashboard => dashboard(frame, app, area, t),
        Screen::Versions => versions(frame, app, area, t),
        Screen::Mods => mods(frame, app, area, t),
        Screen::Content => content(frame, app, area, t),
        Screen::Accounts => accounts(frame, app, area, t),
        Screen::Settings => settings(frame, app, area, t),
        Screen::Logs => logs(frame, app, area, t),
        Screen::About => about(frame, app, area, t),
    }
}

// ── dashboard ─────────────────────────────────────────────────────

fn dashboard(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    let hero_height = if area.height >= 26 {
        10
    } else if area.height >= 21 {
        8
    } else {
        7
    };
    let show_stats = area.height >= 18;
    let stats_height = if area.height >= 24 { 5 } else { 3 };
    let mut constraints = vec![Constraint::Length(hero_height), Constraint::Length(1)];
    if show_stats {
        constraints.push(Constraint::Length(stats_height));
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Min(4));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);
    let mut index = 0;
    hero(frame, app, rows[index], t);
    index += 2;
    if show_stats {
        stats(frame, app, rows[index], t);
        index += 2;
    }
    activity(frame, app, rows[index], t);
}

fn hero(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let lang = app.lang();
    let block = panel(
        lang.pick("INSTANCE", "实例"),
        theme::GREEN,
        true,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 3 || inner.width < 30 {
        return;
    }
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(58),
            Constraint::Length(1),
            Constraint::Min(20),
        ])
        .split(inner);
    let left = columns[0];
    let right = columns[2];

    let mut lines = vec![Line::from(theme::gradient_text_bg(
        "M I N E C R A F T",
        theme::GREEN,
        theme::CYAN,
        theme::SURFACE,
        t * 0.8,
        true,
    ))];
    if left.height >= 7 {
        lines.push(Line::from(theme::gradient_text_bg(
            "C L I   ·   L A U N C H E R",
            theme::CYAN,
            theme::PURPLE,
            theme::SURFACE,
            t * 0.55 + 1.0,
            false,
        )));
        lines.push(Line::from(""));
    }
    if app.versions.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            lang.pick("No instance yet.", "还没有游戏实例。"),
            Style::default()
                .fg(theme::TEXT)
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        )]));
    } else {
        let version = app
            .settings
            .active_version
            .clone()
            .unwrap_or_else(|| "?".into());
        let loader = app
            .active_loader()
            .unwrap_or_else(|| lang.pick("Vanilla", "原版").into());
        let mods = app
            .current_version()
            .map(|row| row.mods_enabled)
            .unwrap_or(0);
        lines.push(Line::from(vec![
            Span::styled("◆ ", Style::default().fg(theme::GREEN).bg(theme::SURFACE)),
            Span::styled(
                version,
                Style::default()
                    .fg(theme::TEXT)
                    .bg(theme::SURFACE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "  ·  {loader}  ·  {mods} {}",
                    if lang == Lang::Chinese {
                        "个模组"
                    } else if mods == 1 {
                        "mod"
                    } else {
                        "mods"
                    }
                ),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]));
    }
    if let Some(account) = app.current_account() {
        let expiry = app.account_expiry_label(account);
        lines.push(Line::from(vec![
            Span::styled(
                "◉ ",
                Style::default()
                    .fg(theme::mix(
                        theme::GREEN,
                        theme::TEXT,
                        theme::pulse(t, 2.0) * 0.25,
                    ))
                    .bg(theme::SURFACE),
            ),
            Span::styled(
                account.username.clone(),
                Style::default().fg(theme::TEXT).bg(theme::SURFACE),
            ),
            Span::styled(
                format!(
                    "  ·  {}{}",
                    account_kind_label(lang, &account.kind),
                    if expiry.is_empty() {
                        String::new()
                    } else {
                        format!("  ·  {expiry}")
                    }
                ),
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            ),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("◌ ", Style::default().fg(theme::RED).bg(theme::SURFACE)),
            Span::styled(
                lang.pick("No account · press a to add one", "没有账号 · 按 a 添加"),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]));
    }
    if app.versions.is_empty() && app.accounts.is_empty() && left.height as usize > lines.len() + 1
    {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("1 ", Style::default().fg(theme::GOLD).bg(theme::SURFACE)),
            Span::styled(
                lang.pick("account  ", "账号  "),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
            Span::styled("2 ", Style::default().fg(theme::GOLD).bg(theme::SURFACE)),
            Span::styled(
                lang.pick("version  ", "版本  "),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
            Span::styled("3 ", Style::default().fg(theme::GOLD).bg(theme::SURFACE)),
            Span::styled(
                lang.pick("play", "启动"),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(theme::SURFACE)),
        left,
    );

    let action = if app.versions.is_empty() {
        (
            "↓",
            lang.pick("DOWNLOAD", "下载"),
            lang.pick("press n", "按 n"),
            theme::GOLD,
        )
    } else if app.current_account().is_none() {
        (
            "◉",
            lang.pick("ADD ACCOUNT", "添加账号"),
            lang.pick("press a", "按 a"),
            theme::CYAN,
        )
    } else {
        (
            "▶",
            lang.pick("PLAY", "启动"),
            lang.pick("press enter", "按回车"),
            theme::GREEN,
        )
    };
    let action_area = Rect {
        x: right.x,
        y: right.y,
        width: right.width,
        height: right.height.min(4),
    };
    let action_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::mix(
            action.3,
            theme::TEXT,
            0.1 + theme::pulse(t, 2.2) * 0.25,
        )))
        .style(Style::default().bg(theme::mix(theme::SURFACE, action.3, 0.1)));
    let action_inner = action_block.inner(action_area);
    frame.render_widget(action_block, action_area);
    register_mouse(action_area, MouseAction::DashboardPrimary);
    let button_lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{}  ", action.0),
                Style::default()
                    .fg(theme::mix(
                        action.3,
                        theme::TEXT,
                        theme::pulse(t, 2.5) * 0.3,
                    ))
                    .bg(theme::mix(theme::SURFACE, action.3, 0.1))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                action.1,
                Style::default()
                    .fg(theme::TEXT)
                    .bg(theme::mix(theme::SURFACE, action.3, 0.1))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![Span::styled(
            action.2,
            Style::default()
                .fg(theme::TEXT_FAINT)
                .bg(theme::mix(theme::SURFACE, action.3, 0.1)),
        )]),
    ];
    frame.render_widget(
        Paragraph::new(button_lines)
            .alignment(Alignment::Center)
            .style(Style::default().bg(theme::mix(theme::SURFACE, action.3, 0.1))),
        action_inner,
    );
    if right.height > 4 {
        let detail_area = Rect {
            x: right.x,
            y: right.y + 5,
            width: right.width,
            height: right.height.saturating_sub(5),
        };
        let loader = app
            .active_loader()
            .unwrap_or_else(|| lang.pick("Vanilla", "原版").into());
        let details = vec![
            Line::from(vec![
                Span::styled(
                    lang.pick("RAM  ", "内存  "),
                    Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                ),
                Span::styled(
                    app.settings.ram_label(),
                    Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    lang.pick("LOAD ", "加载器 "),
                    Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                ),
                Span::styled(
                    loader,
                    Style::default().fg(theme::PURPLE).bg(theme::SURFACE),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    lang.pick("PATH ", "路径 "),
                    Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                ),
                Span::styled(
                    theme::truncate_chars(
                        &app.game_dir.display().to_string(),
                        right.width as usize - 6,
                    ),
                    Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                ),
            ]),
        ];
        frame.render_widget(
            Paragraph::new(details)
                .wrap(Wrap { trim: true })
                .style(Style::default().bg(theme::SURFACE)),
            detail_area,
        );
    }
}

fn stats(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
        ])
        .split(area);
    let lang = app.lang();
    let latest = app
        .versions
        .first()
        .map(|row| row.id.clone())
        .unwrap_or_else(|| lang.pick("none", "无").into());
    let active_mods = app.current_version();
    let mods_value = active_mods
        .map(|row| format!("{}", row.mods_enabled))
        .unwrap_or_else(|| "0".into());
    let mods_hint = if !app.mod_updates.is_empty() {
        if app.lang() == crate::tui::i18n::Lang::Chinese {
            format!("{} 个可更新 · 按 u 查看", app.mod_updates.len())
        } else {
            format!("{} update(s) · press u", app.mod_updates.len())
        }
    } else {
        active_mods
            .map(|row| format!("{} {}", row.mods_disabled, lang.pick("disabled", "已禁用")))
            .unwrap_or_else(|| lang.pick("no version", "无版本").into())
    };
    let java_value = if app.java_probing {
        format!(
            "{}{}",
            lang.pick("detecting", "检测中"),
            ".".repeat((app.frame as usize / 8) % 4)
        )
    } else if let Some(major) = app.java_major {
        format!("Java {major}")
    } else {
        lang.pick("not found", "未找到").into()
    };
    let java_hint = if app.java_probing {
        "scanning system…".to_string()
    } else {
        app.java_path
            .as_deref()
            .and_then(|path| {
                Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "install Java 17+".into())
    };
    stat_card(
        frame,
        chunks[0],
        lang.pick("VERSIONS", "版本"),
        &format!("{}", app.versions.len()),
        &latest,
        theme::CYAN,
        t,
    );
    register_mouse(chunks[0], MouseAction::StatVersions);
    stat_card(
        frame,
        chunks[1],
        lang.pick("MODS ACTIVE", "已启用模组"),
        &mods_value,
        &mods_hint,
        theme::PURPLE,
        t,
    );
    register_mouse(chunks[1], MouseAction::StatMods);
    stat_card(
        frame,
        chunks[2],
        lang.pick("MEMORY", "内存"),
        &app.settings.ram_label(),
        &format!("{} {}", app.settings.threads, lang.pick("threads", "线程")),
        theme::GOLD,
        t,
    );
    register_mouse(chunks[2], MouseAction::StatSettings);
    stat_card(
        frame,
        chunks[3],
        lang.pick("JAVA", "JAVA"),
        &java_value,
        &java_hint,
        theme::BLUE,
        t,
    );
    register_mouse(chunks[3], MouseAction::StatSettings);
}

fn stat_card(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    hint_text: &str,
    color: Color,
    t: f64,
) {
    let block = mini_panel(label, color, t);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = vec![
        Line::from(vec![Span::styled(
            value.to_string(),
            Style::default()
                .fg(theme::mix(color, theme::TEXT, 0.15))
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            theme::truncate_chars(hint_text, inner.width as usize),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
    ];
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::SURFACE)),
        inner,
    );
}

fn activity(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let block = panel(
        if app.lang() == crate::tui::i18n::Lang::Chinese {
            "最近活动 · l 查看完整日志"
        } else {
            "RECENT ACTIVITY  ·  l for the full log"
        },
        theme::PINK,
        false,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    register_mouse(area, MouseAction::DashboardActivity);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    if app.logs.is_empty() {
        empty_state(
            frame,
            area,
            "≡",
            app.lang().pick("Quiet in here", "暂无记录"),
            &[app.lang().pick(
                "Actions and background jobs appear here.",
                "操作和后台任务会显示在这里。",
            )],
            theme::PINK,
            t,
        );
        return;
    }
    let visible = inner.height as usize;
    let start = app.logs.len().saturating_sub(visible);
    let lines: Vec<Line> = app.logs[start..]
        .iter()
        .map(|log| log_line(log, inner.width))
        .collect();
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::SURFACE)),
        inner,
    );
}

// ── versions ──────────────────────────────────────────────────────

fn versions(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(44),
            Constraint::Length(1),
            Constraint::Percentage(55),
        ])
        .split(area);
    list_versions(frame, app, columns[0], t);
    version_detail(frame, app, columns[2], t);
}

fn list_versions(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    let block = panel(
        format!(
            "{} · {}",
            app.lang().pick("INSTALLED", "已安装"),
            app.versions.len()
        ),
        theme::CYAN,
        true,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.versions.is_empty() {
        empty_state(
            frame,
            area,
            "▤",
            app.lang().pick("No versions installed", "尚未安装版本"),
            &[
                app.lang().pick(
                    "Press n to browse Minecraft versions.",
                    "按 n 浏览 Minecraft 版本。",
                ),
                app.lang().pick(
                    "Downloads are checked and cached.",
                    "下载会校验摘要并使用缓存。",
                ),
            ],
            theme::CYAN,
            t,
        );
        return;
    }
    let items: Vec<ListItem> = app
        .versions
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let active = app
                .settings
                .active_version
                .as_deref()
                .is_some_and(|value| value == row.id);
            let selection = index == app.version_sel;
            let width = inner.width as usize;
            let marker = if active { "●" } else { "○" };
            let loader = row
                .loaders
                .first()
                .map(|value| shorten_loader(value))
                .unwrap_or_else(|| "—".into());
            let mods = if row.mods_enabled + row.mods_disabled > 0 {
                format!(
                    "{}{}",
                    row.mods_enabled + row.mods_disabled,
                    app.lang().pick("m", "个")
                )
            } else {
                String::new()
            };
            let right = format!("{loader:<4}{mods:>4}");
            let id_width = width.saturating_sub(right.chars().count() + 4);
            let line = Line::from(vec![
                Span::styled(
                    format!(" {marker} "),
                    Style::default()
                        .fg(if active {
                            theme::GREEN
                        } else {
                            theme::TEXT_FAINT
                        })
                        .bg(if selection {
                            theme::mix(theme::SURFACE, theme::CYAN, 0.16)
                        } else {
                            theme::SURFACE
                        }),
                ),
                Span::styled(
                    format!(
                        "{:<width$}",
                        theme::truncate_chars(&row.id, id_width),
                        width = id_width
                    ),
                    Style::default()
                        .fg(if selection {
                            theme::TEXT
                        } else {
                            theme::TEXT_DIM
                        })
                        .bg(if selection {
                            theme::mix(theme::SURFACE, theme::CYAN, 0.16)
                        } else {
                            theme::SURFACE
                        })
                        .add_modifier(if selection {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    right,
                    Style::default()
                        .fg(if selection {
                            theme::CYAN
                        } else {
                            theme::TEXT_FAINT
                        })
                        .bg(if selection {
                            theme::mix(theme::SURFACE, theme::CYAN, 0.16)
                        } else {
                            theme::SURFACE
                        }),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();
    let list = List::new(items)
        .highlight_symbol("▌")
        .highlight_style(Style::default().bg(theme::mix(theme::SURFACE, theme::CYAN, 0.16)))
        .style(Style::default().bg(theme::SURFACE));
    let visible = inner.height as usize;
    let offset = window_offset(app.version_sel, app.versions.len(), visible);
    let mut state = ListState::default()
        .with_offset(offset)
        .with_selected(Some(app.version_sel));
    frame.render_stateful_widget(list, inner, &mut state);
    let count = visible.min(app.versions.len().saturating_sub(offset));
    for slot in 0..count {
        register_mouse(
            Rect {
                x: inner.x,
                y: inner.y + slot as u16,
                width: inner.width,
                height: 1,
            },
            MouseAction::VersionsRow(offset + slot),
        );
    }
}

fn version_detail(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let lang = app.lang();
    let block = panel(
        lang.pick("VERSION DETAIL", "版本详情"),
        theme::CYAN,
        true,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(row) = app.selected_version() else {
        empty_state(
            frame,
            area,
            "▤",
            lang.pick("Select a version", "选择一个版本"),
            &[lang.pick(
                "Installed versions and their loader state appear here.",
                "这里显示已安装版本及其加载器状态。",
            )],
            theme::CYAN,
            t,
        );
        return;
    };
    if inner.height < 4 {
        return;
    }
    let is_active = app
        .settings
        .active_version
        .as_deref()
        .is_some_and(|value| value == row.id);
    let version_dir = app.game_dir.join("versions").join(&row.id);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                row.id.clone(),
                Style::default()
                    .fg(theme::mix(theme::CYAN, theme::TEXT, 0.15))
                    .bg(theme::SURFACE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default().bg(theme::SURFACE)),
            soft_badge(
                match row.kind.as_str() {
                    "release" => lang.pick("release", "正式版"),
                    "snapshot" => lang.pick("snapshot", "快照"),
                    other => other,
                }
                .to_string(),
                match row.kind.as_str() {
                    "release" => theme::GREEN,
                    "snapshot" => theme::GOLD,
                    _ => theme::PURPLE,
                },
                theme::SURFACE,
            ),
            Span::styled(" ", Style::default().bg(theme::SURFACE)),
            if is_active {
                badge(lang.pick("ACTIVE", "当前"), theme::GREEN, theme::SURFACE)
            } else {
                Span::styled("", Style::default().bg(theme::SURFACE))
            },
        ]),
        Line::from(""),
    ];
    lines.push(kv(
        lang.pick("Loader", "加载器"),
        if row.loaders.is_empty() {
            lang.pick("Vanilla", "原版").into()
        } else {
            row.loaders.join(" + ")
        },
        theme::PURPLE,
    ));
    lines.push(kv(
        lang.pick("Mods", "模组"),
        format!(
            "{} {} · {} {}",
            row.mods_enabled,
            lang.pick("enabled", "启用"),
            row.mods_disabled,
            lang.pick("disabled", "禁用")
        ),
        theme::PURPLE,
    ));
    lines.push(kv(
        lang.pick("Java", "Java"),
        row.java_major
            .map(|major| format!("Java {major}"))
            .unwrap_or_else(|| lang.pick("unknown", "未知").into()),
        theme::GREEN,
    ));
    lines.push(kv(
        lang.pick("Client jar", "客户端 JAR"),
        if row.jar_size == 0 {
            lang.pick("missing", "缺失").into()
        } else {
            theme::format_bytes(row.jar_size)
        },
        if row.jar_size == 0 {
            theme::RED
        } else {
            theme::TEXT_DIM
        },
    ));
    lines.push(kv(
        lang.pick("On disk", "占用空间"),
        theme::format_bytes(row.total_size),
        theme::TEXT_DIM,
    ));
    if row.base != row.id {
        lines.push(kv(
            lang.pick("Based on", "基于版本"),
            row.base.clone(),
            theme::CYAN,
        ));
    }
    lines.push(kv(
        lang.pick("Modified", "修改时间"),
        age_label(lang, row.age),
        theme::TEXT_DIM,
    ));
    lines.push(kv(
        lang.pick("Directory", "目录"),
        theme::truncate_chars(
            &version_dir.display().to_string(),
            inner.width.saturating_sub(15) as usize,
        ),
        theme::TEXT_FAINT,
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("◈ ", Style::default().fg(theme::GREEN).bg(theme::SURFACE)),
        Span::styled(
            if is_active {
                lang.pick("This is the active instance.", "这是当前活跃实例。")
            } else {
                lang.pick(
                    "Press v to make this the active instance.",
                    "按 v 将其设为活跃实例。",
                )
            },
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            " [enter] ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            lang.pick(" play this version", " 启动此版本"),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            " [i]     ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            lang.pick(" install a loader", " 选择 / 安装加载器"),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            " [n]     ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::GOLD)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            lang.pick(" download another version", " 下载其他版本"),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            " [e]     ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::PURPLE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            lang.pick(" export as .mrpack", " 导出为 .mrpack"),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            " [R] [c] ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::BLUE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            lang.pick(" rename / copy instance", " 重命名 / 复制实例"),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            " [d]     ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::RED)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            lang.pick(" delete instance", " 删除实例"),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        ),
    ]));
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(theme::SURFACE)),
        inner,
    );
    let _ = t;
}

// ── mods ──────────────────────────────────────────────────────────

fn mods(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(52),
            Constraint::Length(1),
            Constraint::Percentage(47),
        ])
        .split(area);
    list_mods(frame, app, columns[0], t);
    mod_detail(frame, app, columns[2], t);
}

fn list_mods(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    let lang = app.lang();
    let version = app
        .settings
        .active_version
        .clone()
        .unwrap_or_else(|| lang.pick("no version", "无版本").into());
    let enabled = app.mods.iter().filter(|row| row.enabled).count();
    let disabled = app.mods.len() - enabled;
    let checking = if app.updates_loading {
        if lang == crate::tui::i18n::Lang::Chinese {
            " · 检查更新…"
        } else {
            " · checking updates…"
        }
    } else {
        ""
    };
    let block = panel(
        format!(
            "{} · {version} · {enabled} {} / {disabled} {}{checking}",
            lang.pick("MODS", "模组"),
            lang.pick("on", "启用"),
            lang.pick("off", "禁用")
        ),
        theme::PURPLE,
        true,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.settings.active_version.is_none() {
        empty_state(
            frame,
            area,
            "❖",
            lang.pick("No active version", "没有活跃版本"),
            &[lang.pick(
                "Download a version, then install mods.",
                "先下载一个版本，再安装模组。",
            )],
            theme::PURPLE,
            t,
        );
        return;
    }
    if app.mods.is_empty() {
        empty_state(
            frame,
            area,
            "❖",
            lang.pick("No mods installed", "尚未安装模组"),
            &[
                lang.pick("Press s to search Modrinth.", "按 s 搜索 Modrinth。"),
                lang.pick(
                    "Mods install into this version's mods folder.",
                    "模组会安装到此版本的 mods 目录。",
                ),
            ],
            theme::PURPLE,
            t,
        );
        return;
    }
    let items: Vec<ListItem> = app
        .mods
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let selection = index == app.mod_sel;
            let bg = if selection {
                theme::mix(theme::SURFACE, theme::PURPLE, 0.16)
            } else {
                theme::SURFACE
            };
            let status = if row.enabled { "●" } else { "○" };
            let color = if row.enabled {
                theme::GREEN
            } else {
                theme::TEXT_FAINT
            };
            let max = inner.width.saturating_sub(16) as usize;
            let line = Line::from(vec![
                Span::styled(format!(" {status} "), Style::default().fg(color).bg(bg)),
                Span::styled(
                    format!(
                        "{:<max$}",
                        theme::truncate_chars(&row.display, max),
                        max = max
                    ),
                    Style::default()
                        .fg(if row.enabled {
                            theme::TEXT
                        } else {
                            theme::TEXT_DIM
                        })
                        .bg(bg)
                        .add_modifier(if selection {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    if row.update.is_some() { "↑ " } else { "  " },
                    Style::default()
                        .fg(if row.update.is_some() {
                            theme::GOLD
                        } else {
                            bg
                        })
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:>7}", short_bytes(row.size)),
                    Style::default()
                        .fg(if selection {
                            theme::PURPLE
                        } else {
                            theme::TEXT_FAINT
                        })
                        .bg(bg),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();
    let list = List::new(items)
        .highlight_symbol("▌")
        .highlight_style(Style::default().bg(theme::mix(theme::SURFACE, theme::PURPLE, 0.16)))
        .style(Style::default().bg(theme::SURFACE));
    let visible = inner.height as usize;
    let offset = window_offset(app.mod_sel, app.mods.len(), visible);
    let mut state = ListState::default()
        .with_offset(offset)
        .with_selected(Some(app.mod_sel));
    frame.render_stateful_widget(list, inner, &mut state);
    let count = visible.min(app.mods.len().saturating_sub(offset));
    for slot in 0..count {
        register_mouse(
            Rect {
                x: inner.x,
                y: inner.y + slot as u16,
                width: inner.width,
                height: 1,
            },
            MouseAction::ModsRow(offset + slot),
        );
    }
}

fn mod_detail(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let lang = app.lang();
    let block = panel(
        lang.pick("MOD DETAIL", "模组详情"),
        theme::PURPLE,
        true,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(row) = app.selected_mod() else {
        empty_state(
            frame,
            area,
            "❖",
            lang.pick("Select a mod", "选择一个模组"),
            &[
                lang.pick("Space toggles a mod.", "空格启用 / 禁用模组。"),
                lang.pick("x removes it from this version.", "x 从当前版本中卸载。"),
            ],
            theme::PURPLE,
            t,
        );
        return;
    };
    let mods_dir = app
        .settings
        .active_version
        .as_deref()
        .map(|version| app.game_dir.join("versions").join(version).join("mods"));
    let mut lines = vec![
        Line::from(vec![Span::styled(
            theme::truncate_chars(
                row.display.strip_suffix(".jar").unwrap_or(&row.display),
                inner.width.saturating_sub(2) as usize,
            ),
            Style::default()
                .fg(theme::mix(theme::PURPLE, theme::TEXT, 0.15))
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            if row.enabled {
                badge(lang.pick("ENABLED", "已启用"), theme::GREEN, theme::SURFACE)
            } else {
                badge(lang.pick("DISABLED", "已禁用"), theme::GOLD, theme::SURFACE)
            },
            Span::styled("  ", Style::default().bg(theme::SURFACE)),
            Span::styled(
                theme::format_bytes(row.size),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]),
        Line::from(""),
        kv(
            lang.pick("File", "文件"),
            theme::one_line(&row.file, 200),
            theme::TEXT_DIM,
        ),
        kv(
            lang.pick("Folder", "目录"),
            mods_dir
                .map(|path| {
                    theme::truncate_chars(
                        &path.display().to_string(),
                        inner.width.saturating_sub(14) as usize,
                    )
                })
                .unwrap_or_default(),
            theme::TEXT_FAINT,
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled("◈ ", Style::default().fg(theme::PURPLE).bg(theme::SURFACE)),
            Span::styled(
                lang.pick(
                    "Disabled mods are renamed with a .disabled suffix.",
                    "禁用模组会重命名并加上 .disabled 后缀。",
                ),
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " [space] ",
                Style::default()
                    .fg(theme::BG)
                    .bg(theme::PURPLE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                lang.pick(" toggle", " 启用 / 禁用"),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                " [x]     ",
                Style::default()
                    .fg(theme::BG)
                    .bg(theme::RED)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                lang.pick(" uninstall", " 卸载"),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                " [s]     ",
                Style::default()
                    .fg(theme::BG)
                    .bg(theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                lang.pick(" search Modrinth", " 搜索 Modrinth"),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                " [u] [U] ",
                Style::default()
                    .fg(theme::BG)
                    .bg(theme::GOLD)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                lang.pick(" check / apply updates", " 检查 / 应用更新"),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]),
    ];
    if let Some(latest) = row.update.as_ref() {
        lines.push(Line::from(""));
        lines.push(kv(
            lang.pick("Update", "更新"),
            format!("→ {latest}"),
            theme::GOLD,
        ));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(theme::SURFACE)),
        inner,
    );
}

// ── content (saves / resource packs) ─────────────────────────────

fn content(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(4)])
        .split(area);
    render_content_tabs(frame, app, rows[0], t);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(46),
            Constraint::Length(1),
            Constraint::Percentage(54),
        ])
        .split(rows[1]);
    content_list(frame, app, columns[0], t);
    content_detail(frame, app, columns[2], t);
}

fn render_content_tabs(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    let lang = app.lang();
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .split(area);
    for (index, kind) in ContentKind::ALL.iter().enumerate() {
        let active = *kind == app.content_kind;
        let color = theme::ORANGE;
        let bg = if active {
            theme::mix(theme::SURFACE, color, 0.18 + theme::pulse(t, 1.6) * 0.05)
        } else {
            theme::SURFACE
        };
        let count = match kind {
            ContentKind::Saves => app.saves.len(),
            ContentKind::ResourcePacks => app.resourcepacks.len(),
            ContentKind::Shaders => app.shaders.len(),
        };
        let marker = if active { "▸" } else { " " };
        let label = format!(" {marker} {} · {count} ", kind.title(lang));
        let style = if active {
            Style::default()
                .fg(theme::mix(color, theme::TEXT, 0.25))
                .bg(bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::TEXT_FAINT).bg(bg)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(label, style)]))
                .style(Style::default().bg(bg)),
            chunks[index],
        );
        register_mouse(chunks[index], MouseAction::ContentTab(index));
    }
}

fn content_list(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    let lang = app.lang();
    let version = app
        .settings
        .active_version
        .clone()
        .unwrap_or_else(|| lang.pick("no version", "无版本").into());
    let title = match app.content_kind {
        ContentKind::Saves => format!(
            "{} · {} · {}",
            lang.pick("SAVES", "存档"),
            app.saves.len(),
            version
        ),
        ContentKind::ResourcePacks => {
            let enabled = app.resourcepacks.iter().filter(|pack| pack.enabled).count();
            format!(
                "{} · {} / {} · {}",
                lang.pick("RESOURCE PACKS", "资源包"),
                enabled,
                app.resourcepacks.len(),
                version
            )
        }
        ContentKind::Shaders => {
            let enabled = app.shaders.iter().filter(|shader| shader.enabled).count();
            format!(
                "{} · {} / {} · {}",
                lang.pick("SHADER PACKS", "光影包"),
                enabled,
                app.shaders.len(),
                version
            )
        }
    };
    let block = panel(title, theme::ORANGE, true, t, theme::SURFACE);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.settings.active_version.is_none() {
        empty_state(
            frame,
            area,
            "▣",
            lang.pick("No active version", "没有活跃版本"),
            &[lang.pick(
                "Select an installed instance first.",
                "请先选择一个已安装实例。",
            )],
            theme::ORANGE,
            t,
        );
        return;
    }
    match app.content_kind {
        ContentKind::Saves => {
            if app.saves.is_empty() {
                empty_state(
                    frame,
                    area,
                    "▣",
                    lang.pick("No worlds yet", "还没有存档"),
                    &[
                        lang.pick(
                            "Start the game once, or press i to import a world zip.",
                            "先启动一次游戏，或按 i 导入存档压缩包。",
                        ),
                        lang.pick(
                            "Backups are stored under backups/saves.",
                            "备份会保存在 backups/saves。",
                        ),
                    ],
                    theme::ORANGE,
                    t,
                );
                return;
            }
            let items: Vec<ListItem> = app
                .saves
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let selection = index == app.save_sel;
                    let bg = if selection {
                        theme::mix(theme::SURFACE, theme::ORANGE, 0.16)
                    } else {
                        theme::SURFACE
                    };
                    let icon = if row.has_icon { "◆" } else { "◇" };
                    let icon_color = if row.hardcore {
                        theme::RED
                    } else {
                        theme::ORANGE
                    };
                    let right = format!("{:>9}", short_bytes(row.size));
                    let width = inner.width.saturating_sub(14) as usize;
                    let line = Line::from(vec![
                        Span::styled(format!(" {icon} "), Style::default().fg(icon_color).bg(bg)),
                        Span::styled(
                            format!(
                                "{:<width$}",
                                theme::truncate_chars(&row.name, width),
                                width = width
                            ),
                            Style::default()
                                .fg(if selection {
                                    theme::TEXT
                                } else {
                                    theme::TEXT_DIM
                                })
                                .bg(bg)
                                .add_modifier(if selection {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        ),
                        Span::styled(
                            right,
                            Style::default()
                                .fg(if selection {
                                    theme::ORANGE
                                } else {
                                    theme::TEXT_FAINT
                                })
                                .bg(bg),
                        ),
                    ]);
                    ListItem::new(line)
                })
                .collect();
            render_content_rows(
                frame,
                app,
                inner,
                items,
                app.saves.len(),
                app.save_sel,
                theme::ORANGE,
            );
        }
        ContentKind::ResourcePacks => {
            if app.resourcepacks.is_empty() {
                empty_state(
                    frame,
                    area,
                    "▣",
                    lang.pick("No resource packs", "没有资源包"),
                    &[
                        lang.pick(
                            "Press i to import a .zip or folder.",
                            "按 i 导入 .zip 或文件夹。",
                        ),
                        lang.pick(
                            "Enabled packs are stored in options.txt.",
                            "启用的资源包会写入 options.txt。",
                        ),
                    ],
                    theme::ORANGE,
                    t,
                );
                return;
            }
            let items: Vec<ListItem> = app
                .resourcepacks
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let selection = index == app.pack_sel;
                    let bg = if selection {
                        theme::mix(theme::SURFACE, theme::ORANGE, 0.16)
                    } else {
                        theme::SURFACE
                    };
                    let (status, color) = if !row.valid {
                        ("✕", theme::RED)
                    } else if row.enabled {
                        ("●", theme::GREEN)
                    } else {
                        ("○", theme::TEXT_FAINT)
                    };
                    let right = format!("{:>9}", short_bytes(row.size));
                    let width = inner.width.saturating_sub(14) as usize;
                    let line = Line::from(vec![
                        Span::styled(format!(" {status} "), Style::default().fg(color).bg(bg)),
                        Span::styled(
                            format!(
                                "{:<width$}",
                                theme::truncate_chars(&row.file, width),
                                width = width
                            ),
                            Style::default()
                                .fg(if selection {
                                    theme::TEXT
                                } else {
                                    theme::TEXT_DIM
                                })
                                .bg(bg)
                                .add_modifier(if selection {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        ),
                        Span::styled(
                            right,
                            Style::default()
                                .fg(if selection {
                                    theme::ORANGE
                                } else {
                                    theme::TEXT_FAINT
                                })
                                .bg(bg),
                        ),
                    ]);
                    ListItem::new(line)
                })
                .collect();
            render_content_rows(
                frame,
                app,
                inner,
                items,
                app.resourcepacks.len(),
                app.pack_sel,
                theme::ORANGE,
            );
        }
        ContentKind::Shaders => {
            if app.shaders.is_empty() {
                empty_state(
                    frame,
                    area,
                    "▣",
                    lang.pick("No shader packs", "没有光影包"),
                    &[
                        lang.pick(
                            "Press d to browse Modrinth, or i to import a .zip.",
                            "按 d 浏览 Modrinth，或按 i 导入 .zip。",
                        ),
                        lang.pick(
                            "Enabling writes shaderPack to options.txt.",
                            "启用状态会写入 options.txt 的 shaderPack。",
                        ),
                    ],
                    theme::ORANGE,
                    t,
                );
                return;
            }
            let items: Vec<ListItem> = app
                .shaders
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let selection = index == app.shader_sel;
                    let bg = if selection {
                        theme::mix(theme::SURFACE, theme::ORANGE, 0.16)
                    } else {
                        theme::SURFACE
                    };
                    let (status, color) = if row.enabled {
                        ("●", theme::GREEN)
                    } else {
                        ("○", theme::TEXT_FAINT)
                    };
                    let label = row.title.as_deref().unwrap_or(&row.file);
                    let right = format!("{:>9}", short_bytes(row.size));
                    let width = inner.width.saturating_sub(14) as usize;
                    let line = Line::from(vec![
                        Span::styled(format!(" {status} "), Style::default().fg(color).bg(bg)),
                        Span::styled(
                            format!(
                                "{:<width$}",
                                theme::truncate_chars(label, width),
                                width = width
                            ),
                            Style::default()
                                .fg(if selection {
                                    theme::TEXT
                                } else {
                                    theme::TEXT_DIM
                                })
                                .bg(bg)
                                .add_modifier(if selection {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        ),
                        Span::styled(
                            right,
                            Style::default()
                                .fg(if selection {
                                    theme::ORANGE
                                } else {
                                    theme::TEXT_FAINT
                                })
                                .bg(bg),
                        ),
                    ]);
                    ListItem::new(line)
                })
                .collect();
            render_content_rows(
                frame,
                app,
                inner,
                items,
                app.shaders.len(),
                app.shader_sel,
                theme::ORANGE,
            );
        }
    }
}

fn render_content_rows(
    frame: &mut Frame,
    _app: &mut App,
    inner: Rect,
    items: Vec<ListItem>,
    len: usize,
    selected: usize,
    color: Color,
) {
    let list = List::new(items)
        .highlight_symbol("▌")
        .highlight_style(Style::default().bg(theme::mix(theme::SURFACE, color, 0.16)))
        .style(Style::default().bg(theme::SURFACE));
    let visible = inner.height as usize;
    let offset = window_offset(selected, len, visible);
    let mut state = ListState::default()
        .with_offset(offset)
        .with_selected(Some(selected));
    frame.render_stateful_widget(list, inner, &mut state);
    let count = visible.min(len.saturating_sub(offset));
    for slot in 0..count {
        register_mouse(
            Rect {
                x: inner.x,
                y: inner.y + slot as u16,
                width: inner.width,
                height: 1,
            },
            MouseAction::ContentRow(offset + slot),
        );
    }
}

fn content_detail(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let lang = app.lang();
    let block = panel(
        lang.pick("CONTENT DETAIL", "内容详情"),
        theme::ORANGE,
        true,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match app.content_kind {
        ContentKind::Saves => {
            let Some(save) = app.selected_save() else {
                empty_state(
                    frame,
                    area,
                    "▣",
                    lang.pick("Select a world", "选择一个存档"),
                    &[lang.pick(
                        "Back up or delete the selected world here.",
                        "在这里备份或删除所选存档。",
                    )],
                    theme::ORANGE,
                    t,
                );
                return;
            };
            let version = save
                .version
                .clone()
                .unwrap_or_else(|| lang.pick("unknown", "未知").to_string());
            let mode = save
                .game_type
                .map(|code| game_mode_label(code, lang == Lang::Chinese))
                .unwrap_or_else(|| lang.pick("unknown", "未知"));
            let age = save
                .modified
                .and_then(|time| std::time::SystemTime::now().duration_since(time).ok());
            let path = app
                .settings
                .active_version
                .as_deref()
                .map(|id| {
                    app.game_dir
                        .join("versions")
                        .join(id)
                        .join("saves")
                        .join(&save.folder)
                })
                .unwrap_or_default();
            let mut lines = vec![
                Line::from(vec![
                    Span::styled(
                        theme::truncate_chars(&save.name, inner.width.saturating_sub(2) as usize),
                        Style::default()
                            .fg(theme::mix(theme::ORANGE, theme::TEXT, 0.2))
                            .bg(theme::SURFACE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  ", Style::default().bg(theme::SURFACE)),
                    if save.hardcore {
                        badge(lang.pick("HARDCORE", "极限"), theme::RED, theme::SURFACE)
                    } else {
                        Span::styled("", Style::default().bg(theme::SURFACE))
                    },
                ]),
                Line::from(""),
                kv(
                    lang.pick("Folder", "文件夹"),
                    save.folder.clone(),
                    theme::TEXT_DIM,
                ),
                kv(lang.pick("Version", "版本"), version, theme::CYAN),
                kv(
                    lang.pick("Game mode", "游戏模式"),
                    mode.to_string(),
                    theme::PURPLE,
                ),
                kv(
                    lang.pick("Size", "大小"),
                    theme::format_bytes(save.size),
                    theme::GOLD,
                ),
                kv(
                    lang.pick("Modified", "修改时间"),
                    age_label(lang, age),
                    theme::TEXT_DIM,
                ),
                kv(
                    lang.pick("Icon", "图标"),
                    if save.has_icon {
                        lang.pick("present", "有").to_string()
                    } else {
                        lang.pick("missing", "无").to_string()
                    },
                    theme::TEXT_FAINT,
                ),
                kv(
                    lang.pick("Path", "路径"),
                    theme::truncate_chars(
                        &path.display().to_string(),
                        inner.width.saturating_sub(14) as usize,
                    ),
                    theme::TEXT_FAINT,
                ),
                Line::from(""),
                Line::from(vec![
                    Span::styled("◈ ", Style::default().fg(theme::ORANGE).bg(theme::SURFACE)),
                    Span::styled(
                        lang.pick(
                            "Backups include region files, level.dat and icons.",
                            "备份包含区域文件、level.dat 和图标。",
                        ),
                        Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                    ),
                ]),
                Line::from(""),
            ];
            push_content_hints(
                &mut lines,
                lang,
                &[
                    ("b", lang.pick(" backup world", " 备份存档"), theme::GREEN),
                    ("i", lang.pick(" import world", " 导入存档"), theme::CYAN),
                    ("x", lang.pick(" delete world", " 删除存档"), theme::RED),
                    ("o", lang.pick(" open folder", " 打开目录"), theme::TEXT_DIM),
                ],
            );
            frame.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: true })
                    .style(Style::default().bg(theme::SURFACE)),
                inner,
            );
        }
        ContentKind::ResourcePacks => {
            let Some(pack) = app.selected_resourcepack() else {
                empty_state(
                    frame,
                    area,
                    "▣",
                    lang.pick("Select a resource pack", "选择一个资源包"),
                    &[lang.pick(
                        "Toggle, import or delete packs here.",
                        "在这里启停、导入或删除资源包。",
                    )],
                    theme::ORANGE,
                    t,
                );
                return;
            };
            let state = if !pack.valid {
                badge(lang.pick("INVALID", "无效"), theme::RED, theme::SURFACE)
            } else if pack.enabled {
                badge(lang.pick("ENABLED", "已启用"), theme::GREEN, theme::SURFACE)
            } else {
                badge(lang.pick("DISABLED", "已禁用"), theme::GOLD, theme::SURFACE)
            };
            let format = pack
                .pack_format
                .map(|value| format!("{value}"))
                .unwrap_or_else(|| "-".into());
            let path = app
                .settings
                .active_version
                .as_deref()
                .map(|id| {
                    app.game_dir
                        .join("versions")
                        .join(id)
                        .join("resourcepacks")
                        .join(&pack.file)
                })
                .unwrap_or_default();
            let mut lines = vec![
                Line::from(vec![
                    Span::styled(
                        theme::truncate_chars(&pack.file, inner.width.saturating_sub(2) as usize),
                        Style::default()
                            .fg(theme::mix(theme::ORANGE, theme::TEXT, 0.2))
                            .bg(theme::SURFACE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  ", Style::default().bg(theme::SURFACE)),
                    state,
                ]),
                Line::from(""),
                kv(lang.pick("Format", "格式"), format, theme::CYAN),
                kv(
                    lang.pick("Type", "类型"),
                    if pack.is_dir {
                        lang.pick("folder", "文件夹").to_string()
                    } else {
                        "zip".to_string()
                    },
                    theme::PURPLE,
                ),
                kv(
                    lang.pick("Size", "大小"),
                    theme::format_bytes(pack.size),
                    theme::GOLD,
                ),
                kv(
                    lang.pick("Compatible", "兼容"),
                    if pack.incompatible {
                        lang.pick("marked incompatible", "标记为不兼容").to_string()
                    } else {
                        lang.pick("yes", "是").to_string()
                    },
                    if pack.incompatible {
                        theme::RED
                    } else {
                        theme::GREEN
                    },
                ),
                kv(
                    lang.pick("Path", "路径"),
                    theme::truncate_chars(
                        &path.display().to_string(),
                        inner.width.saturating_sub(14) as usize,
                    ),
                    theme::TEXT_FAINT,
                ),
                Line::from(""),
            ];
            if !pack.description.is_empty() {
                lines.push(Line::from(vec![Span::styled(
                    theme::truncate_chars(
                        &pack.description,
                        inner.width.saturating_sub(2) as usize,
                    ),
                    Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
                )]));
                lines.push(Line::from(""));
            }
            lines.push(Line::from(vec![
                Span::styled("◈ ", Style::default().fg(theme::ORANGE).bg(theme::SURFACE)),
                Span::styled(
                    lang.pick(
                        "Enabling edits options.txt; packs load on the next launch.",
                        "启用状态写入 options.txt，下次启动生效。",
                    ),
                    Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                ),
            ]));
            lines.push(Line::from(""));
            push_content_hints(
                &mut lines,
                lang,
                &[
                    ("space", lang.pick(" toggle", " 启停"), theme::GREEN),
                    ("i", lang.pick(" import pack", " 导入资源包"), theme::CYAN),
                    ("x", lang.pick(" delete pack", " 删除资源包"), theme::RED),
                    ("o", lang.pick(" open folder", " 打开目录"), theme::TEXT_DIM),
                ],
            );
            frame.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: true })
                    .style(Style::default().bg(theme::SURFACE)),
                inner,
            );
        }
        ContentKind::Shaders => {
            let Some(shader) = app.selected_shader() else {
                empty_state(
                    frame,
                    area,
                    "▣",
                    lang.pick("Select a shader pack", "选择一个光影包"),
                    &[lang.pick(
                        "Toggle, browse or delete shader packs here.",
                        "在这里启停、浏览或删除光影包。",
                    )],
                    theme::ORANGE,
                    t,
                );
                return;
            };
            let state = if shader.enabled {
                badge(lang.pick("ENABLED", "已启用"), theme::GREEN, theme::SURFACE)
            } else {
                badge(lang.pick("DISABLED", "已禁用"), theme::GOLD, theme::SURFACE)
            };
            let support = shader.support;
            let support_text = support
                .map(|support| support.summary(lang == Lang::Chinese))
                .unwrap_or_else(|| lang.pick("unknown", "未知").to_string());
            let version = app.settings.active_version.clone().unwrap_or_default();
            let loaders =
                crate::content::ContentManager::new(&app.game_dir).shader_loader_status(&version);
            let path = app
                .game_dir
                .join("versions")
                .join(&version)
                .join("shaderpacks")
                .join(&shader.file);
            let title = shader.title.as_deref().unwrap_or(&shader.file);
            let mut lines = vec![
                Line::from(vec![
                    Span::styled(
                        theme::truncate_chars(title, inner.width.saturating_sub(2) as usize),
                        Style::default()
                            .fg(theme::mix(theme::ORANGE, theme::TEXT, 0.2))
                            .bg(theme::SURFACE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  ", Style::default().bg(theme::SURFACE)),
                    state,
                ]),
                Line::from(""),
                kv(
                    lang.pick("File", "文件"),
                    theme::one_line(&shader.file, 200),
                    theme::TEXT_DIM,
                ),
                kv(lang.pick("Loaders", "加载器"), support_text, theme::PURPLE),
                kv(
                    lang.pick("Type", "类型"),
                    if shader.is_dir {
                        lang.pick("folder", "文件夹").to_string()
                    } else {
                        "zip".to_string()
                    },
                    theme::PURPLE,
                ),
                kv(
                    lang.pick("Size", "大小"),
                    theme::format_bytes(shader.size),
                    theme::GOLD,
                ),
                kv(
                    lang.pick("Path", "路径"),
                    theme::truncate_chars(
                        &path.display().to_string(),
                        inner.width.saturating_sub(14) as usize,
                    ),
                    theme::TEXT_FAINT,
                ),
                Line::from(""),
            ];
            if let Some(support) = support {
                if support.iris && !loaders.iris {
                    lines.push(Line::from(vec![Span::styled(
                        lang.pick(
                            "Iris is not installed; install it from Mods browse (iris).",
                            "尚未安装 Iris；可在模组浏览中搜索 iris 安装。",
                        ),
                        Style::default().fg(theme::GOLD).bg(theme::SURFACE),
                    )]));
                }
                if support.optifine && !loaders.optifine && !support.iris {
                    lines.push(Line::from(vec![Span::styled(
                        lang.pick(
                            "OptiFine is required; install it manually from optifine.net.",
                            "需要 OptiFine；请从 optifine.net 手动安装。",
                        ),
                        Style::default().fg(theme::GOLD).bg(theme::SURFACE),
                    )]));
                }
                lines.push(Line::from(""));
            }
            lines.push(Line::from(vec![
                Span::styled("◈ ", Style::default().fg(theme::ORANGE).bg(theme::SURFACE)),
                Span::styled(
                    lang.pick(
                        "Enabling writes shaderPack to options.txt (and optionsof.txt for OptiFine).",
                        "启用状态写入 options.txt 的 shaderPack（OptiFine 还会写入 optionsof.txt）。",
                    ),
                    Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                ),
            ]));
            lines.push(Line::from(""));
            push_content_hints(
                &mut lines,
                lang,
                &[
                    ("space", lang.pick(" toggle", " 启停"), theme::GREEN),
                    ("d", lang.pick(" browse shaders", " 浏览光影"), theme::CYAN),
                    ("i", lang.pick(" import shader", " 导入光影"), theme::CYAN),
                    ("x", lang.pick(" delete shader", " 删除光影"), theme::RED),
                    ("o", lang.pick(" open folder", " 打开目录"), theme::TEXT_DIM),
                ],
            );
            frame.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: true })
                    .style(Style::default().bg(theme::SURFACE)),
                inner,
            );
        }
    }
}

fn push_content_hints(lines: &mut Vec<Line<'static>>, _lang: Lang, hints: &[(&str, &str, Color)]) {
    for (key, text, color) in hints {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" [{key}] "),
                Style::default()
                    .fg(theme::BG)
                    .bg(*color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                (*text).to_string(),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]));
    }
}

// ── accounts ──────────────────────────────────────────────────────

fn accounts(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(1),
            Constraint::Percentage(59),
        ])
        .split(area);
    list_accounts(frame, app, columns[0], t);
    account_detail(frame, app, columns[2], t);
}

fn list_accounts(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    let lang = app.lang();
    let block = panel(
        format!("{} · {}", lang.pick("ACCOUNTS", "账号"), app.accounts.len()),
        theme::GOLD,
        true,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.accounts.is_empty() {
        empty_state(
            frame,
            area,
            "◉",
            lang.pick("No accounts yet", "还没有账号"),
            &[
                lang.pick("Press a to add an offline profile.", "按 a 添加离线档案。"),
                lang.pick("Press m for Microsoft login.", "按 m 使用 Microsoft 登录。"),
            ],
            theme::GOLD,
            t,
        );
        return;
    }
    let items: Vec<ListItem> = app
        .accounts
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let selection = index == app.account_sel;
            let bg = if selection {
                theme::mix(theme::SURFACE, theme::GOLD, 0.14)
            } else {
                theme::SURFACE
            };
            let max = inner.width.saturating_sub(18) as usize;
            let line = Line::from(vec![
                Span::styled(
                    if row.default { " ● " } else { " ○ " },
                    Style::default()
                        .fg(if row.default {
                            theme::GREEN
                        } else {
                            theme::TEXT_FAINT
                        })
                        .bg(bg),
                ),
                Span::styled(
                    theme::pad_width(&theme::truncate_chars(&row.username, max), max),
                    Style::default()
                        .fg(theme::TEXT)
                        .bg(bg)
                        .add_modifier(if selection {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    format!(
                        " {}",
                        theme::pad_width(account_kind_label(lang, &row.kind), 5)
                    ),
                    Style::default()
                        .fg(if selection {
                            theme::GOLD
                        } else {
                            theme::TEXT_FAINT
                        })
                        .bg(bg),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();
    let list = List::new(items)
        .highlight_symbol("▌")
        .highlight_style(Style::default().bg(theme::mix(theme::SURFACE, theme::GOLD, 0.14)))
        .style(Style::default().bg(theme::SURFACE));
    let visible = inner.height as usize;
    let offset = window_offset(app.account_sel, app.accounts.len(), visible);
    let mut state = ListState::default()
        .with_offset(offset)
        .with_selected(Some(app.account_sel));
    frame.render_stateful_widget(list, inner, &mut state);
    let count = visible.min(app.accounts.len().saturating_sub(offset));
    for slot in 0..count {
        register_mouse(
            Rect {
                x: inner.x,
                y: inner.y + slot as u16,
                width: inner.width,
                height: 1,
            },
            MouseAction::AccountsRow(offset + slot),
        );
    }
}

fn account_detail(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let lang = app.lang();
    let block = panel(
        lang.pick("ACCOUNT DETAIL", "账号详情"),
        theme::GOLD,
        true,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(row) = app.selected_account() else {
        empty_state(
            frame,
            area,
            "◉",
            lang.pick("Select an account", "选择一个账号"),
            &[lang.pick(
                "Set a default, add an offline name, or sign in.",
                "设为默认、添加离线名称或登录。",
            )],
            theme::GOLD,
            t,
        );
        return;
    };
    let expiry = app.account_expiry_label(row);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                row.username.clone(),
                Style::default()
                    .fg(theme::mix(theme::GOLD, theme::TEXT, 0.2))
                    .bg(theme::SURFACE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default().bg(theme::SURFACE)),
            badge(
                account_kind_label(lang, &row.kind),
                if row.kind == "msa" {
                    theme::CYAN
                } else {
                    theme::GREEN
                },
                theme::SURFACE,
            ),
            Span::styled(" ", Style::default().bg(theme::SURFACE)),
            if row.default {
                badge(lang.pick("DEFAULT", "默认"), theme::GOLD, theme::SURFACE)
            } else {
                Span::styled("", Style::default().bg(theme::SURFACE))
            },
        ]),
        Line::from(""),
        kv("UUID", short_uuid(&row.uuid), theme::TEXT_FAINT),
        kv(
            lang.pick("Session", "会话"),
            if expiry.is_empty() {
                lang.pick("offline profile", "离线档案").into()
            } else {
                expiry
            },
            if row.expires_at.is_some_and(|value| value < now_f64()) {
                theme::RED
            } else {
                theme::GREEN
            },
        ),
        kv(
            lang.pick("Account slot", "账号标识"),
            theme::one_line(&row.key, 64),
            theme::TEXT_DIM,
        ),
        Line::from(""),
    ];
    if row.kind == "msa" {
        lines.push(Line::from(vec![
            Span::styled("◆ ", Style::default().fg(theme::CYAN).bg(theme::SURFACE)),
            Span::styled(
                lang.pick(
                    "Microsoft access and refresh tokens live in the OS keychain.",
                    "Microsoft 访问/刷新令牌保存在操作系统钥匙串中。",
                ),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]));
        lines.push(Line::from(vec![Span::styled(
            lang.pick(
                "   If no keychain is available, an opt-in encrypted local store is used.",
                "   若钥匙串不可用，将使用经确认后启用的本地加密存储。",
            ),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("◆ ", Style::default().fg(theme::GREEN).bg(theme::SURFACE)),
            Span::styled(
                lang.pick(
                    "Offline accounts cannot join servers that require authentication.",
                    "离线账号无法加入要求正版认证的服务器。",
                ),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            " [d] ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::GOLD)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            lang.pick(" set as default", " 设为默认"),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            " [a] ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            lang.pick(" add offline account", " 添加离线账号"),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            " [m] ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            lang.pick(" Microsoft login", " Microsoft 登录"),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            " [x] ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::RED)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            lang.pick(" log out all accounts", " 登出所有账号"),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        ),
    ]));
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(theme::SURFACE)),
        inner,
    );
}

// ── about ─────────────────────────────────────────────────────────

fn about_field(label: &str, value: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<10}", label),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        ),
        Span::styled(
            value.to_string(),
            Style::default().fg(color).bg(theme::SURFACE),
        ),
    ])
}

fn about(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let lang = app.lang();
    let block = panel(
        format!("{} · Hot-RS-Launcher", lang.pick("ABOUT", "关于")),
        theme::LIME,
        true,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width < 16 {
        return;
    }

    let dim = |text: &str| {
        Span::styled(
            text.to_string(),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        )
    };
    let note = |text: &str| {
        Span::styled(
            text.to_string(),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )
    };

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  ◆ ",
                Style::default()
                    .fg(theme::LIME)
                    .bg(theme::SURFACE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Hot-RS-Launcher",
                Style::default()
                    .fg(theme::TEXT)
                    .bg(theme::SURFACE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("   v{}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            ),
        ]),
        Line::from(vec![
            Span::raw("    "),
            dim(lang.pick(
                "A Rust Minecraft launcher with Microsoft and offline accounts,",
                "Rust 编写的 Minecraft 启动器：微软 / 离线账号、",
            )),
        ]),
        Line::from(vec![
            Span::raw("    "),
            dim(lang.pick(
                "Fabric/Forge/NeoForge, Modrinth mods and an optional terminal frontend.",
                "Fabric/Forge/NeoForge、Modrinth 模组与可选终端界面。",
            )),
        ]),
        Line::from(""),
        about_field("AUTHOR", "Hotsteel2901", theme::TEXT),
        about_field(
            "GITHUB",
            "github.com/Hotsteel2901/mc_rs_cli_launcher",
            theme::CYAN,
        ),
        about_field(
            "CODEBERG",
            "codeberg.org/hotsteel/mc_rs_cli_launcher",
            theme::PURPLE,
        ),
        about_field("LICENSE", "GPL-3.0-only", theme::GOLD),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            note(lang.pick(
                "Both repositories use the same name: mc_rs_cli_launcher.",
                "两个托管平台上的仓库同名：mc_rs_cli_launcher。",
            )),
        ]),
        Line::from(vec![
            Span::raw("  "),
            note(lang.pick(
                "Issues and pull requests are welcome on either host.",
                "欢迎在任一平台提交 issue 或 PR。",
            )),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            note(lang.pick(
                "Built with Rust · ratatui · ureq · clap. Not affiliated with Mojang or Microsoft.",
                "使用 Rust · ratatui · ureq · clap 构建。与 Mojang / Microsoft 无隶属关系。",
            )),
        ]),
    ];
    lines.retain(|line| line.width() <= inner.width as usize || line.spans.len() == 1);
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(theme::SURFACE))
            .wrap(Wrap { trim: false }),
        inner,
    );
}

// ── settings ──────────────────────────────────────────────────────

fn settings(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    if area.width < 84 {
        settings_fields(frame, app, area, t);
        return;
    }
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(54),
            Constraint::Length(1),
            Constraint::Percentage(45),
        ])
        .split(area);
    settings_fields(frame, app, columns[0], t);
    settings_preview(frame, app, columns[2], t);
}

fn settings_fields(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let lang = app.lang();
    let block = panel(
        lang.pick("PREFERENCES", "偏好设置"),
        theme::BLUE,
        true,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 6 {
        return;
    }
    let labels = App::settings_labels(lang);
    let descriptions = [
        lang.pick(
            "Switch the TUI language. Applies immediately.",
            "切换 TUI 界面语言，立即生效。",
        ),
        lang.pick(
            "Java heap passed as -Xmx. 512 MB steps.",
            "以 -Xmx 传给 Java 的堆内存，512 MB 步进。",
        ),
        lang.pick(
            "Game window width. Auto lets Minecraft choose.",
            "游戏窗口宽度。自动交给 Minecraft 决定。",
        ),
        lang.pick(
            "Game window height. Auto lets Minecraft choose.",
            "游戏窗口高度。自动交给 Minecraft 决定。",
        ),
        lang.pick(
            "Parallel download workers. Assets keep at least 16.",
            "并行下载线程数；资源下载至少保持 16 并发。",
        ),
        lang.pick(
            "Auto prefers BMCLAPI for assets; other files try official first.",
            "自动模式下资源优先 BMCLAPI，其他文件优先官方。",
        ),
        lang.pick(
            "Enter to pick a detected runtime or type a path.",
            "回车选择已检测的运行时，或手动输入路径。",
        ),
        lang.pick(
            "Extra JVM flags such as -XX:+UseStringDeduplication.",
            "额外的 JVM 参数，例如 -XX:+UseStringDeduplication。",
        ),
        lang.pick(
            "GC preset; zgc needs Java 21 for generational mode.",
            "垃圾回收器预设；ZGC 的分代模式需要 Java 21。",
        ),
        lang.pick(
            "Turn off decorative motion for lower CPU use.",
            "关闭装饰动效以降低 CPU 占用。",
        ),
        lang.pick(
            "Show the short animated intro on start.",
            "启动时显示短暂的开场动画。",
        ),
    ];
    let field_rows = (inner.height as usize).saturating_sub(5).min(labels.len());
    let selected = app.settings_cursor.min(labels.len() - 1);
    let start = selected.saturating_sub(field_rows.saturating_sub(1));
    for (slot, index) in (start..labels.len()).take(field_rows).enumerate() {
        let y = inner.y + slot as u16;
        let row = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 1,
        };
        let active = index == selected;
        let bg = if active {
            theme::mix(theme::SURFACE, theme::BLUE, 0.18)
        } else {
            theme::SURFACE
        };
        frame.render_widget(Block::default().style(Style::default().bg(bg)), row);
        let value = settings_value(app, index);
        let label_width = 20.min(inner.width as usize / 2);
        let mut spans = vec![
            Span::styled(
                if active { "▌ " } else { "  " },
                Style::default()
                    .fg(if active {
                        theme::mix(theme::BLUE, theme::TEXT, theme::pulse(t, 2.0) * 0.25)
                    } else {
                        theme::SURFACE
                    })
                    .bg(bg),
            ),
            Span::styled(
                theme::pad_width(
                    &theme::truncate_chars(labels[index], label_width),
                    label_width,
                ),
                Style::default()
                    .fg(if active { theme::TEXT } else { theme::TEXT_DIM })
                    .bg(bg)
                    .add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(
                if active { "‹ " } else { "  " },
                Style::default().fg(theme::TEXT_FAINT).bg(bg),
            ),
            Span::styled(
                value,
                Style::default()
                    .fg(if active {
                        theme::mix(theme::BLUE, theme::TEXT, 0.25)
                    } else {
                        theme::TEXT_DIM
                    })
                    .bg(bg),
            ),
            Span::styled(
                if active { " ›" } else { "" },
                Style::default().fg(theme::TEXT_FAINT).bg(bg),
            ),
        ];
        if active {
            let right = lang.pick("enter edit  ·  ←→ change", "回车编辑 · ←→ 调整");
            let used = 2 + label_width + 3 + 20;
            if used + right.len() < inner.width as usize {
                spans.push(Span::styled(
                    " ".repeat(inner.width as usize - used - right.len()),
                    Style::default().bg(bg),
                ));
                spans.push(Span::styled(
                    right,
                    Style::default().fg(theme::TEXT_FAINT).bg(bg),
                ));
            }
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(bg)),
            row,
        );
        register_mouse(row, MouseAction::SettingsRow(index));
        let value_x = (2 + label_width + 2) as u16;
        let value_width = inner.width.saturating_sub(value_x);
        let half = value_width / 2;
        register_mouse(
            Rect {
                x: inner.x.saturating_add(value_x),
                y,
                width: half,
                height: 1,
            },
            MouseAction::SettingsDecrease(index),
        );
        register_mouse(
            Rect {
                x: inner.x.saturating_add(value_x).saturating_add(half),
                y,
                width: value_width.saturating_sub(half),
                height: 1,
            },
            MouseAction::SettingsIncrease(index),
        );
    }
    let help_y = inner.y + field_rows as u16 + 1;
    if help_y + 2 < inner.y + inner.height {
        let help = Rect {
            x: inner.x,
            y: help_y,
            width: inner.width,
            height: 2,
        };
        let saved = app
            .settings_saved_at
            .map(|at| {
                if at.elapsed().as_secs() < 2 {
                    lang.pick("saved now", "刚刚保存").to_string()
                } else if lang == crate::tui::i18n::Lang::Chinese {
                    format!("{} 前保存", theme::format_duration(at.elapsed()))
                } else {
                    format!("saved {} ago", theme::format_duration(at.elapsed()))
                }
            })
            .unwrap_or_else(|| lang.pick("auto-saved", "自动保存").to_string());
        let lines = vec![
            Line::from(vec![
                Span::styled("◆ ", Style::default().fg(theme::BLUE).bg(theme::SURFACE)),
                Span::styled(
                    descriptions[selected],
                    Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
                ),
            ]),
            Line::from(vec![Span::styled(
                format!(
                    "  {} · {saved} · {}",
                    lang.pick("auto-saved to launcher_tui.json", "已自动保存"),
                    lang.pick("R resets", "R 恢复默认")
                ),
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            )]),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: true })
                .style(Style::default().bg(theme::SURFACE)),
            help,
        );
    }
}

fn settings_value(app: &App, index: usize) -> String {
    let lang = app.lang();
    match index {
        0 => lang.label().into(),
        1 => app.settings.ram_label(),
        2 => {
            if app.settings.width == 0 {
                lang.pick("Auto", "自动").into()
            } else {
                format!("{} px", app.settings.width)
            }
        }
        3 => {
            if app.settings.height == 0 {
                lang.pick("Auto", "自动").into()
            } else {
                format!("{} px", app.settings.height)
            }
        }
        4 => format!("{}", app.settings.threads),
        5 => match app.settings.source.as_str() {
            "official" => lang.pick("Official only", "仅官方").into(),
            "bmcl" => "BMCLAPI".into(),
            _ => lang.pick("Auto", "自动").into(),
        },
        6 => {
            if app.settings.java.trim().is_empty() {
                lang.pick("auto-detect", "自动检测").into()
            } else {
                theme::truncate_chars(
                    Path::new(&app.settings.java)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(&app.settings.java),
                    24,
                )
            }
        }
        7 => {
            if app.settings.jvm_args.trim().is_empty() {
                lang.pick("none", "无").into()
            } else {
                theme::truncate_chars(&app.settings.jvm_args, 28)
            }
        }
        8 => app.settings.gc.to_uppercase(),
        9 => if app.settings.reduced_motion {
            lang.pick("On", "开")
        } else {
            lang.pick("Off", "关")
        }
        .into(),
        10 => if app.settings.splash {
            lang.pick("On", "开")
        } else {
            lang.pick("Off", "关")
        }
        .into(),
        _ => String::new(),
    }
}

fn settings_preview(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let lang = app.lang();
    let block = panel(
        lang.pick("PROFILE PREVIEW", "配置预览"),
        theme::BLUE,
        false,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let version = app
        .settings
        .active_version
        .clone()
        .unwrap_or_else(|| lang.pick("<version>", "<版本>").into());
    let loader = app
        .active_loader()
        .unwrap_or_else(|| "vanilla".into())
        .to_lowercase();
    let (width, height) = app.settings.window();
    let window = match (width, height) {
        (Some(width), Some(height)) => format!("{width}×{height}"),
        _ => lang.pick("game default", "游戏默认").into(),
    };
    let java = if app.settings.java.trim().is_empty() {
        app.java_path
            .as_deref()
            .map(|path| {
                Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(path)
                    .to_string()
            })
            .unwrap_or_else(|| lang.pick("auto-detect", "自动检测").into())
    } else {
        app.settings.java.clone()
    };
    let source = match app.settings.source.as_str() {
        "official" => lang.pick("official", "官方"),
        "bmcl" => "BMCLAPI",
        _ => lang.pick("auto", "自动"),
    };
    let mut command = format!(
        "hot-rs-launcher play -v {} --ram {}",
        version,
        app.settings.ram_label().replace(' ', "")
    );
    if loader != "vanilla" {
        command.push_str(&format!(" --{loader}"));
    }
    if app.settings.gc != "auto" {
        command.push_str(&format!(" --gc {}", app.settings.gc));
    }
    if !app.settings.jvm_args.trim().is_empty() {
        command.push_str(&format!(" --jvm-args '{}'", app.settings.jvm_args));
    }
    let lines = vec![
        Line::from(vec![Span::styled(
            lang.pick("Launch profile", "启动配置"),
            Style::default()
                .fg(theme::TEXT)
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        kv(
            lang.pick("Memory", "内存"),
            app.settings.ram_label(),
            theme::GOLD,
        ),
        kv(lang.pick("Window", "窗口"), window, theme::CYAN),
        kv(
            lang.pick("Download", "下载"),
            format!(
                "{} {} · {source}",
                app.settings.threads,
                lang.pick("threads", "线程")
            ),
            theme::BLUE,
        ),
        kv(
            "Java",
            theme::truncate_chars(&java, inner.width.saturating_sub(14) as usize),
            theme::GREEN,
        ),
        kv(
            lang.pick("GC", "GC"),
            app.settings.gc.to_uppercase(),
            theme::PURPLE,
        ),
        kv(
            lang.pick("JVM args", "JVM 参数"),
            if app.settings.jvm_args.trim().is_empty() {
                lang.pick("none", "无").into()
            } else {
                theme::truncate_chars(
                    &app.settings.jvm_args,
                    inner.width.saturating_sub(14) as usize,
                )
            },
            theme::TEXT_DIM,
        ),
        Line::from(""),
        Line::from(vec![Span::styled(
            lang.pick("Equivalent CLI", "等效命令"),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
        Line::from(vec![Span::styled(
            command,
            Style::default().fg(theme::LIME).bg(theme::SURFACE),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            lang.pick(
                "The TUI only wraps the same CLI core;",
                "TUI 只是包装同一份 CLI 核心；",
            ),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
        Line::from(vec![Span::styled(
            lang.pick(
                "every action stays scriptable and terminal-first.",
                "所有操作仍然可脚本化、终端优先。",
            ),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(theme::SURFACE)),
        inner,
    );
}

// ── logs ──────────────────────────────────────────────────────────

fn logs(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let lang = app.lang();
    let block = panel(
        format!(
            "{} · {}  ·  {}",
            lang.pick("ACTIVITY", "活动"),
            app.logs.len(),
            if app.log_follow {
                lang.pick("LIVE", "实时")
            } else {
                lang.pick("PAUSED", "暂停")
            }
        ),
        theme::PINK,
        true,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }
    if app.logs.is_empty() {
        empty_state(
            frame,
            area,
            "≡",
            lang.pick("The log is empty", "日志为空"),
            &[lang.pick(
                "Launches, downloads and UI actions appear here.",
                "启动、下载和界面操作会显示在这里。",
            )],
            theme::PINK,
            t,
        );
        return;
    }
    let visible = inner.height as usize;
    let end = app.logs.len().saturating_sub(app.log_offset);
    let start = end.saturating_sub(visible);
    let lines: Vec<Line> = app.logs[start..end]
        .iter()
        .map(|log| log_line(log, inner.width))
        .collect();
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::SURFACE)),
        inner,
    );
    if !app.log_follow && inner.width > 3 {
        let indicator = Rect {
            x: inner.x + inner.width.saturating_sub(2),
            y: inner.y,
            width: 2,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "↕",
                Style::default().fg(theme::PINK).bg(theme::SURFACE),
            )]))
            .alignment(Alignment::Right),
            indicator,
        );
    }
}

// ── search overlay ────────────────────────────────────────────────

pub fn render_search(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    let lang = app.lang();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(4),
        ])
        .split(area);
    let input_focused = app.search.focus == SearchFocus::Input;
    let accent = match app.search.kind {
        DiscoverKind::Mods => theme::PURPLE,
        DiscoverKind::ResourcePacks => theme::ORANGE,
        DiscoverKind::Shaders => theme::CYAN,
    };
    let mode = if app.search.browse {
        lang.pick("BROWSE", "浏览")
    } else {
        lang.pick("SEARCH", "搜索")
    };
    let input_block = panel(
        format!("{mode} · {}", app.search.kind.title(lang == Lang::Chinese)),
        accent,
        input_focused,
        t,
        theme::SURFACE,
    );
    let input_inner = input_block.inner(rows[0]);
    frame.render_widget(input_block, rows[0]);
    register_mouse(rows[0], MouseAction::SearchInput);
    if input_inner.height > 0 {
        let mut spans = vec![Span::styled(
            " ⌕  ",
            Style::default()
                .fg(theme::mix(
                    theme::CYAN,
                    theme::TEXT,
                    theme::pulse(t, 2.0) * 0.2,
                ))
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        )];
        spans.extend(
            input_line(
                &app.search.input,
                input_inner.width.saturating_sub(4),
                input_focused,
                t,
                theme::SURFACE,
                lang.pick(
                    "browse popular · type to search · enter",
                    "浏览热门 · 输入搜索 · 回车",
                ),
            )
            .spans,
        );
        if app.search.loading {
            spans.push(Span::styled(
                format!(
                    "  {} {}…",
                    theme::spinner(app.frame),
                    lang.pick("searching", "搜索中")
                ),
                Style::default().fg(theme::GOLD).bg(theme::SURFACE),
            ));
        } else if !app.search.results.is_empty() {
            spans.push(Span::styled(
                format!(
                    "  {} {}",
                    app.search.results.len(),
                    lang.pick("results", "个结果")
                ),
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            ));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::SURFACE)),
            input_inner,
        );
    }

    let page_label = if lang == Lang::Chinese {
        format!("第 {} 页", app.search.page + 1)
    } else {
        format!("page {}", app.search.page + 1)
    };
    let sort_label = if lang == Lang::Chinese {
        format!("排序 {}", app.search.sort)
    } else {
        format!("sort {}", app.search.sort)
    };
    let category_label = if lang == Lang::Chinese {
        format!("分类 {}", app.search.category.as_deref().unwrap_or("all"))
    } else {
        format!(
            "category {}",
            app.search.category.as_deref().unwrap_or("all")
        )
    };
    let total_label = if lang == Lang::Chinese {
        format!("共 {} 个", app.search.total_hits)
    } else {
        format!("{} total", app.search.total_hits)
    };
    let hints = lang.pick(
        "[ ] page · b browse · c category · tab focus",
        "[ ] 翻页 · b 浏览 · c 分类 · tab 切换焦点",
    );
    let info =
        format!(" ◆ {page_label} · {sort_label} · {category_label} · {total_label} · {hints}");
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            theme::truncate_chars(&info, area.width as usize),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]))
        .style(Style::default().bg(theme::SURFACE)),
        rows[1],
    );

    let results = panel(
        format!(
            "{} · {}",
            lang.pick("RESULTS", "结果"),
            app.search.kind.title(lang == Lang::Chinese)
        ),
        accent,
        !input_focused,
        t,
        theme::SURFACE,
    );
    let results_inner = results.inner(rows[2]);
    frame.render_widget(results, rows[2]);
    if results_inner.height == 0 {
        return;
    }
    if let Some(error) = app.search.error.as_ref() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![Span::styled(
                    lang.pick("✕  Search failed", "✕  搜索失败"),
                    Style::default()
                        .fg(theme::RED)
                        .bg(theme::SURFACE)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(""),
                Line::from(vec![Span::styled(
                    theme::truncate_chars(error, results_inner.width as usize),
                    Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
                )]),
                Line::from(""),
                Line::from(vec![Span::styled(
                    lang.pick(
                        "Press enter to retry, or esc to close.",
                        "按回车重试，或按 Esc 关闭。",
                    ),
                    Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                )]),
            ])
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(theme::SURFACE)),
            results_inner,
        );
        return;
    }
    if app.search.loading && app.search.results.is_empty() {
        let mut lines = Vec::new();
        for index in 0..(results_inner.height as usize / 2).min(5) {
            let shimmer = theme::pulse(t * 1.4 + index as f64 * 0.5, 2.5);
            let width = results_inner.width as usize;
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {} ", theme::spinner(app.frame + index as u64 * 3)),
                    Style::default().fg(theme::CYAN).bg(theme::SURFACE),
                ),
                Span::styled(
                    "░".repeat(width.saturating_sub(6)),
                    Style::default()
                        .fg(theme::mix(
                            theme::SURFACE,
                            theme::CYAN,
                            0.15 + shimmer * 0.3,
                        ))
                        .bg(theme::SURFACE),
                ),
            ]));
            lines.push(Line::from(""));
        }
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(theme::SURFACE)),
            results_inner,
        );
        return;
    }
    if app.search.results.is_empty() {
        empty_state(
            frame,
            rows[2],
            "⌕",
            lang.pick("Nothing to show", "没有内容"),
            &[
                lang.pick(
                    "Press b to browse popular projects, or type a query.",
                    "按 b 浏览热门项目，或输入关键词搜索。",
                ),
                lang.pick(
                    "Facets use the active version and loader.",
                    "筛选条件使用当前版本与加载器。",
                ),
            ],
            accent,
            t,
        );
        return;
    }
    let items: Vec<ListItem> = app
        .search
        .results
        .iter()
        .enumerate()
        .map(|(index, hit)| {
            let selection = index == app.search.selected;
            let bg = if selection {
                theme::mix(theme::SURFACE, accent, 0.13)
            } else {
                theme::SURFACE
            };
            let installed = if hit.installed {
                match hit.installed_version.as_deref() {
                    Some(version) if !version.is_empty() => {
                        format!("  [{} {version}]", lang.pick("installed", "已安装"))
                    }
                    _ => format!("  [{}]", lang.pick("installed", "已安装")),
                }
            } else {
                String::new()
            };
            let title = format!(
                "{} {}{}",
                theme::truncate_chars(&hit.title, 32),
                installed,
                if selection { " ›" } else { "" }
            );
            let author = if hit.author == "unknown" {
                lang.pick("unknown", "未知")
            } else {
                &hit.author
            };
            let support = if app.search.kind == DiscoverKind::Shaders {
                let support = ShaderSupport::from_labels(hit.loaders.iter().map(String::as_str));
                if support.is_empty() {
                    String::new()
                } else if lang == Lang::Chinese {
                    format!(" · 光影加载器 {}", support.summary(true))
                } else {
                    format!(" · shader loaders {}", support.label())
                }
            } else {
                String::new()
            };
            let meta = if lang == Lang::Chinese {
                format!(
                    "作者 {} · {} 次下载{}{}",
                    author,
                    format_num(hit.downloads),
                    support,
                    if hit.categories.is_empty() {
                        " · 未分类".to_string()
                    } else {
                        format!(" · {}", hit.categories)
                    }
                )
            } else {
                format!(
                    "by {} · {} downloads{}{}",
                    author,
                    format_num(hit.downloads),
                    support,
                    if hit.categories.is_empty() {
                        " · uncategorized".into()
                    } else {
                        format!(" · {}", hit.categories)
                    }
                )
            };
            let description = theme::truncate_chars(
                if hit.description.is_empty() {
                    lang.pick("No description", "无描述")
                } else {
                    &hit.description
                },
                results_inner.width.saturating_sub(4) as usize,
            );
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(
                        " ◆ ",
                        Style::default()
                            .fg(if selection { accent } else { theme::TEXT_FAINT })
                            .bg(bg),
                    ),
                    Span::styled(
                        title,
                        Style::default()
                            .fg(theme::TEXT)
                            .bg(bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("   ", Style::default().bg(bg)),
                    Span::styled(
                        theme::truncate_chars(
                            &description,
                            results_inner.width.saturating_sub(5) as usize,
                        ),
                        Style::default().fg(theme::TEXT_DIM).bg(bg),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("   ", Style::default().bg(bg)),
                    Span::styled(meta, Style::default().fg(theme::TEXT_FAINT).bg(bg)),
                ]),
                Line::from(""),
            ])
        })
        .collect();
    let list = List::new(items)
        .highlight_symbol("▌")
        .highlight_style(Style::default().bg(theme::mix(theme::SURFACE, accent, 0.13)))
        .style(Style::default().bg(theme::SURFACE));
    let item_height = 4usize;
    let visible_items = (results_inner.height as usize / item_height).max(1);
    let offset = window_offset(app.search.selected, app.search.results.len(), visible_items);
    let mut state = ListState::default()
        .with_offset(offset)
        .with_selected(Some(app.search.selected));
    frame.render_stateful_widget(list, results_inner, &mut state);
    let count = visible_items.min(app.search.results.len().saturating_sub(offset));
    for slot in 0..count {
        let y = results_inner.y + (slot * item_height) as u16;
        let height = item_height
            .min((results_inner.y + results_inner.height).saturating_sub(y) as usize)
            as u16;
        register_mouse(
            Rect {
                x: results_inner.x,
                y,
                width: results_inner.width,
                height,
            },
            MouseAction::SearchRow(offset + slot),
        );
    }
}

// ── small helpers ─────────────────────────────────────────────────

fn kv(label: &str, value: String, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            theme::pad_width(label, 14),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        ),
        Span::styled(value, Style::default().fg(color).bg(theme::SURFACE)),
    ])
}

fn shorten_loader(value: &str) -> String {
    match value {
        "Fabric" => "Fab".into(),
        "NeoForge" => "Neo".into(),
        "Forge" => "For".into(),
        other => theme::truncate_chars(other, 4),
    }
}

fn short_bytes(size: u64) -> String {
    if size >= 1_048_576 {
        format!("{:.1}M", size as f64 / 1_048_576.0)
    } else if size >= 1024 {
        format!("{:.0}K", size as f64 / 1024.0)
    } else {
        format!("{size}B")
    }
}

fn account_kind_label(lang: Lang, kind: &str) -> &'static str {
    match kind {
        "msa" => lang.pick("MSA", "微软"),
        _ => lang.pick("OFFLINE", "离线"),
    }
}

fn short_uuid(uuid: &str) -> String {
    let chars: Vec<char> = uuid.chars().collect();
    if chars.len() > 20 {
        let head: String = chars.iter().take(8).collect();
        let tail: String = chars.iter().rev().take(4).rev().collect();
        format!("{head}…{tail}")
    } else if uuid.is_empty() {
        "unknown".into()
    } else {
        uuid.to_string()
    }
}

fn age_label(lang: Lang, age: Option<Duration>) -> String {
    let Some(age) = age else {
        return lang.pick("unknown", "未知").into();
    };
    let secs = age.as_secs();
    if secs < 120 {
        lang.pick("just now", "刚刚").into()
    } else if secs < 3600 {
        if lang == Lang::Chinese {
            format!("{} 分钟前", secs / 60)
        } else {
            format!("{}m ago", secs / 60)
        }
    } else if secs < 86_400 {
        if lang == Lang::Chinese {
            format!("{} 小时前", secs / 3600)
        } else {
            format!("{}h ago", secs / 3600)
        }
    } else if lang == Lang::Chinese {
        format!("{} 天前", secs / 86_400)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

fn format_num(value: u64) -> String {
    let text = value.to_string();
    let mut out = String::with_capacity(text.len() + text.len() / 3);
    for (index, ch) in text.chars().enumerate() {
        if index > 0 && (text.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn window_offset(selected: usize, len: usize, visible: usize) -> usize {
    if visible == 0 || len <= visible {
        0
    } else {
        selected
            .saturating_sub(visible - 1)
            .min(len.saturating_sub(visible))
    }
}

fn now_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
