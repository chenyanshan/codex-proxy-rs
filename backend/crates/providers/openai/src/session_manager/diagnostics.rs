//! 探活专用结构化诊断；State 按管理员要求保留，鉴权与代理认证仍脱敏。

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

/// 只记录完整结构化字段，截断或非 JSON 内容保留长度和遗漏原因，避免半个凭据绕过脱敏。
pub(super) fn response_body(bytes: &[u8], incomplete: bool) -> Value {
    if !incomplete && let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        return value;
    }
    let text = String::from_utf8_lossy(bytes);
    let normalized = text.replace("\r\n", "\n");
    let mut frames: Vec<&str> = normalized.split("\n\n").collect();
    let trailing = frames.pop().unwrap_or_default();
    let mut omitted = usize::from(!trailing.is_empty());
    let mut events = Vec::new();
    for frame in frames {
        let data = frame
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if data == "[DONE]" {
            events.push(json!({"data":"[DONE]"}));
        } else if let Ok(value) = serde_json::from_str::<Value>(&data) {
            let fields = frame
                .lines()
                .filter(|line| !line.starts_with("data:"))
                .filter_map(|line| line.split_once(':'))
                .map(|(name, value)| {
                    (
                        name.to_owned(),
                        Value::String(value.trim_start().to_owned()),
                    )
                })
                .collect::<serde_json::Map<_, _>>();
            events.push(json!({"fields":fields, "data":value}));
        } else {
            omitted += 1;
        }
    }
    json!({"events":events,"omittedUnstructuredOrIncompleteFrames":omitted,"capturedBytes":bytes.len()})
}
