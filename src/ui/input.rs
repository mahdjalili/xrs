use unicode_width::UnicodeWidthStr;

/// Single-line text field with a byte-indexed cursor that always sits on a
/// char boundary.
#[derive(Debug, Default, Clone)]
pub struct TextInput {
    value: String,
    cursor: usize,
}

impl TextInput {
    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    pub fn insert(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        self.value.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert(c);
        }
    }

    pub fn backspace(&mut self) {
        if let Some(prev) = self.prev_boundary() {
            self.value.replace_range(prev..self.cursor, "");
            self.cursor = prev;
        }
    }

    pub fn delete(&mut self) {
        if let Some(next) = self.next_boundary() {
            self.value.replace_range(self.cursor..next, "");
        }
    }

    pub fn delete_word(&mut self) {
        let before = &self.value[..self.cursor];
        let trimmed = before.trim_end();
        let start = trimmed.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
        self.value.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    pub fn left(&mut self) {
        if let Some(prev) = self.prev_boundary() {
            self.cursor = prev;
        }
    }

    pub fn right(&mut self) {
        if let Some(next) = self.next_boundary() {
            self.cursor = next;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.value.len();
    }

    /// Display column of the cursor, accounting for wide glyphs.
    pub fn cursor_col(&self) -> usize {
        self.value[..self.cursor].width()
    }

    /// Returns the slice that fits in `width` columns while keeping the cursor
    /// visible, plus the cursor's column within that slice.
    pub fn viewport(&self, width: usize) -> (&str, usize) {
        if width == 0 {
            return ("", 0);
        }
        let mut start = 0;
        let mut col = self.cursor_col();
        let mut iter = self.value.char_indices();
        while col >= width {
            let Some((_, c)) = iter.next() else { break };
            let w = UnicodeWidthStr::width(c.encode_utf8(&mut [0; 4]) as &str);
            start += c.len_utf8();
            col = col.saturating_sub(w);
        }
        let visible = &self.value[start..];
        let mut end = visible.len();
        let mut used = 0;
        for (i, c) in visible.char_indices() {
            let w = UnicodeWidthStr::width(c.encode_utf8(&mut [0; 4]) as &str);
            if used + w > width {
                end = i;
                break;
            }
            used += w;
        }
        (&visible[..end], col)
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.value[..self.cursor].char_indices().next_back().map(|(i, _)| i)
    }

    fn next_boundary(&self) -> Option<usize> {
        self.value[self.cursor..].chars().next().map(|c| self.cursor + c.len_utf8())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_around_multibyte_chars() {
        let mut input = TextInput::default();
        input.insert_str("a🇫🇮b");
        input.left();
        input.backspace();
        input.backspace();
        assert_eq!(input.value(), "ab");
        input.home();
        input.delete();
        assert_eq!(input.value(), "b");
        input.end();
        input.insert('c');
        assert_eq!(input.value(), "bc");
    }

    #[test]
    fn strips_control_characters_from_paste() {
        let mut input = TextInput::default();
        input.insert_str("vless://x\r\n");
        assert_eq!(input.value(), "vless://x");
    }

    #[test]
    fn delete_word_removes_last_token() {
        let mut input = TextInput::default();
        input.insert_str("fi1 reality ");
        input.delete_word();
        assert_eq!(input.value(), "fi1 ");
    }

    #[test]
    fn viewport_keeps_cursor_visible() {
        let mut input = TextInput::default();
        input.insert_str("0123456789");
        let (slice, col) = input.viewport(4);
        assert_eq!(slice, "789");
        assert_eq!(col, 3);
        input.home();
        let (slice, col) = input.viewport(4);
        assert_eq!(slice, "0123");
        assert_eq!(col, 0);
    }
}
