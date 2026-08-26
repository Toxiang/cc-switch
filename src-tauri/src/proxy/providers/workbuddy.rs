use crate::proxy::{
    sse::{append_utf8_safe, strip_sse_field, take_sse_block},
    ProxyError,
};
use bytes::Bytes;
use futures::{Stream, StreamExt};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub fn chat_request_to_responses(body: Value) -> Result<Value, ProxyError> {
    let source = body.as_object().ok_or_else(|| {
        ProxyError::TransformError("WorkBuddy Chat request must be a JSON object".to_string())
    })?;
    let mut result = Map::new();

    copy(source, &mut result, "model", "model");
    copy(source, &mut result, "stream", "stream");
    copy(source, &mut result, "temperature", "temperature");
    copy(source, &mut result, "top_p", "top_p");
    copy(
        source,
        &mut result,
        "parallel_tool_calls",
        "parallel_tool_calls",
    );
    copy(source, &mut result, "metadata", "metadata");
    copy(source, &mut result, "store", "store");

    if let Some(limit) = source
        .get("max_completion_tokens")
        .or_else(|| source.get("max_tokens"))
    {
        result.insert("max_output_tokens".to_string(), limit.clone());
    }
    if let Some(effort) = source.get("reasoning_effort") {
        result.insert("reasoning".to_string(), json!({"effort": effort}));
    }

    let mut input = Vec::new();
    let mut instructions = Vec::new();
    for message in source
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        append_chat_message(message, &mut input, &mut instructions)?;
    }
    if !instructions.is_empty() {
        result.insert(
            "instructions".to_string(),
            Value::String(instructions.join("\n\n")),
        );
    }
    result.insert("input".to_string(), Value::Array(input));

    if let Some(tools) = source.get("tools").and_then(Value::as_array) {
        let converted = tools
            .iter()
            .enumerate()
            .map(|(index, tool)| chat_tool_to_response_tool(tool, index))
            .collect::<Result<Vec<_>, _>>()?;
        if !converted.is_empty() {
            result.insert("tools".to_string(), Value::Array(converted));
        }
    }
    if let Some(choice) = source.get("tool_choice") {
        result.insert(
            "tool_choice".to_string(),
            chat_tool_choice_to_responses(choice)?,
        );
    }
    if let Some(format) = source.get("response_format") {
        if let Some(format) = chat_response_format_to_responses(format) {
            result.insert("text".to_string(), json!({"format": format}));
        }
    }

    if result.get("stream").and_then(Value::as_bool) == Some(true) {
        result.insert("stream_options".to_string(), json!({"include_usage": true}));
    }
    Ok(Value::Object(result))
}

fn copy(source: &Map<String, Value>, target: &mut Map<String, Value>, from: &str, to: &str) {
    if let Some(value) = source.get(from) {
        target.insert(to.to_string(), value.clone());
    }
}

fn append_chat_message(
    message: &Value,
    input: &mut Vec<Value>,
    instructions: &mut Vec<String>,
) -> Result<(), ProxyError> {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user");
    if matches!(role, "system" | "developer") {
        let text = chat_content_text(message.get("content"));
        if !text.is_empty() {
            instructions.push(text);
        }
        return Ok(());
    }

    if role == "tool" {
        input.push(json!({
            "type": "function_call_output",
            "call_id": message.get("tool_call_id").and_then(Value::as_str).unwrap_or(""),
            "output": chat_content_text(message.get("content"))
        }));
        return Ok(());
    }

    let content = chat_content_to_responses(message.get("content"), role);
    if !content.is_empty() {
        input.push(json!({"type":"message", "role": role, "content": content}));
    }

    if role == "assistant" {
        if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
            for call in tool_calls {
                let function = call.get("function").unwrap_or(&Value::Null);
                input.push(json!({
                    "type": "function_call",
                    "call_id": call.get("id").and_then(Value::as_str).unwrap_or(""),
                    "name": function.get("name").and_then(Value::as_str).unwrap_or(""),
                    "arguments": canonical_arguments(function.get("arguments"))
                }));
            }
        }
    }
    Ok(())
}

fn chat_content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(value) if !value.is_null() => value.to_string(),
        _ => String::new(),
    }
}

