//! Modal dialogs: help, confirmations, text input, version/loader pickers and
//! the Microsoft login method chooser.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};
use ratatui::Frame;

use crate::tui::app::{App, Modal, MouseAction, PickerState};
use crate::tui::i18n::Lang;
use crate::tui::theme;
use crate::tui::ui::register_mouse;
use crate::tui::ui::widgets::{centered_rect, input_line, panel};

pub fn render(frame: &mut Frame, app: &App, area: Rect, t: f64) {
    let Some(modal) = app.modal.as_ref() else {
        return;
    };
    // A solid backdrop keeps the dialog readable over any screen. Terminals
    // cannot blend colors, so a clean stage beats a noisy half-visible card.
    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().style(Style::default().bg(theme::BG)), area);
    register_mouse(area, MouseAction::CloseModal);
    let lang = app.lang();
    match modal {
        Modal::Help { scroll } => help(frame, app, area, *scroll, t),
        Modal::Confirm {
            title,
            lines,
            action: _,
            danger,
        } => confirm(frame, lang, title, lines, *danger, area, t),
        Modal::Input { .. } => input_modal(frame, lang, modal, area, t),
        Modal::Picker(picker) => picker_modal(frame, lang, picker, area, t),
        Modal::Login { selected } => login(frame, lang, *selected, area, t),
    }
}

fn help(frame: &mut Frame, app: &App, area: Rect, scroll: usize, t: f64) {
    let rect = centered_rect(area, 90, 92, area.height.saturating_sub(4).min(32));
    frame.render_widget(Clear, rect);
    let lang = app.lang();
    let block = panel(
        lang.pick("HELP · Hot-RS-Launcher", "帮助 · Hot-RS-Launcher"),
        theme::GREEN,
        true,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    register_mouse(rect, MouseAction::Noop);
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
                    " {}/{}  ·  ↑↓ {}  ·  esc {}",
                    (scroll + 1).min(max_scroll + 1),
                    max_scroll + 1,
                    app.lang().pick("scroll", "滚动"),
                    app.lang().pick("close", "关闭")
                ),
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            )]))
            .alignment(Alignment::Right),
            footer,
        );
    }
}

