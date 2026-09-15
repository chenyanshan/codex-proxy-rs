//! Chat Completions 请求转换；只生成已实现的 Responses 字段，不透传未知 Chat 语义。

use axum::http::HeaderMap;
use serde_json::{Map, Value, json};

use super::super::responses::{
    DecodedResponsesRequest, OpenAiRequestHeaders, RequestDecodeError, RequestDecodeSource,
    decode_request_object, decompress_request_body,
};

pub(super) struct DecodedChatRequest {
    pub(super) responses: DecodedResponsesRequest,
    pub(super) include_usage: bool,
}

pub(super) fn decode_request_with_headers(
    body: &[u8],
    headers: &HeaderMap,
) -> Result<DecodedChatRequest, RequestDecodeError> {
    let body = decompress_request_body(body, headers)?;
    let value: Value =
        serde_json::from_slice(&body).map_err(|_| RequestDecodeError::MalformedJson)?;
    let Value::Object(mut chat) = value else {
        return Err(RequestDecodeError::ExpectedObject);
    };
    let mut responses = Map::new();
    responses.insert("model".into(), take_required(&mut chat, "model")?);
    let stream = take_bool(&mut chat, "stream")?.unwrap_or(false);
    responses.insert("stream".into(), stream.into());
    responses.insert("store".into(), false.into());
    let include_usage = stream_options(chat.remove("stream_options"), stream)?;
    responses.insert(
        "input".into(),
        messages(take_required(&mut chat, "messages")?)?,
    );
    if let Some(tools) = take_optional(&mut chat, "tools") {
        responses.insert("tools".into(), function_tools(tools)?);
    }
    if let Some(choice) = take_optional(&mut chat, "tool_choice") {
        responses.insert(
            "tool_choice".into(),
            tool_choice(choice, responses.get("tools"))?,
        );
    }
    if let Some(parallel) = take_bool(&mut chat, "parallel_tool_calls")? {
        responses.insert("parallel_tool_calls".into(), parallel.into());
    }
    if let Some(format) = take_optional(&mut chat, "response_format") {
        responses.insert("text".into(), json!({"format": response_format(format)?}));
    }
    if let Some(verbosity) = take_optional(&mut chat, "verbosity") {
        if !matches!(verbosity.as_str(), Some("low" | "medium" | "high")) {
            return Err(invalid("verbosity"));
        }
        responses.entry("text").or_insert_with(|| json!({}))["verbosity"] = verbosity;
    }
    if let Some(top_p) = take_optional(&mut chat, "top_p") {
        if !top_p
            .as_f64()
            .is_some_and(|value| (0.0..=1.0).contains(&value))
        {
            return Err(invalid("top_p"));
        }
        responses.insert("top_p".into(), top_p);
    }
    validate_remaining(chat)?;
    let responses = decode_request_object(
        responses,
        &OpenAiRequestHeaders::from_headers(headers),
        RequestDecodeSource::Http,
    )?;
    Ok(DecodedChatRequest {
        responses,
        include_usage,
    })
}

fn stream_options(value: Option<Value>, stream: bool) -> Result<bool, RequestDecodeError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(false);
    };
    let mut options = object(value, "stream_options")?;
    let include_usage = take_bool_at(
        &mut options,
        "include_usage",
        "stream_options.include_usage",
    )?
    .unwrap_or(false);
    if take_bool_at(
        &mut options,
        "include_obfuscation",
        "stream_options.include_obfuscation",
    )? == Some(true)
    {
        return Err(unsupported("stream_options.include_obfuscation"));
    }
    reject_remaining(&options, "stream_options")?;
    if !stream {
        return Err(invalid("stream_options"));
    }
    Ok(include_usage)
}

