//! Unicode-safe text helpers for TUI rendering and previews.

/// Return at most `max_chars` Unicode scalar values from `text`.
pub fn take_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// Truncate `text` by characters and append `...` when truncation occurs.
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    truncate_chars_with_suffix(text, max_chars, "...")
}

/// Truncate `text` by characters and append `suffix` when truncation occurs.
pub fn truncate_chars_with_suffix(text: &str, max_chars: usize, suffix: &str) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let suffix_len = suffix.chars().count();
    if max_chars <= suffix_len {
        return take_chars(suffix, max_chars);
    }

    let mut out = take_chars(text, max_chars - suffix_len);
    out.push_str(suffix);
    out
}

/// Clamp a byte index down to the nearest valid UTF-8 character boundary.
pub fn clamp_to_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Return the byte index after `char_count` Unicode scalar values.
pub fn byte_index_after_chars(text: &str, char_count: usize) -> usize {
    text.char_indices()
        .nth(char_count)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len())
}

/// Split `text` into chunks containing at most `max_chars` characters.
pub fn chunk_chars(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return vec![String::new()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if current.chars().count() >= max_chars {
            chunks.push(current);
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() || chunks.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Hard-wrap a line by character count without slicing UTF-8 byte offsets.
pub fn wrap_line_chars(line: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![String::new()];
    }
    if line.chars().count() <= max_width {
        return vec![line.to_string()];
    }

    let mut result = Vec::new();
    let mut current_line = String::new();

    for word in line.split_whitespace() {
        let word_len = word.chars().count();
        let current_len = current_line.chars().count();

        if current_line.is_empty() {
            if word_len > max_width {
                result.extend(chunk_chars(word, max_width));
            } else {
                current_line = word.to_string();
            }
        } else if current_len + 1 + word_len <= max_width {
            current_line.push(' ');
            current_line.push_str(word);
        } else {
            result.push(current_line);
            if word_len > max_width {
                result.extend(chunk_chars(word, max_width));
                current_line = String::new();
            } else {
                current_line = word.to_string();
            }
        }
    }

    if !current_line.is_empty() {
        result.push(current_line);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_curly_quotes_and_dash_without_byte_boundary_panic() {
        let text = "Plan “alpha” — verify Unicode safety";
        let truncated = truncate_chars(text, 14);
        assert_eq!(truncated, "Plan “alpha...");
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn wraps_long_unicode_words_without_byte_boundary_panic() {
        let wrapped = wrap_line_chars("“unicode-heavy-token”—continues", 5);
        assert!(wrapped.len() > 1);
        assert!(wrapped.iter().all(|line| line.is_char_boundary(line.len())));
    }

    #[test]
    fn clamps_byte_index_to_valid_boundary() {
        let text = "a—b";
        assert_eq!(clamp_to_char_boundary(text, 2), 1);
        assert_eq!(clamp_to_char_boundary(text, 4), 4);
    }
}
