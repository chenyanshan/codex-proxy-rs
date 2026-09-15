//! Responses wire 到 Chat 消息的有状态投影；推理正文不进入 Chat 输出。

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use gateway_core::engine::ModelRequestId;
use gateway_core::event::{GatewayEvent, ProviderEvent};
use serde_json::{Value, json};

use crate::openai::responses::{ProtocolError, ProtocolErrorBody};

#[derive(Default)]
struct Tool {
    index: usize,
    id: String,
    name: String,
    arguments: String,
}

pub(in crate::openai) struct ChatEncoder {
    id: String,
    created: u64,
    model: String,
    include_usage: bool,
    role_sent: bool,
    text: BTreeMap<(u64, u64), String>,
    refusal: BTreeMap<(u64, u64), String>,
    tools: BTreeMap<u64, Tool>,
    finish_reason: Option<&'static str>,
    usage: Option<Value>,
    terminal_frames: Vec<Bytes>,
    wire_failure: bool,
    canonical_completed: bool,
}

impl ChatEncoder {
    pub(in crate::openai) fn new(
        id: &ModelRequestId,
        created: SystemTime,
        model: String,
        include_usage: bool,
    ) -> Self {
        Self {
            id: format!("chatcmpl-{}", id.as_str()),
            created: created
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            model,
            include_usage,
            role_sent: false,
            text: BTreeMap::new(),
            refusal: BTreeMap::new(),
            tools: BTreeMap::new(),
            finish_reason: None,
            usage: None,
            terminal_frames: Vec::new(),
            wire_failure: false,
            canonical_completed: false,
        }
    }

    pub(in crate::openai) fn is_completed(&self) -> bool {
        self.finish_reason.is_some()
    }

    pub(in crate::openai) fn has_wire_failure(&self) -> bool {
        self.wire_failure
    }

    pub(in crate::openai) fn push_sse(
        &mut self,
        event: &ProviderEvent,
    ) -> Result<Vec<Bytes>, ProtocolErrorBody> {
        self.canonical_completed |= event
            .canonical_facts()
            .iter()
            .any(|fact| matches!(fact, GatewayEvent::Completed(_)));
        let mut frames = Vec::new();
        // 首批只有 created、推理或心跳时也给 Core 一个可提交的角色帧。
        if !self.role_sent {
            self.role_sent = true;
            frames.push(self.delta(json!({"role":"assistant","content":""})));
        }
        let Some(wire) = event
            .wire_event()
            .filter(|wire| wire.protocol() == "openai" && wire.has_json_data())
        else {
            return Ok(frames);
        };
        let data = wire.data();
        let event_type = wire
            .event_type()
            .or_else(|| data.get("type").and_then(Value::as_str));
        if self.is_completed() || self.wire_failure {
            return Err(invalid_response());
        }
        match event_type {
            Some("response.output_text.delta" | "response.refusal.delta") => {
                let key = content_key(data)?;
                let value = required_str(data, "delta")?;
                let refusal = event_type == Some("response.refusal.delta");
                let field = if refusal { "refusal" } else { "content" };
                let content = if refusal {
                    &mut self.refusal
                } else {
                    &mut self.text
                };
                content.entry(key).or_default().push_str(value);
                frames.push(self.delta(json!({field:value})));
            }
            Some("response.output_text.done" | "response.refusal.done") => {
                let refusal = event_type == Some("response.refusal.done");
                let value = required_str(data, if refusal { "refusal" } else { "text" })?;
                self.text_snapshot(content_key(data)?, value, refusal, &mut frames)?;
            }
            Some("response.content_part.done") => {
                self.part(
                    content_key(data)?,
                    data.get("part").ok_or_else(invalid_response)?,
                    &mut frames,
                )?;
            }
            Some("response.output_item.added" | "response.output_item.done") => {
                self.item(
                    required_index(data, "output_index")?,
                    data.get("item").ok_or_else(invalid_response)?,
                    &mut frames,
                )?;
            }
            Some("response.function_call_arguments.delta") => {
                let index = required_index(data, "output_index")?;
                let delta = required_str(data, "delta")?;
                let tool = self.tools.get_mut(&index).ok_or_else(invalid_response)?;
                tool.arguments.push_str(delta);
                let tool_index = tool.index;
                frames.push(self.delta(
                    json!({"tool_calls":[{"index":tool_index,"function":{"arguments":delta}}]}),
                ));
            }
            Some("response.function_call_arguments.done") => {
                let index = required_index(data, "output_index")?;
                self.arguments_snapshot(index, required_str(data, "arguments")?, &mut frames)?;
            }
            Some("response.completed" | "response.incomplete") => {
                let response = data
                    .get("response")
                    .filter(|value| value.is_object())
                    .ok_or_else(invalid_response)?;
                let status = required_str(response, "status")?;
                if !matches!(status, "completed" | "incomplete")
                    || (event_type == Some("response.incomplete") && status != "incomplete")
                {
                    return Err(invalid_response());
                }
                let mut terminal_frames = Vec::new();
                if let Some(output) = response.get("output") {
                    for (index, item) in output
                        .as_array()
                        .ok_or_else(invalid_response)?
                        .iter()
                        .enumerate()
                    {
                        self.item(
                            u64::try_from(index).map_err(|_| invalid_response())?,
                            item,
                            &mut terminal_frames,
                        )?;
                    }
                } else if self.text.is_empty() && self.refusal.is_empty() && self.tools.is_empty() {
                    return Err(invalid_response());
                }
                self.usage = response
                    .get("usage")
                    .filter(|value| !value.is_null())
                    .map(chat_usage)
                    .transpose()?;
                self.finish_reason = Some(
                    if event_type == Some("response.incomplete")
                        || response.get("status").and_then(Value::as_str) == Some("incomplete")
                    {
                        match response
                            .pointer("/incomplete_details/reason")
                            .and_then(Value::as_str)
                        {
                            Some("max_output_tokens") => "length",
                            Some("content_filter") => "content_filter",
                            _ => return Err(invalid_response()),
                        }
                    } else if self.tools.is_empty() {
                        "stop"
                    } else {
                        "tool_calls"
                    },
                );
                // 终态快照补出的内容与 finish/usage 必须在执行终结成功后一起交付。
                self.terminal_frames = terminal_frames;
            }
            Some("response.failed" | "error") => {
                let error = data
                    .pointer("/response/error")
                    .or_else(|| data.get("error"))
                    .unwrap_or(data);
                let message = required_str(error, "message")?;
                self.wire_failure = true;
                frames.push(frame(&json!({"error": {
                    "message": message,
                    "type": error.get("type").and_then(Value::as_str).unwrap_or("server_error"),
                    "code": error.get("code").cloned().unwrap_or(Value::Null),
                    "param": error.get("param").cloned().unwrap_or(Value::Null),
                }})));
            }
            _ => {}
        }
        Ok(frames)
    }