fn validate_remaining(chat: Map<String, Value>) -> Result<(), RequestDecodeError> {
    for (field, value) in chat {
        let neutral = match field.as_str() {
            "n" => value.is_null() || value.as_u64() == Some(1),
            "store" | "logprobs" => value.is_null() || value == false,
            "frequency_penalty" | "presence_penalty" => {
                value.is_null() || value.as_f64() == Some(0.0)
            }
            "logit_bias" | "metadata" => {
                value.is_null() || value.as_object().is_some_and(Map::is_empty)
            }
            "modalities" => value.is_null() || value == json!(["text"]),
            "service_tier" => value.is_null() || value == "auto",
            "temperature"
            | "max_tokens"
            | "max_completion_tokens"
            | "reasoning_effort"
            | "stop"
            | "seed"
            | "audio"
            | "prediction"
            | "functions"
            | "function_call"
            | "top_logprobs"
            | "web_search_options"
            | "safety_identifier"
            | "prompt_cache_key"
            | "prompt_cache_retention"
            | "prompt_cache_options"
            | "user"
            | "verbosity"
            | "moderation" => value.is_null(),
            _ => return Err(RequestDecodeError::UnknownField { field }),
        };
        if !neutral {
            return Err(unsupported(&field));
        }
    }
    Ok(())
}

fn messages(value: Value) -> Result<Value, RequestDecodeError> {
    let Value::Array(messages) = value else {
        return Err(invalid("messages"));
    };
    if messages.is_empty() {
        return Err(RequestDecodeError::EmptyField {
            field: "messages".into(),
        });
    }
    let mut input = Vec::new();
    for (index, message) in messages.into_iter().enumerate() {
        let path = format!("messages[{index}]");
        let mut message = object(message, &path)?;
        let role = take_string(&mut message, "role", &path, false)?;
        for field in ["name", "audio", "function_call", "refusal"] {
            if take_optional(&mut message, field).is_some() {
                return Err(unsupported(&format!("{path}.{field}")));
            }
        }
        if role == "tool" {
            let call_id = take_string(&mut message, "tool_call_id", &path, false)?;
            let output = content(
                take_required_at(&mut message, "content", &format!("{path}.content"))?,
                "tool",
                &format!("{path}.content"),
            )?;
            input.push(json!({"type":"function_call_output","call_id":call_id,"output":output}));
        } else if matches!(role.as_str(), "system" | "developer" | "user" | "assistant") {
            let text = take_optional(&mut message, "content");
            let tool_calls = take_optional(&mut message, "tool_calls");
            if role != "assistant" && tool_calls.is_some() {
                return Err(invalid(&path));
            }
            if text.is_none() && tool_calls.is_none() {
                return Err(invalid(&format!("{path}.content")));
            }
            if let Some(text) = text {
                input.push(json!({"role":role,"content":content(text, &role, &format!("{path}.content"))?}));
            }
            if let Some(tool_calls) = tool_calls {
                let Value::Array(tool_calls) = tool_calls else {
                    return Err(invalid(&format!("{path}.tool_calls")));
                };
                for (index, call) in tool_calls.into_iter().enumerate() {
                    let call_path = format!("{path}.tool_calls[{index}]");
                    let mut call = object(call, &call_path)?;
                    let id = take_string(&mut call, "id", &call_path, false)?;
                    if take_string(&mut call, "type", &call_path, false)? != "function" {
                        return Err(unsupported(&format!("{call_path}.type")));
                    }
                    let function_path = format!("{call_path}.function");
                    let mut function = object(
                        take_required_at(&mut call, "function", &function_path)?,
                        &function_path,
                    )?;
                    let name = take_string(&mut function, "name", &function_path, false)?;
                    let arguments = take_string(&mut function, "arguments", &function_path, true)?;
                    reject_remaining(&function, &function_path)?;
                    reject_remaining(&call, &call_path)?;
                    input.push(json!({"type":"function_call","call_id":id,"name":name,"arguments":arguments}));
                }
            }
        } else {
            return Err(unsupported(&format!("{path}.role")));
        }
        reject_remaining(&message, &path)?;
    }
    Ok(input.into())
}

