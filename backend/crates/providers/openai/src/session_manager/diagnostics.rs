//! 重写专用结构化诊断；State 按管理员要求保留，鉴权与代理认证仍脱敏。

use reqwest::header::HeaderMap;
use serde_json::{Value, json};

pub(super) fn headers(headers: &HeaderMap) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|(name, value)| {
                json!({"name": name.as_str(), "value": if sensitive(name.as_str()) {
            "[REDACTED]".to_owned()
        } else { String::from_utf8_lossy(value.as_bytes()).into_owned() }})
            })
            .collect(),
    )
}

pub(super) fn redact(value: &mut Value, secrets: &[&str]) {
    match value {
        Value::Object(object) => {
            for (name, value) in object {
                if sensitive(name) {
                    *value = Value::String("[REDACTED]".to_owned());
                } else {
                    redact(value, secrets);
                }
            }
        }
        Value::Array(values) => values.iter_mut().for_each(|value| redact(value, secrets)),
        Value::String(text) => {
            for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
                *text = text.replace(secret, "[REDACTED]");
            }
        }
        _ => {}
    }
}

fn sensitive(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase().replace(['-', '_'], "");
    if normalized.ends_with("token") || normalized.ends_with("secret") {
        return true;
    }
    matches!(
        normalized.as_str(),
        "authorization"
            | "proxyauthorization"
            | "cookie"
            | "setcookie"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "apikey"
            | "xapikey"
            | "credentials"
            | "credential"
            | "password"
            | "secret"
    )
}

pub(super) struct ProbeLog<'a> {
    pub account_id: &'a str,
    pub model: &'a str,
    pub probe_id: &'a str,
    pub attempt: u32,
    pub secrets: Vec<&'a str>,
}

impl ProbeLog<'_> {
    pub fn record(&self, stage: &'static str, mut details: Value) {
        redact(&mut details, &self.secrets);
        tracing::info!(target: "session_keepalive",
            account_id = self.account_id, model = self.model, probe_id = self.probe_id,
            attempt = self.attempt, stage, details = %details, "Session keepalive diagnostic");
    }
}
