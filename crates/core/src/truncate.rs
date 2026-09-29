//! Truncate DB statements for span attributes (mirror postback_processor style).

const DEFAULT_MAX: usize = 512;

/// Truncate a statement to `max` bytes on a UTF-8 boundary, appending `...` when cut.
pub fn truncate_statement(s: &str, max: usize) -> String {
    let max = if max == 0 { DEFAULT_MAX } else { max };
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max.saturating_sub(3);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_unchanged() {
        assert_eq!(truncate_statement("select 1", 512), "select 1");
    }

    #[test]
    fn truncates_long() {
        let s = "a".repeat(600);
        let out = truncate_statement(&s, 100);
        assert!(out.ends_with("..."));
        assert!(out.len() <= 100);
    }
}
