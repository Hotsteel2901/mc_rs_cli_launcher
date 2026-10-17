//! Palette, gradient text and small animation helpers for the TUI.

use std::time::Duration;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

pub const BG: Color = Color::Rgb(9, 12, 18);
pub const SURFACE: Color = Color::Rgb(18, 23, 33);
pub const SURFACE_ALT: Color = Color::Rgb(25, 31, 44);
pub const BORDER: Color = Color::Rgb(46, 58, 78);
pub const BORDER_HI: Color = Color::Rgb(82, 102, 136);
pub const TEXT: Color = Color::Rgb(229, 235, 245);
pub const TEXT_DIM: Color = Color::Rgb(135, 149, 173);
pub const TEXT_FAINT: Color = Color::Rgb(82, 96, 120);
pub const GREEN: Color = Color::Rgb(122, 214, 100);
pub const GREEN_DARK: Color = Color::Rgb(55, 112, 54);
pub const LIME: Color = Color::Rgb(190, 246, 128);
pub const CYAN: Color = Color::Rgb(92, 205, 235);
pub const BLUE: Color = Color::Rgb(122, 160, 245);
pub const GOLD: Color = Color::Rgb(245, 199, 91);
pub const RED: Color = Color::Rgb(240, 110, 120);
pub const PURPLE: Color = Color::Rgb(176, 145, 245);
pub const PINK: Color = Color::Rgb(240, 150, 195);

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn spinner(frame: u64) -> &'static str {
    SPINNER[(frame as usize / 2) % SPINNER.len()]
}

/// 0..1 triangle ease used for glowing borders and badges.
pub fn pulse(t: f64, speed: f64) -> f64 {
    ((t * speed).sin() * 0.5 + 0.5).clamp(0.0, 1.0)
}

pub fn ease_out_back(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

pub fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (a as f64 + (b as f64 - a as f64) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

pub fn mix(a: Color, b: Color, t: f64) -> Color {
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => {
            Color::Rgb(lerp_u8(ar, br, t), lerp_u8(ag, bg, t), lerp_u8(ab, bb, t))
        }
        _ => b,
    }
}

pub fn dim(color: Color, amount: f64) -> Color {
    mix(color, BG, amount.clamp(0.0, 1.0))
}

pub fn brighten(color: Color, amount: f64) -> Color {
    mix(color, Color::Rgb(255, 255, 255), amount.clamp(0.0, 1.0))
}

/// A per-character gradient with a soft travelling shimmer.
pub fn gradient_text_bg(
    value: &str,
    from: Color,
    to: Color,
    bg: Color,
    phase: f64,
    bold: bool,
) -> Vec<Span<'static>> {
    let count = value.chars().count().max(1);
    value
        .chars()
        .enumerate()
        .map(|(i, ch)| {
            let t = i as f64 / count.saturating_sub(1).max(1) as f64;
            let mut color = mix(from, to, t);
            let shimmer = ((i as f64 * 0.55) - phase * 1.6).sin() * 0.5 + 0.5;
            color = brighten(color, shimmer.powf(8.0) * 0.25);
            let mut style = Style::default().fg(color).bg(bg);
            if bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            Span::styled(ch.to_string(), style)
        })
        .collect()
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}

/// Collapse control characters (newlines from logs, tabs from installers) into
/// spaces and truncate for one-line TUI surfaces.
pub fn one_line(value: &str, max: usize) -> String {
    let cleaned: String = value
        .chars()
        .map(|ch| {
            if ch == '\n' || ch == '\r' || ch == '\t' || ch.is_control() {
                ' '
            } else {
                ch
            }
        })
        .collect();
    truncate_chars(cleaned.trim(), max)
}

pub fn truncate_chars(value: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = value.chars().count();
    if count <= max {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Terminal cell width for the characters used in the UI. This is a compact
/// approximation of Unicode East Asian Width: CJK, Hangul, fullwidth forms and
/// emoji take two cells; everything else takes one. Combining marks are rare in
/// labels and are not specially handled.
pub fn char_width(ch: char) -> usize {
    let code = ch as u32;
    if code == 0 {
        return 0;
    }
    if (0x0300..=0x036F).contains(&code) {
        return 0;
    }
    if (0x1100..=0x115F).contains(&code)
        || (0x2E80..=0xA4CF).contains(&code)
        || (0xAC00..=0xD7A3).contains(&code)
        || (0xF900..=0xFAFF).contains(&code)
        || (0xFE10..=0xFE19).contains(&code)
        || (0xFE30..=0xFE6F).contains(&code)
        || (0xFF00..=0xFF60).contains(&code)
        || (0xFFE0..=0xFFE6).contains(&code)
        || (0x1F300..=0x1FAFF).contains(&code)
        || (0x20000..=0x3FFFD).contains(&code)
    {
        2
    } else {
        1
    }
}

pub fn display_width(value: &str) -> usize {
    value.chars().map(char_width).sum()
}

/// Pad a label to a terminal-cell width. Used for aligned key/value rows where
/// `format!` width counts chars and would misalign CJK text.
pub fn pad_width(value: &str, width: usize) -> String {
    let used = display_width(value);
    if used >= width {
        value.to_string()
    } else {
        format!("{value}{}", " ".repeat(width - used))
    }
}

/// Remove CSI/OSC escape sequences from process logs before rendering them.
pub fn strip_ansi(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            if ch == '\r' {
                continue;
            }
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            Some(']') => {
                let mut last = None;
                for next in chars.by_ref() {
                    if next == '\u{7}' || (last == Some('\u{1b}') && next == '\\') {
                        break;
                    }
                    last = Some(next);
                }
            }
            Some(_) | None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_helpers_are_stable() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(truncate_chars("abcdef", 4), "abc…");
        assert_eq!(truncate_chars("abc", 4), "abc");
        assert_eq!(one_line("a\nb\tc", 10), "a b c");
        assert_eq!(format_duration(Duration::from_secs(65)), "1m 05s");
    }

    #[test]
    fn ansi_and_carriage_returns_are_removed() {
        let dirty = "\u{1b}[32mhello\u{1b}[0m\r\nworld";
        assert_eq!(strip_ansi(dirty), "hello\nworld");
    }

    #[test]
    fn gradient_returns_one_span_per_char() {
        let spans = gradient_text_bg("hello", GREEN, CYAN, SURFACE, 1.0, true);
        assert_eq!(spans.len(), 5);
        assert!(spans.iter().all(|span| span.style.fg.is_some()));
    }

    #[test]
    fn easing_is_bounded() {
        for t in [-1.0, 0.0, 0.25, 0.5, 1.0, 2.0] {
            assert!((0.0..=1.0).contains(&pulse(t, 3.0)));
        }
        assert!((ease_out_back(0.0) - 0.0).abs() < 1e-9);
        assert!((ease_out_back(1.0) - 1.0).abs() < 1e-9);
    }
}
