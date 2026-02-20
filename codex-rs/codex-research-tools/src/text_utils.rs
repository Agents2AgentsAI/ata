pub(crate) fn truncate_chars(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    if max_chars <= 3 {
        return value.chars().take(max_chars).collect::<String>();
    }

    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::truncate_chars;

    #[test]
    fn truncate_chars_handles_small_budgets() {
        assert_eq!(truncate_chars("abcdef", 0), "");
        assert_eq!(truncate_chars("abcdef", 2), "ab");
        assert_eq!(truncate_chars("abcdef", 3), "abc");
    }

    #[test]
    fn truncate_chars_appends_ellipsis_for_larger_budgets() {
        assert_eq!(truncate_chars("abcdefghij", 5), "ab...");
        assert_eq!(truncate_chars("abc", 5), "abc");
    }
}