fn chat_content_to_responses(content: Option<&Value>, role: &str) -> Vec<Value> {
    let text_type = if role == "assistant" {
        "output_text"
    } else {
        "input_text"
    };
    match content {
        Some(Value::String(text)) if !text.is_empty() => {
            vec![json!({"type":text_type,"text":text})]
        }
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                Some("text") | Some("input_text") | Some("output_text") => part
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|text| json!({"type":text_type,"text":text})),
                Some("image_url") => part
                    .pointer("/image_url/url")
                    .and_then(Value::as_str)
                    .map(|url| json!({"type":"input_image","image_url":url})),
                Some("input_image") => Some(part.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn canonical_arguments(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(value) => serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()),
        None => "{}".to_string(),
    }
}

fn chat_tool_to_response_tool(tool: &Value, index: usize) -> Result<Value, ProxyError> {
    // WorkBuddy versions/plugins have emitted all of these shapes:
    //   Chat:      {"type":"function","function":{"name":"...","parameters":{}}}
    //   Responses: {"type":"function","name":"...","parameters":{}}
    //   Anthropic: {"name":"...","input_schema":{}}
    // Prefer the nested Chat function when present, while falling back to the
    // top-level fields for hybrid payloads.
    let function = tool
        .get("function")
        .filter(|value| value.is_object())
        .unwrap_or(tool);
    let name = function
        .get("name")
        .or_else(|| tool.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            ProxyError::TransformError(format!(
                "WorkBuddy tool at index {index} is missing a non-empty function name ({})",
                tool_shape(tool)
            ))
        })?;

    let parameters = function
        .get("parameters")
        .or_else(|| function.get("input_schema"))
        .or_else(|| tool.get("parameters"))
        .or_else(|| tool.get("input_schema"))
        .cloned()
        .unwrap_or_else(|| json!({"type":"object","properties":{}}));
    let mut converted = Map::new();
    converted.insert("type".to_string(), Value::String("function".to_string()));
    converted.insert("name".to_string(), Value::String(name.to_string()));
    if let Some(description) = function
        .get("description")
        .or_else(|| tool.get("description"))
        .filter(|value| !value.is_null())
    {
        converted.insert("description".to_string(), description.clone());
    }
    converted.insert("parameters".to_string(), parameters);
    converted.insert(
        "strict".to_string(),
        function
            .get("strict")
            .or_else(|| tool.get("strict"))
            .cloned()
            .unwrap_or(Value::Bool(false)),
    );
    Ok(Value::Object(converted))
}

fn chat_tool_choice_to_responses(choice: &Value) -> Result<Value, ProxyError> {
    let function = choice
        .get("function")
        .filter(|value| value.is_object())
        .unwrap_or(choice);
    if choice.get("function").is_some()
        || choice.get("name").is_some()
        || choice.get("type").and_then(Value::as_str) == Some("function")
    {
        let name = function
            .get("name")
            .or_else(|| choice.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                ProxyError::TransformError(format!(
                    "WorkBuddy tool_choice is missing a non-empty function name ({})",
                    tool_shape(choice)
                ))
            })?;
        return Ok(json!({"type":"function", "name":name}));
    }
    Ok(match choice.as_str() {
        Some("required") => json!("required"),
        Some(value) => json!(value),
        None => choice.clone(),
    })
}

