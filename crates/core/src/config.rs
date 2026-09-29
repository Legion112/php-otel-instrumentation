//! Environment-based OTEL configuration (matches infra common.env / Go libs).

use std::env;

/// Runtime config read from process environment.
#[derive(Debug, Clone, PartialEq)]
pub struct OtelConfig {
    pub enabled: bool,
    /// Host:port or full URL base (e.g. `alloy:4318` or `http://alloy:4318`).
    pub endpoint: String,
    pub sample_rate: f64,
    pub service_name: String,
}

impl OtelConfig {
    /// Load from environment. Defaults align with Alanbase local stack.
    pub fn from_env() -> Self {
        Self::from_env_map(|k| env::var(k).ok())
    }

    /// Testable loader using a key→value lookup.
    pub fn from_env_map<F>(mut get: F) -> Self
    where
        F: FnMut(&str) -> Option<String>,
    {
        let enabled = parse_bool(get("OTEL_ENABLED").as_deref()).unwrap_or(true);
        let endpoint = get("OTEL_ENDPOINT").unwrap_or_else(|| "alloy:4318".to_string());
        let sample_rate: f64 = get("OTEL_SAMPLE_RATE")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.0);
        let sample_rate = sample_rate.clamp(0.0, 1.0);
        let service_name = get("OTEL_SERVICE_NAME")
            .or_else(|| get("HOSTNAME"))
            .unwrap_or_else(|| "php".to_string());

        Self {
            enabled,
            endpoint,
            sample_rate,
            service_name,
        }
    }

    /// Whether this process should emit spans (enabled and sample bucket).
    pub fn should_sample(&self) -> bool {
        if !self.enabled || self.sample_rate <= 0.0 {
            return false;
        }
        if self.sample_rate >= 1.0 {
            return true;
        }
        rand::random::<f64>() < self.sample_rate
    }

    /// Full OTLP HTTP traces URL.
    pub fn traces_url(&self) -> String {
        let base = self.endpoint.trim().trim_end_matches('/');
        if base.contains("://") {
            if base.ends_with("/v1/traces") {
                base.to_string()
            } else {
                format!("{base}/v1/traces")
            }
        } else {
            format!("http://{base}/v1/traces")
        }
    }
}

fn parse_bool(v: Option<&str>) -> Option<bool> {
    match v?.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn map_get<'a>(m: &'a HashMap<&str, &str>) -> impl FnMut(&str) -> Option<String> + 'a {
        move |k| m.get(k).map(|s| (*s).to_string())
    }

    #[test]
    fn defaults_match_common_env() {
        let cfg = OtelConfig::from_env_map(|_| None);
        assert!(cfg.enabled);
        assert_eq!(cfg.endpoint, "alloy:4318");
        assert_eq!(cfg.sample_rate, 1.0);
        assert_eq!(cfg.service_name, "php");
        assert_eq!(cfg.traces_url(), "http://alloy:4318/v1/traces");
    }

    #[test]
    fn disabled_when_otel_enabled_false() {
        let mut m = HashMap::new();
        m.insert("OTEL_ENABLED", "false");
        let cfg = OtelConfig::from_env_map(map_get(&m));
        assert!(!cfg.enabled);
        assert!(!cfg.should_sample());
    }

    #[test]
    fn sample_rate_zero_never_samples() {
        let mut m = HashMap::new();
        m.insert("OTEL_SAMPLE_RATE", "0");
        let cfg = OtelConfig::from_env_map(map_get(&m));
        assert!(!cfg.should_sample());
    }

    #[test]
    fn full_url_endpoint() {
        let mut m = HashMap::new();
        m.insert("OTEL_ENDPOINT", "http://host.docker.internal:4318");
        let cfg = OtelConfig::from_env_map(map_get(&m));
        assert_eq!(
            cfg.traces_url(),
            "http://host.docker.internal:4318/v1/traces"
        );
    }

    #[test]
    fn service_name_from_hostname() {
        let mut m = HashMap::new();
        m.insert("HOSTNAME", "alanbase");
        let cfg = OtelConfig::from_env_map(map_get(&m));
        assert_eq!(cfg.service_name, "alanbase");
    }
}