    // 必须在 Core 的下一次轮询结算成功前发现不可投影的终态。
    pub(in crate::openai) fn validate_batch(&self) -> Result<(), ProtocolErrorBody> {
        if self.canonical_completed && !self.is_completed() && !self.wire_failure {
            return Err(invalid_response());
        }
        Ok(())
    }

    fn text_snapshot(
        &mut self,
        key: (u64, u64),
        value: &str,
        refusal: bool,
        frames: &mut Vec<Bytes>,
    ) -> Result<(), ProtocolErrorBody> {
        let content = if refusal {
            &mut self.refusal
        } else {
            &mut self.text
        };
        let accumulated = content.entry(key).or_default();
        let suffix = value
            .strip_prefix(accumulated.as_str())
            .ok_or_else(invalid_response)?
            .to_owned();
        value.clone_into(accumulated);
        if !suffix.is_empty() {
            frames.push(self.delta(json!({if refusal { "refusal" } else { "content" }:suffix})));
        }
        Ok(())
    }

    fn part(
        &mut self,
        key: (u64, u64),
        part: &Value,
        frames: &mut Vec<Bytes>,
    ) -> Result<(), ProtocolErrorBody> {
        match part.get("type").and_then(Value::as_str) {
            Some("output_text") => {
                self.text_snapshot(key, required_str(part, "text")?, false, frames)
            }
            Some("refusal") => {
                self.text_snapshot(key, required_str(part, "refusal")?, true, frames)
            }
            _ => Err(invalid_response()),
        }
    }

