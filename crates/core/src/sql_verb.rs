//! Derive a short span name from the leading SQL verb.

/// First contiguous ASCII alphabetic word in `sql` (after trim / leading `(`).
pub fn sql_verb(sql: &str) -> Option<&str> {
    let s = sql.trim();
    let s = s.trim_start_matches(|c: char| c == '(' || c.is_whitespace());
    if s.is_empty() {
        return None;
    }
    let end = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_alphabetic())
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    if end == 0 {
        None
    } else {
        Some(&s[..end])
    }
}

/// Span name: `SQL: SELECT`, `SQL: SET`, … or `SQL: QUERY` when no verb.
pub fn sql_span_name(sql: &str) -> String {
    match sql_verb(sql) {
        Some(v) => format!("SQL: {}", v.to_ascii_uppercase()),
        None => "SQL: QUERY".to_string(),
    }
}

/// Lowercase verb for `db.operation`, or `"query"` when missing.
pub fn sql_operation(sql: &str) -> String {
    sql_verb(sql)
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| "query".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_verb() {
        assert_eq!(
            sql_verb(r#"select * from "oauth_clients" where id = ?"#),
            Some("select")
        );
        assert_eq!(
            sql_span_name(r#"select * from "oauth_clients" where id = ?"#),
            "SQL: SELECT"
        );
        assert_eq!(
            sql_operation(r#"select * from "oauth_clients" where id = ?"#),
            "select"
        );
    }

    #[test]
    fn set_verb() {
        assert_eq!(sql_verb("set names 'utf8'"), Some("set"));
        assert_eq!(sql_span_name("set names 'utf8'"), "SQL: SET");
    }

    #[test]
    fn insert_update_delete() {
        assert_eq!(sql_span_name("INSERT INTO t VALUES (1)"), "SQL: INSERT");
        assert_eq!(sql_span_name("update t set a=1"), "SQL: UPDATE");
        assert_eq!(sql_span_name("DELETE FROM t"), "SQL: DELETE");
    }

    #[test]
    fn with_cte() {
        assert_eq!(
            sql_span_name("WITH x AS (SELECT 1) SELECT * FROM x"),
            "SQL: WITH"
        );
    }

    #[test]
    fn leading_whitespace_and_paren() {
        assert_eq!(sql_span_name("  \nselect 1"), "SQL: SELECT");
        assert_eq!(sql_span_name("(select 1)"), "SQL: SELECT");
    }

    #[test]
    fn empty_fallback() {
        assert_eq!(sql_verb(""), None);
        assert_eq!(sql_verb("   "), None);
        assert_eq!(sql_span_name(""), "SQL: QUERY");
        assert_eq!(sql_operation(""), "query");
        assert_eq!(sql_span_name("123 not sql"), "SQL: QUERY");
    }
}
