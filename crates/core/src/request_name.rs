//! Root span naming for PHP HTTP / CLI requests.

use std::path::Path;

/// Build a root span name from optional SAPI / env request fields.
///
/// - HTTP: `{METHOD} {path}` (query string stripped)
/// - CLI (no method): `CLI {script_basename}`
pub fn root_span_name(
    method: Option<&str>,
    request_uri: Option<&str>,
    path_translated: Option<&str>,
) -> String {
    let method = method.map(str::trim).filter(|m| !m.is_empty());
    match method {
        Some(m) => {
            let path = request_uri
                .or(path_translated)
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .unwrap_or("/");
            let path = path.split('?').next().unwrap_or(path);
            let path = if path.is_empty() { "/" } else { path };
            format!("{m} {path}")
        }
        None => {
            let script = path_translated
                .or(request_uri)
                .and_then(|p| Path::new(p).file_name())
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("php");
            format!("CLI {script}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_get_path() {
        assert_eq!(
            root_span_name(Some("GET"), Some("/v1/crm/login"), None),
            "GET /v1/crm/login"
        );
    }

    #[test]
    fn strips_query_string() {
        assert_eq!(
            root_span_name(Some("POST"), Some("/api/foo?x=1&y=2"), None),
            "POST /api/foo"
        );
    }

    #[test]
    fn falls_back_to_path_translated() {
        assert_eq!(
            root_span_name(Some("GET"), None, Some("/var/www/public/index.php")),
            "GET /var/www/public/index.php"
        );
    }

    #[test]
    fn cli_uses_script_basename() {
        assert_eq!(
            root_span_name(None, None, Some("/var/www/artisan")),
            "CLI artisan"
        );
        assert_eq!(
            root_span_name(None, None, Some("/app/bin/console.php")),
            "CLI console.php"
        );
    }

    #[test]
    fn cli_default_when_no_script() {
        assert_eq!(root_span_name(None, None, None), "CLI php");
    }

    #[test]
    fn empty_method_treated_as_cli() {
        assert_eq!(
            root_span_name(Some("  "), Some("/x"), Some("/app/artisan")),
            "CLI artisan"
        );
    }
}