    fn item(
        &mut self,
        index: u64,
        item: &Value,
        frames: &mut Vec<Bytes>,
    ) -> Result<(), ProtocolErrorBody> {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                if required_str(item, "role")? != "assistant" {
                    return Err(invalid_response());
                }
                for (part_index, part) in item
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or_else(invalid_response)?
                    .iter()
                    .enumerate()
                {
                    self.part(
                        (
                            index,
                            u64::try_from(part_index).map_err(|_| invalid_response())?,
                        ),
                        part,
                        frames,
                    )?;
                }
            }
            Some("function_call") => {
                let id = required_str(item, "call_id")?;
                let name = required_str(item, "name")?;
                let next_index = self.tools.len();
                if let Some(tool) = self.tools.get(&index) {
                    if tool.id != id || tool.name != name {
                        return Err(invalid_response());
                    }
                } else {
                    self.tools.insert(
                        index,
                        Tool {
                            index: next_index,
                            id: id.to_owned(),
                            name: name.to_owned(),
                            arguments: String::new(),
                        },
                    );
                    frames.push(self.delta(json!({"tool_calls":[{"index":next_index,"id":id,"type":"function","function":{"name":name,"arguments":""}}]})));
                }
                self.arguments_snapshot(index, required_str(item, "arguments")?, frames)?;
            }
            Some("reasoning") => {}
            _ => return Err(invalid_response()),
        }
        Ok(())
    }

    fn arguments_snapshot(
        &mut self,
        index: u64,
        value: &str,
        frames: &mut Vec<Bytes>,
    ) -> Result<(), ProtocolErrorBody> {
        let tool = self.tools.get_mut(&index).ok_or_else(invalid_response)?;
        let suffix = value
            .strip_prefix(&tool.arguments)
            .ok_or_else(invalid_response)?
            .to_owned();
        value.clone_into(&mut tool.arguments);
        let tool_index = tool.index;
        if !suffix.is_empty() {
            frames.push(self.delta(
                json!({"tool_calls":[{"index":tool_index,"function":{"arguments":suffix}}]}),
            ));
        }
        Ok(())
    }

    fn envelope(&self, object: &str, choices: Value) -> Value {
        json!({"id":self.id,"object":object,"created":self.created,"model":self.model,"choices":choices})
    }

    fn delta(&self, delta: Value) -> Bytes {
        let mut value = self.envelope(
            "chat.completion.chunk",
            json!([{"index":0,"delta":delta,"finish_reason":null}]),
        );
        if self.include_usage {
            value["usage"] = Value::Null;
        }
        frame(&value)
    }

    pub(in crate::openai) fn completed_frames(&mut self) -> Vec<Bytes> {
        let mut frames = std::mem::take(&mut self.terminal_frames);
        let mut finish = self.envelope(
            "chat.completion.chunk",
            json!([{"index":0,"delta":{},"finish_reason":self.finish_reason}]),
        );
        if self.include_usage {
            finish["usage"] = Value::Null;
        }
        frames.push(frame(&finish));
        if self.include_usage {
            let mut value = self.envelope("chat.completion.chunk", json!([]));
            value["usage"] = self.usage.clone().unwrap_or(Value::Null);
            frames.push(frame(&value));
        }
        frames
    }

    pub(in crate::openai) fn finish(self) -> Result<Value, ProtocolErrorBody> {
        let finish_reason = self.finish_reason.ok_or_else(invalid_response)?;
        let mut message = json!({"role":"assistant","content":null,"refusal":null});
        if !self.text.is_empty() {
            message["content"] = Value::String(self.text.values().cloned().collect());
        }
        if !self.refusal.is_empty() {
            message["refusal"] = Value::String(self.refusal.values().cloned().collect());
        }
        if !self.tools.is_empty() {
            let mut tools: Vec<_> = self.tools.values().collect();
            tools.sort_by_key(|tool| tool.index);
            message["tool_calls"] = Value::Array(tools.iter().map(|tool| json!({"id":tool.id,"type":"function","function":{"name":tool.name,"arguments":tool.arguments}})).collect());
        }
        let mut response = self.envelope(
            "chat.completion",
            json!([{"index":0,"message":message,"finish_reason":finish_reason,"logprobs":null}]),
        );
        if let Some(usage) = self.usage {
            response["usage"] = usage;
        }
        Ok(response)
    }
}

fn chat_usage(usage: &Value) -> Result<Value, ProtocolErrorBody> {
    let input = required_index(usage, "input_tokens")?;
    let output = required_index(usage, "output_tokens")?;
    let total = input.checked_add(output).ok_or_else(invalid_response)?;
    if let Some(supplied) = usage.get("total_tokens")
        && supplied.as_u64() != Some(total)
    {
        return Err(invalid_response());
    }
    let mut value = json!({"prompt_tokens":input,"completion_tokens":output,"total_tokens":total});
    for (source, target, fields) in [
        (
            "input_tokens_details",
            "prompt_tokens_details",
            &[
                "cached_tokens",
                "cache_write_tokens",
                "audio_tokens",
                "image_tokens",
                "text_tokens",
            ][..],
        ),
        (
            "output_tokens_details",
            "completion_tokens_details",
            &[
                "reasoning_tokens",
                "audio_tokens",
                "text_tokens",
                "accepted_prediction_tokens",
                "rejected_prediction_tokens",
            ][..],
        ),
    ] {
        if let Some(details) = usage.get(source).filter(|value| !value.is_null()) {
            let details = details.as_object().ok_or_else(invalid_response)?;
            let mut mapped = serde_json::Map::new();
            for field in fields {
                if let Some(count) = details.get(*field).filter(|value| !value.is_null()) {
                    let count = count.as_u64().ok_or_else(invalid_response)?;
                    mapped.insert((*field).to_owned(), json!(count));
                }
            }
            if !mapped.is_empty() {
                value[target] = Value::Object(mapped);
            }
        }
    }
    Ok(value)
}

fn content_key(value: &Value) -> Result<(u64, u64), ProtocolErrorBody> {
    Ok((
        required_index(value, "output_index")?,
        required_index(value, "content_index")?,
    ))
}

fn required_index(value: &Value, key: &str) -> Result<u64, ProtocolErrorBody> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(invalid_response)
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, ProtocolErrorBody> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(invalid_response)
}

fn frame(value: &Value) -> Bytes {
    Bytes::from(format!("data: {value}\n\n"))
}

fn invalid_response() -> ProtocolErrorBody {
    ProtocolErrorBody {
        error: ProtocolError {
            kind: "server_error",
            code: "invalid_upstream_response",
            message: "The gateway could not convert the upstream response to Chat Completions."
                .to_owned(),
            param: None,
        },
    }
}