fn tool_shape(value: &Value) -> String {
    let top_level_keys = value
        .as_object()
        .map(|object| object.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let function_keys = value
        .get("function")
        .and_then(Value::as_object)
        .map(|object| object.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    format!(
        "top-level keys=[{}], function keys=[{}]",
        top_level_keys.join(","),
        function_keys.join(",")
    )
}

fn chat_response_format_to_responses(format: &Value) -> Option<Value> {
    match format.get("type").and_then(Value::as_str) {
        Some("json_schema") => {
            let schema = format.get("json_schema")?;
            Some(json!({
                "type":"json_schema",
                "name":schema.get("name").and_then(Value::as_str).unwrap_or("response"),
                "schema":schema.get("schema").cloned().unwrap_or_else(|| json!({})),
                "strict":schema.get("strict").cloned().unwrap_or(Value::Bool(false))
            }))
        }
        Some("json_object") => Some(json!({"type":"json_object"})),
        Some("text") => Some(json!({"type":"text"})),
        _ => None,
    }
}

pub fn responses_to_chat_completion(body: Value) -> Result<Value, ProxyError> {
    if body.get("error").is_some() || body.get("status").and_then(Value::as_str) == Some("failed") {
        return Err(ProxyError::TransformError(
            body.pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Responses upstream failed")
                .to_string(),
        ));
    }
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("resp_unknown");
    let model = body.get("model").and_then(Value::as_str).unwrap_or("");
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    for item in body
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => for part in item.get("content").and_then(Value::as_array).into_iter().flatten() {
                match part.get("type").and_then(Value::as_str) {
                    Some("output_text") => text.push_str(part.get("text").and_then(Value::as_str).unwrap_or("")),
                    Some("refusal") => text.push_str(part.get("refusal").and_then(Value::as_str).unwrap_or("")),
                    _ => {}
                }
            },
            Some("reasoning") => {
                for part in item.get("summary").and_then(Value::as_array).into_iter().flatten() {
                    reasoning.push_str(part.get("text").and_then(Value::as_str).unwrap_or(""));
                }
            }
            Some("function_call") => tool_calls.push(json!({
                "id": item.get("call_id").or_else(|| item.get("id")).and_then(Value::as_str).unwrap_or(""),
                "type":"function",
                "function":{
                    "name":item.get("name").and_then(Value::as_str).unwrap_or(""),
                    "arguments":canonical_arguments(item.get("arguments"))
                }
            })),
            _ => {}
        }
    }
    let mut message = json!({"role":"assistant","content": if text.is_empty() { Value::Null } else { Value::String(text) }});
    if !reasoning.is_empty() {
        message["reasoning_content"] = Value::String(reasoning);
    }
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }
    let finish_reason = if message.get("tool_calls").is_some() {
        "tool_calls"
    } else if body.get("status").and_then(Value::as_str) == Some("incomplete") {
        "length"
    } else {
        "stop"
    };
    Ok(json!({
        "id": format!("chatcmpl-{}", id.trim_start_matches("resp_")),
        "object":"chat.completion",
        "created": body.get("created_at").cloned().unwrap_or_else(|| json!(chrono::Utc::now().timestamp())),
        "model":model,
        "choices":[{"index":0,"message":message,"finish_reason":finish_reason}],
        "usage": responses_usage_to_chat(body.get("usage"))
    }))
}

