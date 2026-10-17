//! A small single-line text input with UTF-8-safe cursor movement.

#[derive(Clone, Debug)]
pub struct TextInput {
    value: String,
    /// Cursor position in chars, not bytes.
    cursor: usize,
    max_chars: usize,
}

impl Default for TextInput {
    fn default() -> Self {
        Self::new("")
    }
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        let mut input = Self {
            value: String::new(),
            cursor: 0,
            max_chars: 256,
        };
        input.set(value);
        input
    }

    pub fn with_max_chars(mut self, max: usize) -> Self {
        self.max_chars = max.max(1);
        self.clamp_len();
        self
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set(&mut self, value: impl Into<String>) {
        self.value = sanitize(&value.into());
        self.cursor = self.value.chars().count();
        self.clamp_len();
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    pub fn insert_char(&mut self, ch: char) {
        if ch.is_control() {
            return;
        }
        if self.value.chars().count() >= self.max_chars {
            return;
        }
        let byte = self.byte_index();
        self.value.insert(byte, ch);
        self.cursor += 1;
    }

    pub fn insert_str(&mut self, text: &str) {
        for ch in sanitize(text).chars() {
            self.insert_char(ch);
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = self.byte_index();
        let start = self.byte_index_for(self.cursor - 1);
        self.value.replace_range(start..end, "");
        self.cursor -= 1;
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.value.chars().count() {
            return;
        }
        let start = self.byte_index();
        let end = self.byte_index_for(self.cursor + 1);
        self.value.replace_range(start..end, "");
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.value.chars().count());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.value.chars().count();
    }

    pub fn kill_to_start(&mut self) {
        let end = self.byte_index();
        self.value.replace_range(0..end, "");
        self.cursor = 0;
    }

    fn byte_index(&self) -> usize {
        self.byte_index_for(self.cursor)
    }

    fn byte_index_for(&self, char_index: usize) -> usize {
        self.value
            .char_indices()
            .nth(char_index)
            .map(|(byte, _)| byte)
            .unwrap_or(self.value.len())
    }

    fn clamp_len(&mut self) {
        if self.value.chars().count() > self.max_chars {
            let truncated: String = self.value.chars().take(self.max_chars).collect();
            self.value = truncated;
        }
        self.cursor = self.cursor.min(self.value.chars().count());
    }
}

/// Drop control characters (notably pasted newlines) and keep a sensible length.
pub fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch == '\n' || ch == '\r' || ch == '\t' {
                ' '
            } else if ch.is_control() {
                '\u{0}'
            } else {
                ch
            }
        })
        .filter(|ch| *ch != '\u{0}')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_respects_char_boundaries() {
        let mut input = TextInput::new("a中b");
        assert_eq!(input.cursor(), 3);
        input.home();
        input.right();
        input.insert_char('X');
        assert_eq!(input.value(), "aX中b");
        input.backspace();
        assert_eq!(input.value(), "a中b");
        input.end();
        input.backspace();
        assert_eq!(input.value(), "a中");
        input.home();
        input.delete();
        assert_eq!(input.value(), "中");
        input.kill_to_start();
        assert_eq!(input.value(), "中");
    }

    #[test]
    fn default_input_has_a_usable_capacity() {
        let mut input = TextInput::default();
        input.set("sodium");
        assert_eq!(input.value(), "sodium");
        input.insert_char('!');
        assert_eq!(input.value(), "sodium!");
    }

    #[test]
    fn control_characters_and_length_are_sanitized() {
        let mut input = TextInput::new("a\nb\tc").with_max_chars(4);
        assert_eq!(input.value(), "a b ");
        input.insert_str("danger");
        assert_eq!(input.value().chars().count(), 4);
        assert!(!sanitize("a\u{7}b").contains('\u{7}'));
    }
}
