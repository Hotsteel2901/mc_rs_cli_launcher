//! Shared drawing primitives: panels, badges, progress bars and text inputs.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::tui::app::App;
use crate::tui::input::TextInput;
use crate::tui::theme;

pub fn panel(
    title: impl Into<String>,
    accent: Color,
    focused: bool,
    t: f64,
    bg: Color,
) -> Block<'static> {
    let title = title.into();
    let border = if focused {
        theme::mix(accent, theme::BORDER_HI, 0.25 + theme::pulse(t, 2.2) * 0.35)
    } else {
        theme::BORDER
    };
    let title_color = if focused {
        theme::mix(accent, theme::TEXT, 0.2)
    } else {
        theme::TEXT_DIM
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border).bg(bg))
        .title(Line::from(vec![Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(title_color)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        )]))
        .style(Style::default().fg(theme::TEXT).bg(bg))
}

pub fn mini_panel(title: impl Into<String>, accent: Color, t: f64) -> Block<'static> {
    let title = title.into();
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::mix(
            accent,
            theme::BORDER,
            0.55 + theme::pulse(t, 1.7) * 0.2,
        )))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme::mix(accent, theme::TEXT, 0.25))
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().fg(theme::TEXT).bg(theme::SURFACE))
}

pub fn badge(text: impl Into<String>, color: Color, bg: Color) -> Span<'static> {
    Span::styled(
        format!(" {} ", text.into()),
        Style::default()
            .fg(theme::mix(color, theme::TEXT, 0.12))
            .bg(theme::mix(bg, color, 0.2))
            .add_modifier(Modifier::BOLD),
    )
}

pub fn soft_badge(text: impl Into<String>, color: Color, bg: Color) -> Span<'static> {
    Span::styled(
        format!(" {} ", text.into()),
        Style::default()
            .fg(theme::mix(color, theme::TEXT, 0.15))
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    )
}

pub fn hint(key: &str, what: &str, bg: Color) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            format!(" {key} "),
            Style::default()
                .fg(theme::BG)
                .bg(theme::mix(theme::TEXT_DIM, theme::TEXT, 0.15))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {what}  "),
            Style::default().fg(theme::TEXT_FAINT).bg(bg),
        ),
    ]
}

/// Build a smooth block progress line. `ratio` is 0..1; `None` animates a
/// travelling highlight for work whose total size is unknown.
pub fn progress_spans(
    ratio: Option<f64>,
    width: u16,
    from: Color,
    to: Color,
    t: f64,
    bg: Color,
) -> Vec<Span<'static>> {
    let width = width as usize;
    if width == 0 {
        return Vec::new();
    }
    let mut spans = Vec::with_capacity(width);
    match ratio {
        Some(ratio) => {
            let ratio = ratio.clamp(0.0, 1.0);
            let filled = ratio * width as f64;
            let full = filled.floor() as usize;
            let fraction = filled - full as f64;
            const PARTIALS: [&str; 8] = ["▏", "▎", "▍", "▌", "▋", "▊", "▉", ""];
            for index in 0..width {
                let shade = index as f64 / width.max(1) as f64;
                let color = theme::mix(from, to, shade);
                if index < full {
                    spans.push(Span::styled("█", Style::default().fg(color).bg(bg)));
                } else if index == full && fraction > 0.02 {
                    let partial = PARTIALS[(fraction * 8.0).floor() as usize % 8];
                    spans.push(Span::styled(
                        partial,
                        Style::default().fg(theme::brighten(color, 0.25)).bg(bg),
                    ));
                } else {
                    spans.push(Span::styled(
                        "░",
                        Style::default().fg(theme::dim(theme::BORDER, 0.35)).bg(bg),
                    ));
                }
            }
        }
        None => {
            let head = ((t * 0.38) % 1.3 - 0.15) * width as f64;
            for index in 0..width {
                let distance = (index as f64 - head).abs();
                let color = if distance < 3.0 {
                    let intensity = 1.0 - distance / 3.0;
                    theme::mix(theme::BORDER, to, 0.35 + intensity * 0.65)
                } else if (index as f64 - head) < -3.0 && (index as f64 - head) > -10.0 {
                    theme::mix(theme::BORDER, from, 0.2)
                } else {
                    theme::dim(theme::BORDER, 0.45)
                };
                spans.push(Span::styled(
                    if distance < 3.0 { "█" } else { "░" },
                    Style::default().fg(color).bg(bg),
                ));
            }
        }
    }
    spans
}