fn responses_usage_to_chat(usage: Option<&Value>) -> Value {
    let input = usage
        .and_then(|v| v.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .and_then(|v| v.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cached = usage
        .and_then(|v| v.pointer("/input_tokens_details/cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    json!({
        "prompt_tokens":input,
        "completion_tokens":output,
        "total_tokens":usage.and_then(|v| v.get("total_tokens")).and_then(Value::as_u64).unwrap_or(input + output),
        "prompt_tokens_details":{"cached_tokens":cached}
    })
}

#[derive(Default)]
struct ResponsesToChatStreamState {
    id: String,
    model: String,
    sent_role: bool,
    tool_indices: HashMap<String, usize>,
    next_tool_index: usize,
    has_tool_call: bool,
    done: bool,
}

impl ResponsesToChatStreamState {
    fn chunk(&self, delta: Value, finish_reason: Value, usage: Option<Value>) -> Bytes {
        let mut value = json!({
            "id": if self.id.is_empty() { "chatcmpl-workbuddy" } else { &self.id },
            "object":"chat.completion.chunk",
            "created":chrono::Utc::now().timestamp(),
            "model":self.model,
            "choices":[{"index":0,"delta":delta,"finish_reason":finish_reason}]
        });
        if let Some(usage) = usage {
            value["usage"] = usage;
        }
        Bytes::from(format!("data: {}\n\n", value))
    }

    fn handle(&mut self, event: Value) -> Vec<Bytes> {
        let mut out = Vec::new();
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        if self.id.is_empty() {
            let response = event.get("response").unwrap_or(&event);
            self.id = response
                .get("id")
                .and_then(Value::as_str)
                .map(|id| format!("chatcmpl-{}", id.trim_start_matches("resp_")))
                .unwrap_or_default();
            self.model = response
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
        }
        if !self.sent_role && !matches!(kind, "response.completed" | "response.failed" | "error") {
            self.sent_role = true;
            out.push(self.chunk(json!({"role":"assistant"}), Value::Null, None));
        }
        match kind {
            "response.output_text.delta" => out.push(self.chunk(json!({"content":event.get("delta").cloned().unwrap_or(Value::String(String::new()))}), Value::Null, None)),
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => out.push(self.chunk(json!({"reasoning_content":event.get("delta").cloned().unwrap_or(Value::String(String::new()))}), Value::Null, None)),
            "response.output_item.added" => {
                let item = event.get("item").unwrap_or(&Value::Null);
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    let key = item.get("id").or_else(|| item.get("call_id")).and_then(Value::as_str).unwrap_or("").to_string();
                    let index = self.next_tool_index;
                    self.next_tool_index += 1;
                    self.tool_indices.insert(key, index);
                    self.has_tool_call = true;
                    out.push(self.chunk(json!({"tool_calls":[{"index":index,"id":item.get("call_id").or_else(|| item.get("id")).cloned().unwrap_or(Value::String(String::new())),"type":"function","function":{"name":item.get("name").cloned().unwrap_or(Value::String(String::new())),"arguments":""}}]}), Value::Null, None));
                }
            }
            "response.function_call_arguments.delta" => {
                let key = event.get("item_id").or_else(|| event.get("call_id")).and_then(Value::as_str).unwrap_or("");
                let index = self.tool_indices.get(key).copied().unwrap_or(0);
                out.push(self.chunk(json!({"tool_calls":[{"index":index,"function":{"arguments":event.get("delta").cloned().unwrap_or(Value::String(String::new()))}}]}), Value::Null, None));
            }
            "response.completed" => {
                let response = event.get("response").unwrap_or(&event);
                let finish = if self.has_tool_call { "tool_calls" } else { "stop" };
                out.push(self.chunk(json!({}), json!(finish), Some(responses_usage_to_chat(response.get("usage")))));
                out.push(Bytes::from_static(b"data: [DONE]\n\n"));
                self.done = true;
            }
            "response.failed" | "error" => {
                let message = event.pointer("/response/error/message").or_else(|| event.pointer("/error/message")).or_else(|| event.get("message")).and_then(Value::as_str).unwrap_or("Responses upstream failed");
                out.push(Bytes::from(format!("data: {}\n\n", json!({"error":{"message":message,"type":"responses_error"}}))));
                out.push(Bytes::from_static(b"data: [DONE]\n\n"));
                self.done = true;
            }
            _ => {}
        }
        out
    }
}

pub fn responses_sse_to_chat_sse<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder = Vec::new();
        let mut state = ResponsesToChatStreamState::default();
        tokio::pin!(stream);
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes),
                Err(error) => {
                    yield Ok(Bytes::from(format!("data: {}\n\n", json!({"error":{"message":error.to_string(),"type":"stream_error"}}))));
                    yield Ok(Bytes::from_static(b"data: [DONE]\n\n"));
                    break;
                }
            }
            while let Some(block) = take_sse_block(&mut buffer) {
                let data = block.lines().filter_map(|line| strip_sse_field(line, "data")).collect::<Vec<_>>().join("\n");
                if data.trim().is_empty() || data.trim() == "[DONE]" { continue; }
                if let Ok(event) = serde_json::from_str::<Value>(&data) {
                    for converted in state.handle(event) { yield Ok(converted); }
                }
                if state.done { break; }
            }
            if state.done { break; }
        }
        if !state.done {
            yield Ok(Bytes::from(format!("data: {}\n\n", json!({"error":{"message":"Responses stream ended before response.completed","type":"stream_truncated"}}))));
            yield Ok(Bytes::from_static(b"data: [DONE]\n\n"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_request_maps_tools_reasoning_and_images() {
        let result = chat_request_to_responses(json!({
            "model":"gpt-5.6-sol","stream":true,"reasoning_effort":"high",
            "messages":[
                {"role":"system","content":"be concise"},
                {"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AA=="}}]},
                {"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"read","arguments":"{\"path\":\"a\"}"}}]},
                {"role":"tool","tool_call_id":"c1","content":"ok"}
            ],
            "tools":[{"type":"function","function":{"name":"read","parameters":{"type":"object"}}}]
        })).unwrap();
        assert_eq!(result["instructions"], "be concise");
        assert_eq!(result["reasoning"]["effort"], "high");
        assert_eq!(result["input"][0]["content"][1]["type"], "input_image");
        assert_eq!(result["input"][1]["type"], "function_call");
        assert_eq!(result["input"][2]["type"], "function_call_output");
        assert_eq!(result["tools"][0]["name"], "read");
        assert_eq!(result["stream_options"]["include_usage"], true);
    }

    #[test]
    fn chat_request_accepts_chat_responses_and_anthropic_tool_shapes() {
        let result = chat_request_to_responses(json!({
            "model":"gpt-5.6-sol",
            "messages":[{"role":"user","content":"test"}],
            "tools":[
                {"type":"function","function":{"name":"chat_tool","description":"chat","parameters":{"type":"object"}}},
                {"type":"function","name":"responses_tool","parameters":{"type":"object","properties":{"path":{"type":"string"}}}},
                {"name":"anthropic_tool","description":"anthropic","input_schema":{"type":"object","required":["query"]}},
                {"type":"function","function":{"name":"hybrid_tool","input_schema":{"type":"object","properties":{"id":{"type":"number"}}}}}
            ],
            "tool_choice":{"type":"function","name":"responses_tool"}
        }))
        .unwrap();

        assert_eq!(result["tools"][0]["name"], "chat_tool");
        assert_eq!(result["tools"][1]["name"], "responses_tool");
        assert_eq!(
            result["tools"][1]["parameters"]["properties"]["path"]["type"],
            "string"
        );
        assert_eq!(result["tools"][2]["name"], "anthropic_tool");
        assert_eq!(result["tools"][2]["parameters"]["required"][0], "query");
        assert_eq!(result["tools"][3]["name"], "hybrid_tool");
        assert_eq!(
            result["tools"][3]["parameters"]["properties"]["id"]["type"],
            "number"
        );
        assert_eq!(result["tool_choice"]["name"], "responses_tool");
    }

    #[test]
    fn chat_request_accepts_nested_chat_tool_choice() {
        let result = chat_request_to_responses(json!({
            "messages":[],
            "tool_choice":{"type":"function","function":{"name":"read"}}
        }))
        .unwrap();
        assert_eq!(
            result["tool_choice"],
            json!({"type":"function","name":"read"})
        );
    }

    #[test]
    fn chat_request_rejects_unnamed_tools_before_upstream() {
        let error = chat_request_to_responses(json!({
            "messages":[],
            "tools":[{"type":"function","function":{"parameters":{"type":"object"}}}]
        }))
        .unwrap_err();

        match error {
            ProxyError::TransformError(message) => {
                assert!(message.contains("tool at index 0"));
                assert!(message.contains("top-level keys"));
                assert!(!message.contains("properties"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn chat_request_rejects_unnamed_function_tool_choice() {
        let error = chat_request_to_responses(json!({
            "messages":[],
            "tool_choice":{"type":"function"}
        }))
        .unwrap_err();
        assert!(matches!(
            error,
            ProxyError::TransformError(message)
                if message.contains("tool_choice") && message.contains("function name")
        ));
    }

    #[test]
    fn responses_json_maps_text_reasoning_tools_and_usage() {
        let result = responses_to_chat_completion(json!({
            "id":"resp_1","status":"completed","model":"gpt-5.6-sol","output":[
                {"type":"reasoning","summary":[{"type":"summary_text","text":"think"}]},
                {"type":"message","content":[{"type":"output_text","text":"answer"}]},
                {"type":"function_call","call_id":"c1","name":"read","arguments":"{}"}
            ],"usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12,"input_tokens_details":{"cached_tokens":3}}
        })).unwrap();
        assert_eq!(result["choices"][0]["message"]["content"], "answer");
        assert_eq!(
            result["choices"][0]["message"]["reasoning_content"],
            "think"
        );
        assert_eq!(result["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(result["usage"]["prompt_tokens_details"]["cached_tokens"], 3);
    }
}