fn content(value: Value, role: &str, path: &str) -> Result<Value, RequestDecodeError> {
    if value.is_string() {
        return Ok(value);
    }
    let Value::Array(parts) = value else {
        return Err(invalid(path));
    };
    let mut result = Vec::with_capacity(parts.len());
    for (index, part) in parts.into_iter().enumerate() {
        let path = format!("{path}[{index}]");
        let mut part = object(part, &path)?;
        let kind = take_string(&mut part, "type", &path, false)?;
        let converted = match kind.as_str() {
            "text" => {
                let text = take_string(&mut part, "text", &path, true)?;
                // EasyInputMessage 的 assistant 历史同样使用 input_text；output_text 需要完整输出消息身份。
                json!({"type":"input_text","text":text})
            }
            "image_url" if role == "user" => {
                let image_path = format!("{path}.image_url");
                let mut image = object(
                    take_required_at(&mut part, "image_url", &image_path)?,
                    &image_path,
                )?;
                let url = take_string(&mut image, "url", &image_path, false)?;
                if !url::Url::parse(&url).is_ok_and(|parsed| {
                    matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some()
                        || parsed.scheme() == "data"
                            && url.starts_with("data:image/")
                            && url.contains(";base64,")
                }) {
                    return Err(invalid(&format!("{image_path}.url")));
                }
                let mut converted = json!({"type":"input_image","image_url":url});
                if let Some(detail) = take_optional(&mut image, "detail") {
                    if !matches!(detail.as_str(), Some("auto" | "low" | "high")) {
                        return Err(invalid(&format!("{image_path}.detail")));
                    }
                    converted["detail"] = detail;
                }
                reject_remaining(&image, &image_path)?;
                converted
            }
            _ => return Err(unsupported(&format!("{path}.type"))),
        };
        reject_remaining(&part, &path)?;
        result.push(converted);
    }
    Ok(result.into())
}

fn function_tools(value: Value) -> Result<Value, RequestDecodeError> {
    let Value::Array(tools) = value else {
        return Err(invalid("tools"));
    };
    let mut result: Vec<Value> = Vec::with_capacity(tools.len());
    for (index, tool) in tools.into_iter().enumerate() {
        let path = format!("tools[{index}]");
        let mut tool = object(tool, &path)?;
        if take_string(&mut tool, "type", &path, false)? != "function" {
            return Err(unsupported(&format!("{path}.type")));
        }
        let function_path = format!("{path}.function");
        let mut function = object(
            take_required_at(&mut tool, "function", &function_path)?,
            &function_path,
        )?;
        let name = take_string(&mut function, "name", &function_path, false)?;
        // Chat 函数工具默认非 strict，不能让 Responses 的默认规范化收紧调用合同。
        let mut converted = Map::from_iter([
            ("type".into(), "function".into()),
            ("name".into(), name.into()),
            ("strict".into(), false.into()),
        ]);
        for field in ["description", "parameters", "strict"] {
            if let Some(value) = take_optional(&mut function, field) {
                let valid = match field {
                    "description" => value.is_string(),
                    "parameters" => value.is_object(),
                    _ => value.is_boolean(),
                };
                if !valid {
                    return Err(invalid(&format!("{function_path}.{field}")));
                }
                converted.insert(field.into(), value);
            }
        }
        reject_remaining(&function, &function_path)?;
        reject_remaining(&tool, &path)?;
        result.push(converted.into());
    }
    Ok(result.into())
}

fn tool_choice(value: Value, tools: Option<&Value>) -> Result<Value, RequestDecodeError> {
    if value == "required"
        && !tools
            .and_then(Value::as_array)
            .is_some_and(|tools| !tools.is_empty())
    {
        return Err(invalid("tool_choice"));
    }
    if matches!(value.as_str(), Some("none" | "auto" | "required")) {
        return Ok(value);
    }
    let mut choice = object(value, "tool_choice")?;
    if take_string(&mut choice, "type", "tool_choice", false)? != "function" {
        return Err(unsupported("tool_choice.type"));
    }
    let mut function = object(
        take_required_at(&mut choice, "function", "tool_choice.function")?,
        "tool_choice.function",
    )?;
    let name = take_string(&mut function, "name", "tool_choice.function", false)?;
    if !tools
        .and_then(Value::as_array)
        .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == name))
    {
        return Err(invalid("tool_choice.function.name"));
    }
    reject_remaining(&function, "tool_choice.function")?;
    reject_remaining(&choice, "tool_choice")?;
    Ok(json!({"type":"function","name":name}))
}

