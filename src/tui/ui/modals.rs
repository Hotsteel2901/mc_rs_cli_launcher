//! Modal dialogs: help, confirmations, text input, version/loader pickers and
//! the Microsoft login method chooser.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::app::{App, Modal, PickerState};
use crate::tui::theme;
use crate::tui::ui::widgets::{centered_rect, input_line, panel};

pub fn render(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let Some(modal) = app.modal.as_ref() else {
        return;
    };
    // A solid backdrop keeps the dialog readable over any screen. Terminals
    // cannot blend colors, so a clean stage beats a noisy half-visible card.
    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().style(Style::default().bg(theme::BG)), area);
    match modal {
        Modal::Help { scroll } => help(frame, app, area, *scroll, t),
        Modal::Confirm {
            title,
            lines,
            action: _,
            danger,
        } => confirm(frame, title, lines, *danger, area, t),
        Modal::Input {
            title,
            hint,
            input,
            action: _,
            error,
        } => input_modal(frame, title, hint, input, error.as_deref(), area, t),
        Modal::Picker(picker) => picker_modal(frame, picker, area, t),
        Modal::Login { selected } => login(frame, *selected, area, t),
    }
}

fn help(frame: &mut Frame, app: &App, area: Rect, scroll: usize, t: f64) {
    let rect = centered_rect(area, 90, 92, area.height.saturating_sub(4).min(32));
    frame.render_widget(Clear, rect);
    let block = panel("HELP · mc-launcher", theme::GREEN, true, t, theme::SURFACE);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let lines = help_lines(app);
    let content_height = inner.height.saturating_sub(1) as usize;
    let max_scroll = lines.len().saturating_sub(content_height);
    let scroll = scroll.min(max_scroll);
    let visible: Vec<Line> = lines
        .into_iter()
        .skip(scroll)
        .take(content_height)
        .collect();
    frame.render_widget(
        Paragraph::new(visible).style(Style::default().bg(theme::SURFACE)),
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: content_height as u16,
        },
    );
    if inner.height > 0 {
        let footer = Rect {
            x: inner.x,
            y: inner.y + inner.height.saturating_sub(1),
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!(
                    " {}/{}  ·  ↑↓ scroll  ·  esc close",
                    (scroll + 1).min(max_scroll + 1),
                    max_scroll + 1
                ),
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            )]))
            .alignment(Alignment::Right),
            footer,
        );
    }
}

