//! Root span naming for PHP HTTP / CLI requests.

use std::path::Path;

/// Pick the best HTTP path among candidates (strip query; prefer non-front-controller).
///
/// FPM/`try_files` often sets SAPI `request_uri` to `/index.php`; prefer a more
/// specific route when present (e.g. `$_SERVER['REQUEST_URI']` = `/v1/crm/login`).
pub fn prefer_http_path<'a>(candidates: &'a [Option<&'a str>]) -> &'a str {
    let mut fallback: Option<&str> = None;
    for c in candidates {
        let Some(raw) = c.map(str::trim).filter(|p| !p.is_empty()) else {
            continue;
        };
        let path = raw.split('?').next().unwrap_or(raw);
        if path.is_empty() {
            continue;
        }
        if !is_front_controller(path) {
            return path;
        }
        if fallback.is_none() {
            fallback = Some(path);
        }
    }
    fallback.unwrap_or("/")
}

fn is_front_controller(path: &str) -> bool {
    path == "/index.php"
        || path.ends_with("/index.php")
        || path == "index.php"
}

fn script_basename(script_path: Option<&str>) -> &str {
    script_path
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .and_then(|p| Path::new(p).file_name())
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("php")
}

/// CLI root name from script path + process argv.
///
/// Examples: `CLI artisan db:seed`, `CLI artisan`, `CLI console.php cache:clear`.
pub fn cli_span_name(script_path: Option<&str>, argv: &[impl AsRef<str>]) -> String {
    let basename = script_basename(script_path);
    match cli_command_from_argv(basename, argv) {
        Some(cmd) => format!("CLI {basename} {cmd}"),
        None => format!("CLI {basename}"),
    }
}

/// First non-option arg after the script entry in `argv`.
fn cli_command_from_argv<'a>(basename: &str, argv: &'a [impl AsRef<str>]) -> Option<&'a str> {
    let start = argv
        .iter()
        .position(|a| {
            Path::new(a.as_ref())
                .file_name()
                .and_then(|s| s.to_str())
                == Some(basename)
        })
        .map(|i| i + 1)
        .unwrap_or(1); // skip argv[0] binary when script not found in argv

    argv.get(start..)
        .into_iter()
        .flatten()
        .map(|a| a.as_ref())
        .find(|a| !a.is_empty() && !a.starts_with('-'))
}

/// Build a root span name from optional SAPI / env request fields.
///
/// - HTTP: `{METHOD} {path}` (query string stripped)
/// - CLI (no method): `CLI {script_basename}` (use [`cli_span_name`] when argv is available)
pub fn root_span_name(
    method: Option<&str>,
    request_uri: Option<&str>,
    path_translated: Option<&str>,
) -> String {
    root_span_name_from_candidates(method, &[request_uri, path_translated], path_translated)
}

/// Like [`root_span_name`] but with an ordered list of URI candidates.
pub fn root_span_name_from_candidates(
    method: Option<&str>,
    uri_candidates: &[Option<&str>],
    path_translated: Option<&str>,
) -> String {
    let method = method.map(str::trim).filter(|m| !m.is_empty());
    match method {
        Some(m) => {
            let path = prefer_http_path(uri_candidates);
            format!("{m} {path}")
        }
        None => {
            let script = path_translated
                .or_else(|| uri_candidates.iter().flatten().copied().next());
            cli_span_name(script, &[] as &[&str])
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

    #[test]
    fn prefers_route_over_index_php() {
        assert_eq!(
            root_span_name_from_candidates(
                Some("POST"),
                &[Some("/index.php"), Some("/v1/crm/login")],
                Some("/var/www/public/index.php"),
            ),
            "POST /v1/crm/login"
        );
    }

    #[test]
    fn keeps_index_php_when_only_option() {
        assert_eq!(
            root_span_name_from_candidates(
                Some("GET"),
                &[Some("/index.php")],
                None,
            ),
            "GET /index.php"
        );
    }

    #[test]
    fn cli_artisan_with_command() {
        let argv = ["php", "artisan", "db:seed"];
        assert_eq!(
            cli_span_name(Some("/app/artisan"), &argv),
            "CLI artisan db:seed"
        );
    }

    #[test]
    fn cli_artisan_skips_flags_before_command() {
        let argv = ["php", "/var/www/artisan", "--env=testing", "db:seed"];
        assert_eq!(
            cli_span_name(Some("/var/www/artisan"), &argv),
            "CLI artisan db:seed"
        );
    }

    #[test]
    fn cli_artisan_command_then_flags() {
        let argv = ["php", "artisan", "migrate", "--force"];
        assert_eq!(
            cli_span_name(Some("artisan"), &argv),
            "CLI artisan migrate"
        );
    }

    #[test]
    fn cli_artisan_no_command() {
        let argv = ["php", "artisan"];
        assert_eq!(cli_span_name(Some("artisan"), &argv), "CLI artisan");
    }

    #[test]
    fn cli_non_artisan_script() {
        let argv = ["php", "bin/console.php", "cache:clear"];
        assert_eq!(
            cli_span_name(Some("/app/bin/console.php"), &argv),
            "CLI console.php cache:clear"
        );
    }
}