fn response_format(value: Value) -> Result<Value, RequestDecodeError> {
    let mut format = object(value, "response_format")?;
    let kind = take_string(&mut format, "type", "response_format", false)?;
    let converted = match kind.as_str() {
        "text" | "json_object" => json!({"type":kind}),
        "json_schema" => {
            let path = "response_format.json_schema";
            let mut schema = object(take_required_at(&mut format, "json_schema", path)?, path)?;
            let name = take_string(&mut schema, "name", path, false)?;
            let definition =
                take_required_at(&mut schema, "schema", "response_format.json_schema.schema")?;
            if !definition.is_object() {
                return Err(invalid("response_format.json_schema.schema"));
            }
            let mut converted = json!({"type":"json_schema","name":name,"schema":definition});
            if let Some(description) = take_optional(&mut schema, "description") {
                if !description.is_string() {
                    return Err(invalid("response_format.json_schema.description"));
                }
                converted["description"] = description;
            }
            if let Some(strict) =
                take_bool_at(&mut schema, "strict", "response_format.json_schema.strict")?
            {
                converted["strict"] = strict.into();
            }
            reject_remaining(&schema, path)?;
            converted
        }
        _ => return Err(unsupported("response_format.type")),
    };
    reject_remaining(&format, "response_format")?;
    Ok(converted)
}

fn object(value: Value, path: &str) -> Result<Map<String, Value>, RequestDecodeError> {
    match value {
        Value::Object(object) => Ok(object),
        _ => Err(invalid(path)),
    }
}

fn take_required(
    object: &mut Map<String, Value>,
    field: &str,
) -> Result<Value, RequestDecodeError> {
    take_required_at(object, field, field)
}

fn take_required_at(
    object: &mut Map<String, Value>,
    field: &str,
    path: &str,
) -> Result<Value, RequestDecodeError> {
    object
        .remove(field)
        .ok_or_else(|| RequestDecodeError::MissingField { field: path.into() })
}

fn take_optional(object: &mut Map<String, Value>, field: &str) -> Option<Value> {
    object.remove(field).filter(|value| !value.is_null())
}

fn take_bool(
    object: &mut Map<String, Value>,
    field: &str,
) -> Result<Option<bool>, RequestDecodeError> {
    take_bool_at(object, field, field)
}

fn take_bool_at(
    object: &mut Map<String, Value>,
    field: &str,
    path: &str,
) -> Result<Option<bool>, RequestDecodeError> {
    take_optional(object, field)
        .map(|value| value.as_bool().ok_or_else(|| invalid(path)))
        .transpose()
}

fn take_string(
    object: &mut Map<String, Value>,
    field: &str,
    path: &str,
    allow_empty: bool,
) -> Result<String, RequestDecodeError> {
    let value = object
        .remove(field)
        .ok_or_else(|| RequestDecodeError::MissingField {
            field: format!("{path}.{field}"),
        })?;
    match value {
        Value::String(value) if allow_empty || !value.trim().is_empty() => Ok(value),
        _ => Err(invalid(&format!("{path}.{field}"))),
    }
}

fn reject_remaining(object: &Map<String, Value>, path: &str) -> Result<(), RequestDecodeError> {
    if let Some(field) = object.keys().next() {
        return Err(RequestDecodeError::UnknownField {
            field: format!("{path}.{field}"),
        });
    }
    Ok(())
}

fn invalid(field: &str) -> RequestDecodeError {
    RequestDecodeError::InvalidValue {
        field: field.into(),
    }
}
fn unsupported(field: &str) -> RequestDecodeError {
    RequestDecodeError::UnsupportedField {
        field: field.into(),
    }
}