fn help_lines(app: &App) -> Vec<Line<'static>> {
    let lang = app.lang();
    let tr = |en: &'static str, zh: &'static str| lang.pick(en, zh);
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
            tr(
                "Hot-RS-Launcher  ·  terminal frontend",
                "Hot-RS-Launcher  ·  终端前端",
            ),
            theme::GREEN,
            theme::CYAN,
            theme::SURFACE,
            0.0,
            true,
        )),
        Line::from(vec![Span::styled(
            tr(
                "The CLI remains the primary interface. The TUI reuses the same core.",
                "CLI 仍是主要接口，TUI 复用同一份核心。",
            ),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
        Line::from(""),
        section(tr("GLOBAL", "全局")),
        Line::from(vec![key("tab / shift+tab"), desc(tr("switch screen", "切换界面"))]),
        Line::from(vec![key("1 … 6"), desc(tr("jump to a screen directly", "直接跳转到界面"))]),
        Line::from(vec![key("?"), desc(tr("this help", "显示此帮助"))]),
        Line::from(vec![key("j"), desc(tr("show or hide the job panel", "显示/隐藏任务面板"))]),
        Line::from(vec![key("c"), desc(tr("cancel a running task", "取消正在运行的任务"))]),
        Line::from(vec![
            key("esc"),
            desc(tr("back to dashboard / dismiss a finished job", "返回概览 / 关闭已完成任务")),
        ]),
        Line::from(vec![
            key("q / ctrl+c"),
            desc(tr("quit safely (terminal is restored)", "安全退出（恢复终端）")),
        ]),
        Line::from(""),
        section(tr("DASHBOARD", "概览")),
        Line::from(vec![key("enter / p"), desc(tr("play the active instance", "启动活跃实例"))]),
        Line::from(vec![
            key("n"),
            desc(tr("browse and download Minecraft versions", "浏览并下载 Minecraft 版本")),
        ]),
        Line::from(vec![
            key("i"),
            desc(tr("select or install Fabric / Forge / NeoForge", "选择或安装 Fabric / Forge / NeoForge")),
        ]),
        Line::from(vec![key("s"), desc(tr("search Modrinth", "搜索 Modrinth"))]),
        Line::from(vec![key("a"), desc(tr("accounts", "账号"))]),
        Line::from(""),
        section(tr("VERSIONS", "版本")),
        Line::from(vec![key("↑ / ↓"), desc(tr("choose a version", "选择版本"))]),
        Line::from(vec![key("enter"), desc(tr("play the selected version", "启动所选版本"))]),
        Line::from(vec![key("v"), desc(tr("make it the active instance", "设为活跃实例"))]),
        Line::from(vec![
            key("i"),
            desc(tr("select or install a loader for the active version", "为活跃版本选择或安装加载器")),
        ]),
        Line::from(vec![key("n"), desc(tr("download another version", "下载其他版本"))]),
        Line::from(""),
        section(tr("MODS", "模组")),
        Line::from(vec![
            key("space"),
            desc(tr("enable / disable the selected jar", "启用/禁用所选 JAR")),
        ]),
        Line::from(vec![key("x"), desc(tr("uninstall after confirmation", "确认后卸载"))]),
        Line::from(vec![
            key("s / /"),
            desc(tr("search Modrinth with version + loader facets", "按版本与加载器筛选搜索 Modrinth")),
        ]),
        Line::from(vec![key("v"), desc(tr("switch the active Minecraft version", "切换活跃 Minecraft 版本"))]),
        Line::from(""),
        section(tr("ACCOUNTS", "账号")),
        Line::from(vec![key("a"), desc(tr("add an offline profile", "添加离线档案"))]),
        Line::from(vec![
            key("m"),
            desc(tr("Microsoft browser or device-code login", "Microsoft 浏览器/设备码登录")),
        ]),
        Line::from(vec![key("d"), desc(tr("set the selected account as default", "将所选账号设为默认"))]),
        Line::from(vec![key("x"), desc(tr("log out and delete local tokens", "登出并删除本地令牌"))]),
        Line::from(""),
        section(tr("SETTINGS", "设置")),
        Line::from(vec![key("↑ / ↓"), desc(tr("choose a preference", "选择偏好项"))]),
        Line::from(vec![key("← / →"), desc(tr("adjust it", "调整数值"))]),
        Line::from(vec![key("enter"), desc(tr("edit Java path / toggle options", "编辑 Java 路径/切换开关"))]),
        Line::from(vec![key("R"), desc(tr("restore defaults", "恢复默认值"))]),
        Line::from(""),
        section(tr("FILES", "文件")),
        Line::from(vec![Span::styled(
            format!(
                " {}  {}",
                tr("game dir", "游戏目录"),
                app.game_dir.display()
            ),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
        Line::from(vec![Span::styled(
            format!(
                " {}    {}",
                tr("config", "配置"),
                crate::tui::config::Settings::path(&app.game_dir).display()
            ),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            tr(
                "Downloads verify SHA-1 and publish atomically; the UI never bypasses those checks.",
                "下载会校验 SHA-1 并原子发布；界面不会绕过这些检查。",
            ),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]),
    ]
}

fn confirm(
    frame: &mut Frame,
    lang: Lang,
    title: &str,
    lines: &[String],
    danger: bool,
    area: Rect,
    t: f64,
) {
    let width = area.width.saturating_sub(8).min(68);
    let height = (lines.len() as u16 + 7).min(area.height);
    let rect = centered_rect(area, 90, width, height);
    frame.render_widget(Clear, rect);
    let color = if danger { theme::RED } else { theme::GOLD };
    let block = panel(title.to_string(), color, true, t, theme::SURFACE);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    register_mouse(rect, MouseAction::Noop);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);
    if rows[1].height > 0 {
        let half = rows[1].width / 2;
        register_mouse(
            Rect {
                x: rows[1].x,
                y: rows[1].y,
                width: half,
                height: rows[1].height,
            },
            MouseAction::ConfirmYes,
        );
        register_mouse(
            Rect {
                x: rows[1].x + half,
                y: rows[1].y,
                width: rows[1].width - half,
                height: rows[1].height,
            },
            MouseAction::ConfirmNo,
        );
    }
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
                if lang == Lang::Chinese {
                    " 确认   "
                } else {
                    " confirm   "
                },
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
                lang.pick(" cancel", " 取消"),
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

fn input_modal(frame: &mut Frame, lang: Lang, modal: &Modal, area: Rect, t: f64) {
    let Modal::Input {
        title,
        hint,
        input,
        error,
        ..
    } = modal
    else {
        return;
    };
    let error = error.as_deref();
    let rect = centered_rect(area, 88, 68, 9);
    frame.render_widget(Clear, rect);
    let block = panel(title.to_string(), theme::GREEN, true, t, theme::SURFACE);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    register_mouse(rect, MouseAction::Noop);
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
            lang.pick("enter save · esc cancel", "回车保存 · esc 取消"),
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

fn picker_modal(frame: &mut Frame, lang: Lang, picker: &PickerState, area: Rect, t: f64) {
    let desired = (picker.items.len() as u16 + 8).clamp(12, 30);
    let rect = centered_rect(area, 94, 96, desired.min(area.height.saturating_sub(4)));
    frame.render_widget(Clear, rect);
    let block = panel(picker.title.clone(), theme::CYAN, true, t, theme::SURFACE);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    register_mouse(rect, MouseAction::Noop);
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
                lang.pick("type to filter…", "输入以筛选…"),
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
                format!(" {}", lang.pick("no matches", "无匹配"))
            } else {
                format!(" {} {}", picker.filtered.len(), lang.pick("item(s)", "项"))
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
                    lang.pick(
                        "✕  Could not load remote versions",
                        "✕  无法加载远程版本列表",
                    ),
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
                    lang.pick("r retry · esc close", "r 重试 · esc 关闭"),
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
                    lang.pick("No versions match the filter.", "没有符合筛选条件的版本。"),
                    Style::default()
                        .fg(theme::TEXT_DIM)
                        .bg(theme::SURFACE)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(vec![Span::styled(
                    lang.pick("Backspace to widen the filter.", "退格键放宽筛选条件。"),
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
                let tag_text = match item.tag.as_str() {
                    "release" => lang.pick("release", "正式版"),
                    "snapshot" => lang.pick("snapshot", "快照"),
                    "installed" => lang.pick("installed", "已安装"),
                    "active" => lang.pick("active", "当前"),
                    "new" => lang.pick("install new", "安装"),
                    other => other,
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
                        format!(" {tag_text} "),
                        Style::default().fg(tag_color).bg(theme::mix(
                            theme::SURFACE,
                            tag_color,
                            0.16,
                        )),
                    ));
                }
                if item.installed && item.tag != "installed" && item.tag != "active" {
                    spans.push(Span::styled(
                        if lang == Lang::Chinese {
                            " ✓ 已安装 "
                        } else {
                            " ✓ installed "
                        },
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
        let visible = rows[2].height as usize;
        let offset = window_offset(picker.selected, picker.filtered.len(), visible);
        let mut state = ListState::default()
            .with_offset(offset)
            .with_selected(Some(picker.selected));
        frame.render_stateful_widget(list, rows[2], &mut state);
        let count = visible.min(picker.filtered.len().saturating_sub(offset));
        for slot in 0..count {
            register_mouse(
                Rect {
                    x: rows[2].x,
                    y: rows[2].y + slot as u16,
                    width: rows[2].width,
                    height: 1,
                },
                MouseAction::PickerRow(offset + slot),
            );
        }
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

fn window_offset(selected: usize, len: usize, visible: usize) -> usize {
    if visible == 0 || len <= visible {
        0
    } else {
        selected
            .saturating_sub(visible - 1)
            .min(len.saturating_sub(visible))
    }
}

fn login(frame: &mut Frame, lang: Lang, selected: usize, area: Rect, t: f64) {
    let rect = centered_rect(area, 92, 70, 14);
    frame.render_widget(Clear, rect);
    let block = panel(
        lang.pick("MICROSOFT LOGIN", "Microsoft 登录"),
        theme::CYAN,
        true,
        t,
        theme::SURFACE,
    );
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    register_mouse(rect, MouseAction::Noop);
    if inner.height < 8 {
        return;
    }
    let options = [
        (
            lang.pick("Browser", "浏览器"),
            lang.pick(
                "Opens your default browser, then waits for the callback.",
                "打开默认浏览器，然后等待回调。",
            ),
            lang.pick("fastest · recommended", "最快 · 推荐"),
        ),
        (
            lang.pick("Device code", "设备码"),
            lang.pick(
                "Shows a short code to enter at microsoft.com/link.",
                "显示一个短代码，在 microsoft.com/link 输入。",
            ),
            lang.pick(
                "works on remote/headless-ish terminals",
                "适用于远程/无头环境",
            ),
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
                lang.pick(
                    "The TUI leaves the alternate screen while signing in.",
                    "登录期间 TUI 会暂时离开备用屏幕。",
                ),
                Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
            )]),
            Line::from(vec![Span::styled(
                lang.pick(
                    "Credentials stay in the OS keychain or approved encrypted store.",
                    "凭据保存在系统钥匙串或已确认的加密存储中。",
                ),
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
        register_mouse(rect, MouseAction::LoginRow(index));
    }
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            lang.pick(
                "↑↓ choose · enter continue · esc cancel",
                "↑↓ 选择 · 回车继续 · esc 取消",
            ),
            Style::default().fg(theme::TEXT_FAINT).bg(theme::SURFACE),
        )]))
        .alignment(Alignment::Center)
        .style(Style::default().bg(theme::SURFACE)),
        rows[5],
    );
}