fn help_lines(app: &App) -> Vec<Line<'static>> {
    let key = |text: &str| {
        Span::styled(
            format!(" {text} "),
            Style::default()
                .fg(theme::BG)
                .bg(theme::TEXT_DIM)
                .add_modifier(Modifier::BOLD),
        )
    };
    let desc = |text: &str| {
        Span::styled(
            format!(" {text}"),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        )
    };
    let section = |text: &str| {
        Line::from(vec![
            Span::styled("◆ ", Style::default().fg(theme::GREEN).bg(theme::SURFACE)),
            Span::styled(
                text.to_string(),
                Style::default()
                    .fg(theme::GREEN)
                    .bg(theme::SURFACE)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    };
    vec![
        Line::from(theme::gradient_text_bg(
            "mc-launcher  ·  terminal frontend",
            theme::GREEN,
            theme::CYAN,
            theme::SURFACE,
            0.0,
            true,
        )),
        Line::from(vec![Span::styled(
            "The CLI remains the primary interface. The TUI reuses the same core.",
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
        Line::from(""),
        section("GLOBAL"),
        Line::from(vec![key("tab / shift+tab"), desc("switch screen")]),
        Line::from(vec![key("1 … 6"), desc("jump to a screen directly")]),
        Line::from(vec![key("?"), desc("this help")]),
        Line::from(vec![key("j"), desc("show or hide the job panel")]),
        Line::from(vec![key("c"), desc("cancel a running task")]),
        Line::from(vec![
            key("esc"),
            desc("back to dashboard / dismiss a finished job"),
        ]),
        Line::from(vec![
            key("q / ctrl+c"),
            desc("quit safely (terminal is restored)"),
        ]),
        Line::from(""),
        section("DASHBOARD"),
        Line::from(vec![key("enter / p"), desc("play the active instance")]),
        Line::from(vec![
            key("n"),
            desc("browse and download Minecraft versions"),
        ]),
        Line::from(vec![
            key("i"),
            desc("select or install Fabric / Forge / NeoForge"),
        ]),
        Line::from(vec![key("s"), desc("search Modrinth")]),
        Line::from(vec![key("a"), desc("accounts")]),
        Line::from(""),
        section("VERSIONS"),
        Line::from(vec![key("↑ / ↓"), desc("choose a version")]),
        Line::from(vec![key("enter"), desc("play the selected version")]),
        Line::from(vec![key("v"), desc("make it the active instance")]),
        Line::from(vec![
            key("i"),
            desc("select or install a loader for the active version"),
        ]),
        Line::from(vec![key("n"), desc("download another version")]),
        Line::from(""),
        section("MODS"),
        Line::from(vec![
            key("space"),
            desc("enable / disable the selected jar"),
        ]),
        Line::from(vec![key("x"), desc("uninstall after confirmation")]),
        Line::from(vec![
            key("s / /"),
            desc("search Modrinth with version + loader facets"),
        ]),
        Line::from(vec![key("v"), desc("switch the active Minecraft version")]),
        Line::from(""),
        section("ACCOUNTS"),
        Line::from(vec![key("a"), desc("add an offline profile")]),
        Line::from(vec![
            key("m"),
            desc("Microsoft browser or device-code login"),
        ]),
        Line::from(vec![key("d"), desc("set the selected account as default")]),
        Line::from(vec![key("x"), desc("log out and delete local tokens")]),
        Line::from(""),
        section("SETTINGS"),
        Line::from(vec![key("↑ / ↓"), desc("choose a preference")]),
        Line::from(vec![key("← / →"), desc("adjust it")]),
        Line::from(vec![key("enter"), desc("edit Java path / toggle options")]),
        Line::from(vec![key("R"), desc("restore defaults")]),
        Line::from(""),
        section("FILES"),
        Line::from(vec![Span::styled(
            format!(" game dir  {}", app.game_dir.display()),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
        Line::from(vec![Span::styled(
            format!(
                " config    {}",
                crate::tui::config::Settings::path(&app.game_dir).display()
            ),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Downloads verify SHA-1 and publish atomically; the UI never bypasses those checks.",
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
    ]
}

fn confirm(frame: &mut Frame, title: &str, lines: &[String], danger: bool, area: Rect, t: f64) {
    let width = area.width.saturating_sub(8).min(68);
    let height = (lines.len() as u16 + 7).min(area.height);
    let rect = centered_rect(area, 90, width, height);
    frame.render_widget(Clear, rect);
    let color = if danger { theme::RED } else { theme::GOLD };
    let block = panel(title.to_string(), color, true, t, theme::SURFACE);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);
    let mut content = Vec::new();
    for line in lines {
        content.push(Line::from(vec![Span::styled(
            line.clone(),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        )]));
    }
    frame.render_widget(
        Paragraph::new(content)
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(theme::SURFACE)),
        rows[0],
    );
    if rows[1].height > 0 {
        let buttons = Line::from(vec![
            Span::styled(
                " [y] ",
                Style::default()
                    .fg(theme::BG)
                    .bg(color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " confirm   ",
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
            Span::styled(
                " [n] ",
                Style::default()
                    .fg(theme::BG)
                    .bg(theme::TEXT_DIM)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " cancel",
                Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(buttons)
                .alignment(Alignment::Center)
                .style(Style::default().bg(theme::SURFACE)),
            rows[1],
        );
    }
}

fn input_modal(
    frame: &mut Frame,
    title: &str,
    hint: &str,
    input: &crate::tui::input::TextInput,
    error: Option<&str>,
    area: Rect,
    t: f64,
) {
    let rect = centered_rect(area, 88, 68, 9);
    frame.render_widget(Clear, rect);
    let block = panel(title.to_string(), theme::GREEN, true, t, theme::SURFACE);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height < 3 {
        return;
    }
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::mix(theme::GREEN, theme::TEXT_FAINT, 0.4)))
        .style(Style::default().bg(theme::SURFACE));
    let input_inner = input_block.inner(Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: 3.min(inner.height.saturating_sub(1)),
    });
    frame.render_widget(
        input_block,
        Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: 3.min(inner.height.saturating_sub(1)),
        },
    );
    if input_inner.height > 0 {
        let line = input_line(
            input,
            input_inner.width.saturating_sub(2),
            true,
            t,
            theme::SURFACE,
            "",
        );
        frame.render_widget(
            Paragraph::new(line).style(Style::default().bg(theme::SURFACE)),
            Rect {
                x: input_inner.x + 1,
                y: input_inner.y,
                width: input_inner.width.saturating_sub(2),
                height: 1,
            },
        );
    }
    let mut footer = vec![Line::from(vec![Span::styled(
        theme::truncate_chars(hint, inner.width as usize),
        Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
    )])];
    if let Some(error) = error {
        footer.push(Line::from(vec![Span::styled(
            format!(
                "✕ {}",
                theme::truncate_chars(error, inner.width.saturating_sub(3) as usize)
            ),
            Style::default().fg(theme::RED).bg(theme::SURFACE),
        )]));
    } else {
        footer.push(Line::from(vec![Span::styled(
            "enter save · esc cancel",
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]));
    }
    let y = inner.y + 5;
    if y < inner.y + inner.height {
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().bg(theme::SURFACE)),
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: (inner.y + inner.height - y).min(2),
            },
        );
    }
}

fn picker_modal(frame: &mut Frame, picker: &PickerState, area: Rect, t: f64) {
    let desired = (picker.items.len() as u16 + 8).clamp(12, 30);
    let rect = centered_rect(area, 94, 96, desired.min(area.height.saturating_sub(4)));
    frame.render_widget(Clear, rect);
    let block = panel(picker.title.clone(), theme::CYAN, true, t, theme::SURFACE);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height < 6 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

    let filter = Line::from(
        std::iter::once(Span::styled(
            " ⌕ ",
            Style::default()
                .fg(theme::CYAN)
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        ))
        .chain(
            input_line(
                &picker.input,
                rows[0].width.saturating_sub(3),
                true,
                t,
                theme::SURFACE,
                "type to filter…",
            )
            .spans,
        )
        .collect::<Vec<_>>(),
    );
    frame.render_widget(
        Paragraph::new(filter).style(Style::default().bg(theme::SURFACE)),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            if picker.loading {
                format!(
                    " {} loading remote list…",
                    theme::spinner((t * 10.0) as u64)
                )
            } else if picker.filtered.is_empty() {
                " no matches".to_string()
            } else {
                format!(" {} item(s)", picker.filtered.len())
            },
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]))
        .style(Style::default().bg(theme::SURFACE)),
        rows[1],
    );

    if let Some(error) = picker.error.as_ref() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![Span::styled(
                    "✕  Could not load remote versions",
                    Style::default()
                        .fg(theme::RED)
                        .bg(theme::SURFACE)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(""),
                Line::from(vec![Span::styled(
                    theme::truncate_chars(error, rows[2].width as usize),
                    Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
                )]),
                Line::from(""),
                Line::from(vec![Span::styled(
                    "r retry · esc close",
                    Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                )]),
            ])
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(theme::SURFACE)),
            rows[2],
        );
    } else if picker.loading && picker.items.is_empty() {
        let mut lines = Vec::new();
        for index in 0..(rows[2].height as usize).min(8) {
            let shimmer = theme::pulse(t * 1.5 + index as f64 * 0.45, 2.4);
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {} ", theme::spinner((t * 10.0) as u64 + index as u64)),
                    Style::default().fg(theme::CYAN).bg(theme::SURFACE),
                ),
                Span::styled(
                    "░".repeat(rows[2].width.saturating_sub(4) as usize),
                    Style::default()
                        .fg(theme::mix(
                            theme::SURFACE,
                            theme::CYAN,
                            0.12 + shimmer * 0.3,
                        ))
                        .bg(theme::SURFACE),
                ),
            ]));
        }
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(theme::SURFACE)),
            rows[2],
        );
    } else if picker.filtered.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(vec![Span::styled(
                    "No versions match the filter.",
                    Style::default()
                        .fg(theme::TEXT_DIM)
                        .bg(theme::SURFACE)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(vec![Span::styled(
                    "Backspace to widen the filter.",
                    Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                )]),
            ])
            .alignment(Alignment::Center)
            .style(Style::default().bg(theme::SURFACE)),
            rows[2],
        );
    } else {
        let items: Vec<ListItem> = picker
            .filtered
            .iter()
            .filter_map(|index| picker.items.get(*index))
            .map(|item| {
                let tag_color = match item.tag.as_str() {
                    "release" => theme::GREEN,
                    "snapshot" => theme::GOLD,
                    "installed" | "active" => theme::CYAN,
                    _ => theme::TEXT_FAINT,
                };
                let inner_width = rows[2].width as usize;
                let label_width = if inner_width < 60 { 12 } else { 18 };
                let meta_width = inner_width.saturating_sub(27 + label_width).clamp(6, 36);
                let mut spans = vec![
                    Span::styled(" ◆ ", Style::default().fg(tag_color).bg(theme::SURFACE)),
                    Span::styled(
                        format!(
                            "{:<label_width$}",
                            theme::truncate_chars(&item.label, label_width)
                        ),
                        Style::default()
                            .fg(theme::TEXT)
                            .bg(theme::SURFACE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            " {:<meta_width$}",
                            theme::truncate_chars(&item.meta, meta_width)
                        ),
                        Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
                    ),
                ];
                if !item.tag.is_empty() {
                    spans.push(Span::styled(
                        format!(" {} ", item.tag),
                        Style::default().fg(tag_color).bg(theme::mix(
                            theme::SURFACE,
                            tag_color,
                            0.16,
                        )),
                    ));
                }
                if item.installed && item.tag != "installed" && item.tag != "active" {
                    spans.push(Span::styled(
                        " ✓ installed ",
                        Style::default().fg(theme::GREEN).bg(theme::mix(
                            theme::SURFACE,
                            theme::GREEN,
                            0.14,
                        )),
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        let list = List::new(items)
            .highlight_symbol("▌")
            .highlight_style(
                Style::default()
                    .bg(theme::mix(theme::SURFACE, theme::CYAN, 0.16))
                    .fg(theme::TEXT),
            )
            .style(Style::default().bg(theme::SURFACE));
        let mut state = picker.list_state.clone();
        state.select(Some(picker.selected));
        frame.render_stateful_widget(list, rows[2], &mut state);
    }

    let hint_text = picker.hint.clone();
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(" {}", hint_text),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]))
        .style(Style::default().bg(theme::SURFACE)),
        rows[3],
    );
}

