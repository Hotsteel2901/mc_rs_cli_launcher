//! Individual screens: dashboard, versions, mods, accounts, settings, logs
//! and the Modrinth search overlay.

use std::path::Path;
use std::time::Duration;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::app::{App, Screen, SearchFocus};
use crate::tui::theme;
use crate::tui::ui::log_line;
use crate::tui::ui::widgets::{badge, empty_state, input_line, mini_panel, panel, soft_badge};

pub fn render_screen(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    match app.screen {
        Screen::Dashboard => dashboard(frame, app, area, t),
        Screen::Versions => versions(frame, app, area, t),
        Screen::Mods => mods(frame, app, area, t),
        Screen::Accounts => accounts(frame, app, area, t),
        Screen::Settings => settings(frame, app, area, t),
        Screen::Logs => logs(frame, app, area, t),
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
    let block = panel("INSTANCE", theme::GREEN, true, t, theme::SURFACE);
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
            "No instance yet.",
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
        let loader = app.active_loader().unwrap_or_else(|| "Vanilla".into());
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
                    if mods == 1 { "mod" } else { "mods" }
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
                    account.kind.to_ascii_uppercase(),
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
                "No account · press a to add one",
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
                "account  ",
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
            Span::styled("2 ", Style::default().fg(theme::GOLD).bg(theme::SURFACE)),
            Span::styled(
                "version  ",
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
            Span::styled("3 ", Style::default().fg(theme::GOLD).bg(theme::SURFACE)),
            Span::styled(
                "play",
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
        ("↓", "DOWNLOAD", "press n", theme::GOLD)
    } else if app.current_account().is_none() {
        ("◉", "ADD ACCOUNT", "press a", theme::CYAN)
    } else {
        ("▶", "PLAY", "press enter", theme::GREEN)
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
        let loader = app.active_loader().unwrap_or_else(|| "Vanilla".into());
        let details = vec![
            Line::from(vec![
                Span::styled(
                    "RAM  ",
                    Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                ),
                Span::styled(
                    app.settings.ram_label(),
                    Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "LOAD ",
                    Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                ),
                Span::styled(
                    loader,
                    Style::default().fg(theme::PURPLE).bg(theme::SURFACE),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "PATH ",
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
    let latest = app
        .versions
        .first()
        .map(|row| row.id.clone())
        .unwrap_or_else(|| "none".into());
    let active_mods = app.current_version();
    let mods_value = active_mods
        .map(|row| format!("{}", row.mods_enabled))
        .unwrap_or_else(|| "0".into());
    let mods_hint = active_mods
        .map(|row| format!("{} disabled", row.mods_disabled))
        .unwrap_or_else(|| "no version".into());
    let java_value = if app.java_probing {
        format!("detecting{}", ".".repeat((app.frame as usize / 8) % 4))
    } else if let Some(major) = app.java_major {
        format!("Java {major}")
    } else {
        "not found".into()
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
        "VERSIONS",
        &format!("{}", app.versions.len()),
        &latest,
        theme::CYAN,
        t,
    );
    stat_card(
        frame,
        chunks[1],
        "MODS ACTIVE",
        &mods_value,
        &mods_hint,
        theme::PURPLE,
        t,
    );
    stat_card(
        frame,
        chunks[2],
        "MEMORY",
        &app.settings.ram_label(),
        &format!("{} threads", app.settings.threads),
        theme::GOLD,
        t,
    );
    stat_card(
        frame,
        chunks[3],
        "JAVA",
        &java_value,
        &java_hint,
        theme::BLUE,
        t,
    );
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
        "RECENT ACTIVITY  ·  l for the full log",
        theme::PINK,
        false,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    if app.logs.is_empty() {
        empty_state(
            frame,
            area,
            "≡",
            "Quiet in here",
            &["Actions and background jobs appear here."],
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
        format!("INSTALLED · {}", app.versions.len()),
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
            "No versions installed",
            &[
                "Press n to browse Minecraft versions.",
                "Downloads are checked and cached.",
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
                format!("{}m", row.mods_enabled + row.mods_disabled)
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
    frame.render_stateful_widget(list, inner, &mut app.version_state);
}

fn version_detail(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let block = panel("VERSION DETAIL", theme::CYAN, true, t, theme::SURFACE);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(row) = app.selected_version() else {
        empty_state(
            frame,
            area,
            "▤",
            "Select a version",
            &["Installed versions and their loader state appear here."],
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
                row.kind.clone(),
                match row.kind.as_str() {
                    "release" => theme::GREEN,
                    "snapshot" => theme::GOLD,
                    _ => theme::PURPLE,
                },
                theme::SURFACE,
            ),
            Span::styled(" ", Style::default().bg(theme::SURFACE)),
            if is_active {
                badge("ACTIVE", theme::GREEN, theme::SURFACE)
            } else {
                Span::styled("", Style::default().bg(theme::SURFACE))
            },
        ]),
        Line::from(""),
    ];
    lines.push(kv(
        "Loader",
        if row.loaders.is_empty() {
            "Vanilla".into()
        } else {
            row.loaders.join(" + ")
        },
        theme::PURPLE,
    ));
    lines.push(kv(
        "Mods",
        format!(
            "{} enabled · {} disabled",
            row.mods_enabled, row.mods_disabled
        ),
        theme::PURPLE,
    ));
    lines.push(kv(
        "Client jar",
        if row.jar_size == 0 {
            "missing".into()
        } else {
            theme::format_bytes(row.jar_size)
        },
        if row.jar_size == 0 {
            theme::RED
        } else {
            theme::TEXT_DIM
        },
    ));
    lines.push(kv("Modified", age_label(row.age), theme::TEXT_DIM));
    lines.push(kv(
        "Directory",
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
                "This is the active instance."
            } else {
                "Press v to make this the active instance."
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
            " play this version",
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
            " install a loader",
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
            " download another version",
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
    let version = app
        .settings
        .active_version
        .clone()
        .unwrap_or_else(|| "no version".into());
    let enabled = app.mods.iter().filter(|row| row.enabled).count();
    let disabled = app.mods.len() - enabled;
    let block = panel(
        format!("MODS · {version} · {enabled} on / {disabled} off"),
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
            "No active version",
            &["Download a version, then install mods."],
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
            "No mods installed",
            &[
                "Press s to search Modrinth.",
                "Mods install into this version's mods folder.",
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
            let max = inner.width.saturating_sub(14) as usize;
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
    frame.render_stateful_widget(list, inner, &mut app.mod_state);
}

fn mod_detail(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let block = panel("MOD DETAIL", theme::PURPLE, true, t, theme::SURFACE);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(row) = app.selected_mod() else {
        empty_state(
            frame,
            area,
            "❖",
            "Select a mod",
            &["Space toggles a mod.", "x removes it from this version."],
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
    let lines = vec![
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
                badge("ENABLED", theme::GREEN, theme::SURFACE)
            } else {
                badge("DISABLED", theme::GOLD, theme::SURFACE)
            },
            Span::styled("  ", Style::default().bg(theme::SURFACE)),
            Span::styled(
                theme::format_bytes(row.size),
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]),
        Line::from(""),
        kv("File", theme::one_line(&row.file, 200), theme::TEXT_DIM),
        kv(
            "Folder",
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
                "Disabled mods are renamed with a .disabled suffix.",
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
                " toggle",
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
                " uninstall",
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
                " search Modrinth",
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(theme::SURFACE)),
        inner,
    );
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
    let block = panel(
        format!("ACCOUNTS · {}", app.accounts.len()),
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
            "No accounts yet",
            &[
                "Press a to add an offline profile.",
                "Press m for Microsoft login.",
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
                    format!(
                        "{:<max$}",
                        theme::truncate_chars(&row.username, max),
                        max = max
                    ),
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
                    format!(" {:<5}", row.kind.to_ascii_uppercase()),
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
    frame.render_stateful_widget(list, inner, &mut app.account_state);
}

fn account_detail(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let block = panel("ACCOUNT DETAIL", theme::GOLD, true, t, theme::SURFACE);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(row) = app.selected_account() else {
        empty_state(
            frame,
            area,
            "◉",
            "Select an account",
            &["Set a default, add an offline name, or sign in."],
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
                row.kind.to_ascii_uppercase(),
                if row.kind == "msa" {
                    theme::CYAN
                } else {
                    theme::GREEN
                },
                theme::SURFACE,
            ),
            Span::styled(" ", Style::default().bg(theme::SURFACE)),
            if row.default {
                badge("DEFAULT", theme::GOLD, theme::SURFACE)
            } else {
                Span::styled("", Style::default().bg(theme::SURFACE))
            },
        ]),
        Line::from(""),
        kv("UUID", short_uuid(&row.uuid), theme::TEXT_FAINT),
        kv(
            "Session",
            if expiry.is_empty() {
                "offline profile".into()
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
            "Account slot",
            theme::one_line(&row.key, 64),
            theme::TEXT_DIM,
        ),
        Line::from(""),
    ];
    if row.kind == "msa" {
        lines.push(Line::from(vec![
            Span::styled("◆ ", Style::default().fg(theme::CYAN).bg(theme::SURFACE)),
            Span::styled(
                "Microsoft access and refresh tokens live in the OS keychain.",
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]));
        lines.push(Line::from(vec![Span::styled(
            "   If no keychain is available, an opt-in encrypted local store is used.",
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("◆ ", Style::default().fg(theme::GREEN).bg(theme::SURFACE)),
            Span::styled(
                "Offline accounts cannot join servers that require authentication.",
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
            " set as default",
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
            " add offline account",
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
            " Microsoft login",
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
            " log out all accounts",
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

// ── settings ──────────────────────────────────────────────────────

fn settings(frame: &mut Frame, app: &mut App, area: Rect, t: f64) {
    if area.width < 88 {
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
    let block = panel("PREFERENCES", theme::BLUE, true, t, theme::SURFACE);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 6 {
        return;
    }
    let labels = App::settings_labels();
    let descriptions = [
        "Java heap passed as -Xmx. 512 MB steps.",
        "Game window width. Auto lets Minecraft choose.",
        "Game window height. Auto lets Minecraft choose.",
        "Parallel download workers used by the CLI core.",
        "Auto tries official first, then BMCLAPI on failure.",
        "Empty means auto-detect. Enter to edit.",
        "Turn off decorative motion for lower CPU use.",
        "Show the short animated intro on start.",
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
                format!(
                    "{:<width$}",
                    theme::truncate_chars(labels[index], label_width),
                    width = label_width
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
            let right = "enter edit  ·  ←→ change";
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
                    "saved now".to_string()
                } else {
                    format!("saved {} ago", theme::format_duration(at.elapsed()))
                }
            })
            .unwrap_or_else(|| "auto-saved".to_string());
        let lines = vec![
            Line::from(vec![
                Span::styled("◆ ", Style::default().fg(theme::BLUE).bg(theme::SURFACE)),
                Span::styled(
                    descriptions[selected],
                    Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
                ),
            ]),
            Line::from(vec![Span::styled(
                format!("  auto-saved to launcher_tui.json · {saved} · R resets"),
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
    match index {
        0 => app.settings.ram_label(),
        1 => {
            if app.settings.width == 0 {
                "Auto".into()
            } else {
                format!("{} px", app.settings.width)
            }
        }
        2 => {
            if app.settings.height == 0 {
                "Auto".into()
            } else {
                format!("{} px", app.settings.height)
            }
        }
        3 => format!("{}", app.settings.threads),
        4 => match app.settings.source.as_str() {
            "official" => "Official only".into(),
            "bmcl" => "BMCLAPI".into(),
            _ => "Auto".into(),
        },
        5 => {
            if app.settings.java.trim().is_empty() {
                "auto-detect".into()
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
        6 => if app.settings.reduced_motion {
            "On"
        } else {
            "Off"
        }
        .into(),
        7 => if app.settings.splash { "On" } else { "Off" }.into(),
        _ => String::new(),
    }
}

fn settings_preview(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let block = panel("PROFILE PREVIEW", theme::BLUE, false, t, theme::SURFACE);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let version = app
        .settings
        .active_version
        .clone()
        .unwrap_or_else(|| "<version>".into());
    let loader = app
        .active_loader()
        .unwrap_or_else(|| "vanilla".into())
        .to_lowercase();
    let (width, height) = app.settings.window();
    let window = match (width, height) {
        (Some(width), Some(height)) => format!("{width}×{height}"),
        _ => "game default".into(),
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
            .unwrap_or_else(|| "auto-detect".into())
    } else {
        app.settings.java.clone()
    };
    let mut command = format!(
        "mc-launcher play -v {} --ram {}",
        version,
        app.settings.ram_label().replace(' ', "")
    );
    if loader != "vanilla" {
        command.push_str(&format!(" --{loader}"));
    }
    let lines = vec![
        Line::from(vec![Span::styled(
            "Launch profile",
            Style::default()
                .fg(theme::TEXT)
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        kv("Memory", app.settings.ram_label(), theme::GOLD),
        kv("Window", window, theme::CYAN),
        kv(
            "Download",
            format!("{} threads · {}", app.settings.threads, app.settings.source),
            theme::BLUE,
        ),
        kv(
            "Java",
            theme::truncate_chars(&java, inner.width.saturating_sub(14) as usize),
            theme::GREEN,
        ),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Equivalent CLI",
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
        Line::from(vec![Span::styled(
            command,
            Style::default().fg(theme::LIME).bg(theme::SURFACE),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "The TUI only wraps the same CLI core;",
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
        Line::from(vec![Span::styled(
            "every action stays scriptable and terminal-first.",
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
    let block = panel(
        format!(
            "ACTIVITY · {}  ·  {}",
            app.logs.len(),
            if app.log_follow { "LIVE" } else { "PAUSED" }
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
            "The log is empty",
            &["Launches, downloads and UI actions appear here."],
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
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(4),
        ])
        .split(area);
    let input_focused = app.search.focus == SearchFocus::Input;
    let input_block = panel(
        "SEARCH MODRINTH",
        theme::CYAN,
        input_focused,
        t,
        theme::SURFACE,
    );
    let input_inner = input_block.inner(rows[0]);
    frame.render_widget(input_block, rows[0]);
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
                "type a mod name, then press enter",
            )
            .spans,
        );
        if app.search.loading {
            spans.push(Span::styled(
                format!("  {} searching…", theme::spinner(app.frame)),
                Style::default().fg(theme::GOLD).bg(theme::SURFACE),
            ));
        } else if !app.search.results.is_empty() {
            spans.push(Span::styled(
                format!("  {} results", app.search.results.len()),
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            ));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::SURFACE)),
            input_inner,
        );
    }

    let results = panel("RESULTS", theme::CYAN, !input_focused, t, theme::SURFACE);
    let results_inner = results.inner(rows[2]);
    frame.render_widget(results, rows[2]);
    if results_inner.height == 0 {
        return;
    }
    if let Some(error) = app.search.error.as_ref() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![Span::styled(
                    "✕  Search failed",
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
                    "Press enter to retry, or esc to close.",
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
            "Search Modrinth",
            &[
                "Type a query and press enter.",
                "Facets use the active version and loader.",
            ],
            theme::CYAN,
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
                theme::mix(theme::SURFACE, theme::CYAN, 0.13)
            } else {
                theme::SURFACE
            };
            let title = format!(
                "{}  {}",
                theme::truncate_chars(&hit.title, 34),
                if selection { "›" } else { "" }
            );
            let meta = format!(
                "by {} · {} downloads · {}",
                hit.author,
                format_num(hit.downloads),
                if hit.categories.is_empty() {
                    "uncategorized".into()
                } else {
                    hit.categories.clone()
                }
            );
            let description = theme::truncate_chars(
                if hit.description.is_empty() {
                    "No description"
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
                            .fg(if selection {
                                theme::CYAN
                            } else {
                                theme::TEXT_FAINT
                            })
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
        .highlight_style(Style::default().bg(theme::mix(theme::SURFACE, theme::CYAN, 0.13)))
        .style(Style::default().bg(theme::SURFACE));
    frame.render_stateful_widget(list, results_inner, &mut app.search.list_state);
}

// ── small helpers ─────────────────────────────────────────────────

fn kv(label: &str, value: String, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<14}", label),
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

fn age_label(age: Option<Duration>) -> String {
    let Some(age) = age else {
        return "unknown".into();
    };
    let secs = age.as_secs();
    if secs < 120 {
        "just now".into()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
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

fn now_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