/// A single-line input with a blinking block caret and horizontal scrolling.
pub fn input_line(
    input: &TextInput,
    width: u16,
    focused: bool,
    t: f64,
    bg: Color,
    placeholder: &str,
) -> Line<'static> {
    let available = width.saturating_sub(1).max(1) as usize;
    let chars: Vec<char> = input.value().chars().collect();
    if chars.is_empty() {
        let mut spans = Vec::new();
        if focused {
            let visible = (t * 2.0).fract() < 0.58;
            spans.push(Span::styled(
                if visible { "▌" } else { " " },
                Style::default().fg(theme::GREEN).bg(bg),
            ));
        }
        spans.push(Span::styled(
            placeholder.to_string(),
            Style::default().fg(theme::TEXT_FAINT).bg(bg),
        ));
        return Line::from(spans);
    }

    let cursor = input.cursor().min(chars.len());
    let start = cursor.saturating_sub(available / 2);
    let end = (start + available).min(chars.len());
    let mut spans = Vec::new();
    if start > 0 {
        spans.push(Span::styled(
            "‹",
            Style::default().fg(theme::TEXT_FAINT).bg(bg),
        ));
    }
    for (offset, ch) in chars[start..end].iter().enumerate() {
        let position = start + offset;
        if focused && position == cursor {
            let visible = (t * 2.0).fract() < 0.58;
            if visible {
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default().fg(theme::BG).bg(theme::GREEN),
                ));
            } else {
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default().fg(theme::TEXT).bg(bg),
                ));
            }
        } else {
            spans.push(Span::styled(
                ch.to_string(),
                Style::default().fg(theme::TEXT).bg(bg),
            ));
        }
    }
    if focused && cursor == chars.len() {
        let visible = (t * 2.0).fract() < 0.58;
        spans.push(Span::styled(
            if visible { "▌" } else { " " },
            Style::default().fg(theme::GREEN).bg(bg),
        ));
    }
    if end < chars.len() {
        spans.push(Span::styled(
            "›",
            Style::default().fg(theme::TEXT_FAINT).bg(bg),
        ));
    }
    Line::from(spans)
}

pub fn centered_rect(area: Rect, width_percent: u16, max_width: u16, height: u16) -> Rect {
    let width = (area.width as u32 * width_percent as u32 / 100)
        .max(20)
        .min(max_width as u32)
        .min(area.width as u32) as u16;
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

pub fn empty_state(
    frame: &mut Frame,
    area: Rect,
    icon: &str,
    title: &str,
    lines: &[&str],
    accent: Color,
    t: f64,
) {
    let body = area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });
    if body.height < 3 {
        return;
    }
    let mut content = vec![
        Line::from(vec![Span::styled(
            icon.to_string(),
            Style::default()
                .fg(theme::mix(accent, theme::TEXT, theme::pulse(t, 1.5) * 0.2))
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            title.to_string(),
            Style::default()
                .fg(theme::TEXT)
                .bg(theme::SURFACE)
                .add_modifier(Modifier::BOLD),
        )]),
    ];
    for line in lines {
        content.push(Line::from(vec![Span::styled(
            line.to_string(),
            Style::default().fg(theme::TEXT_DIM).bg(theme::SURFACE),
        )]));
    }
    let paragraph = Paragraph::new(content)
        .alignment(Alignment::Center)
        .style(Style::default().bg(theme::SURFACE));
    frame.render_widget(paragraph, body);
}

pub fn account_name(app: &App) -> String {
    app.current_account()
        .map(|account| account.username.clone())
        .unwrap_or_else(|| "no account".into())
}