fn login(frame: &mut Frame, selected: usize, area: Rect, t: f64) {
    let rect = centered_rect(area, 92, 70, 14);
    frame.render_widget(Clear, rect);
    let block = panel("MICROSOFT LOGIN", theme::CYAN, true, t, theme::SURFACE);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height < 8 {
        return;
    }
    let options = [
        (
            "Browser",
            "Opens your default browser, then waits for the callback.",
            "fastest · recommended",
        ),
        (
            "Device code",
            "Shows a short code to enter at microsoft.com/link.",
            "works on remote/headless-ish terminals",
        ),
    ];
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![Span::styled(
                "The TUI leaves the alternate screen while signing in.",
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            )]),
            Line::from(vec![Span::styled(
                "Credentials stay in the OS keychain or approved encrypted store.",
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            )]),
        ])
        .style(Style::default().bg(theme::SURFACE)),
        rows[0],
    );
    for (index, option) in options.iter().enumerate() {
        let rect = rows[1 + index * 2];
        let active = index == selected;
        let color = if active { theme::CYAN } else { theme::BORDER };
        let bg = if active {
            theme::mix(theme::SURFACE, theme::CYAN, 0.14)
        } else {
            theme::SURFACE
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color).bg(bg));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let title = Line::from(vec![
            Span::styled(
                if active { "▌ " } else { "  " },
                Style::default().fg(color).bg(bg),
            ),
            Span::styled(
                option.0,
                Style::default()
                    .fg(if active { theme::TEXT } else { theme::TEXT_DIM })
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("   {}", option.2),
                Style::default().fg(theme::TEXT_FAINT).bg(bg),
            ),
        ]);
        let description = Line::from(vec![Span::styled(
            theme::truncate_chars(option.1, inner.width.saturating_sub(3) as usize),
            Style::default().fg(theme::TEXT_FAINT).bg(bg),
        )]);
        frame.render_widget(
            Paragraph::new(vec![title, description]).style(Style::default().bg(bg)),
            inner,
        );
    }
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "↑↓ choose · enter continue · esc cancel",
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]))
        .alignment(Alignment::Center)
        .style(Style::default().bg(theme::SURFACE)),
        rows[5],
    );
}
