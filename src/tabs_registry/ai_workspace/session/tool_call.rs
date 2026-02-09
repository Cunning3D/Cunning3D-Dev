//! Tool call extraction with strong validation.
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub tool_name: String,
    pub args: Value,
}

#[derive(Clone, Debug)]
pub enum ToolCallParseError {
    NoToolName,
    InvalidArgs(String),
    MalformedJson(String),
}

fn validate_tool_call(tool_name: &str, args: &Value) -> Result<(), ToolCallParseError> {
    if tool_name.is_empty() {
        return Err(ToolCallParseError::NoToolName);
    }
    if tool_name.contains(char::is_whitespace) {
        return Err(ToolCallParseError::NoToolName);
    }
    if !args.is_object() && !args.is_null() {
        return Err(ToolCallParseError::InvalidArgs(
            "args must be object or null".into(),
        ));
    }
    Ok(())
}

fn from_value(v: &Value) -> Result<ToolCall, ToolCallParseError> {
    let o = v
        .as_object()
        .ok_or(ToolCallParseError::MalformedJson("expected object".into()))?;
    let tool_name = o
        .get("tool_name")
        .and_then(|x| x.as_str())
        .or_else(|| o.get("tool").and_then(|x| x.as_str()))
        .or_else(|| o.get("name").and_then(|x| x.as_str()))
        .ok_or(ToolCallParseError::NoToolName)?;
    let args = o
        .get("args")
        .or_else(|| o.get("arguments"))
        .or_else(|| o.get("parameters"))
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    validate_tool_call(tool_name, &args)?;
    Ok(ToolCall {
        tool_name: tool_name.to_string(),
        args,
    })
}

fn try_parse_json_strict(s: &str) -> Result<Vec<ToolCall>, ToolCallParseError> {
    let v: Value =
        serde_json::from_str(s).map_err(|e| ToolCallParseError::MalformedJson(e.to_string()))?;
    if let Ok(tc) = from_value(&v) {
        return Ok(vec![tc]);
    }
    if let Some(arr) = v.as_array() {
        let calls: Result<Vec<_>, _> = arr.iter().map(from_value).collect();
        return calls;
    }
    Err(ToolCallParseError::MalformedJson(
        "not a tool call object or array".into(),
    ))
}

fn extract_fenced_blocks<'a>(s: &'a str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(a) = s[i..].find("```") {
        let a = i + a + 3;
        let Some(nl) = s[a..].find('\n') else {
            break;
        };
        let lang = s[a..a + nl].trim().to_ascii_lowercase();
        let b = a + nl + 1;
        let Some(c) = s[b..].find("```") else {
            break;
        };
        if lang.is_empty() || lang == "json" || lang == "tool" {
            out.push(&s[b..b + c]);
        }
        i = b + c + 3;
    }
    out
}

fn extract_json_objects(s: &str, limit: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < s.len() && out.len() < limit {
        let Some(start) = s[i..].find('{') else {
            break;
        };
        let mut j = i + start;
        let (mut depth, mut in_str, mut esc) = (0i32, false, false);
        let bytes = s.as_bytes();
        while j < s.len() {
            let ch = bytes[j] as char;
            if in_str {
                if esc {
                    esc = false;
                } else if ch == '\\' {
                    esc = true;
                } else if ch == '"' {
                    in_str = false;
                }
            } else {
                match ch {
                    '"' => in_str = true,
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            out.push(&s[i + start..=j]);
                            j += 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            j += 1;
        }
        i = j.max(i + start + 1);
    }
    out
}

/// Extract tool calls with validation. Returns (calls, errors) for retry feedback.
pub fn extract_tool_calls_validated(
    s: &str,
    max_calls: usize,
) -> (Vec<ToolCall>, Vec<ToolCallParseError>) {
    if max_calls == 0 {
        return (Vec::new(), Vec::new());
    }
    let st = s.trim();
    let mut errors = Vec::new();
    // Try strict JSON first
    match try_parse_json_strict(st) {
        Ok(v) => return (v.into_iter().take(max_calls).collect(), Vec::new()),
        Err(e) => errors.push(e),
    }
    // Try fenced blocks
    for blk in extract_fenced_blocks(st) {
        match try_parse_json_strict(blk.trim()) {
            Ok(v) => return (v.into_iter().take(max_calls).collect(), Vec::new()),
            Err(e) => errors.push(e),
        }
    }
    // Try embedded JSON objects
    for obj in extract_json_objects(st, max_calls * 4) {
        match try_parse_json_strict(obj) {
            Ok(mut v) => {
                v.truncate(max_calls);
                return (v, Vec::new());
            }
            Err(e) => errors.push(e),
        }
    }
    (Vec::new(), errors)
}

/// Legacy API for compatibility
pub fn extract_tool_calls(s: &str, max_calls: usize) -> Vec<ToolCall> {
    extract_tool_calls_validated(s, max_calls).0
}

/// Format parse errors for model feedback (to trigger retry)
pub fn format_parse_errors(errors: &[ToolCallParseError]) -> String {
    if errors.is_empty() {
        return String::new();
    }
    let mut out = String::from("[Tool Call Parse Errors]\n");
    for (i, e) in errors.iter().take(3).enumerate() {
        out.push_str(&format!("{}. {:?}\n", i + 1, e));
    }
    out.push_str("\nPlease respond with a valid JSON tool call:\n```json\n{\"tool_name\": \"...\", \"args\": {...}}\n```");
    out
}
