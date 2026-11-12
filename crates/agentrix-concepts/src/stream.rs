//! Zavora CLI's `-o stream-json` output: one JSON object per line on stdout
//! (`schema_version` `zavora.headless.v1`, with `sequence`, `timestamp`, `type`, `command`,
//! `session_id` and `data`). Shapes, from W0's run of the real build on 8 Nov:
//!
//! | `type` | `data` |
//! |---|---|
//! | `init` | `{"capabilities": [...]}` |
//! | `agent` | `{"name": "assistant"}` |
//! | `message` | `{"role": "assistant", "author": "...", "delta": "<text chunk>"}` |
//! | `tool_use` | `{"call_id", "name": "fs_read", "detail": "<args as JSON text>"}` |
//! | `tool_result` | `{"call_id", "name", "success": false, "detail": "<text>"}` |
//! | `system` | `{"message": "..."}` |
//! | `error` | `{"fatal": false, "message": "..."}` or `{"fatal": true, "error": {"message": ...}}` |
//! | `result` | `{"success": true, "response": "<full text>", "model", "stats": {"duration_ms", ...}}` |
//!
//! Parsing is lenient: a line that isn't JSON is skipped, and a JSON line without a known `type`
//! still gives up its text from `text`, `delta` or `content`.

use serde_json::Value;

/// One thing an agent did, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// The run started.
    Started,
    /// An agent took the turn.
    Agent(String),
    /// A chunk of the agent's answer.
    Text(String),
    /// The agent used a tool (`fs_read` for the reference files).
    Tool {
        /// The tool's name.
        name: String,
    },
    /// A tool finished.
    ToolDone {
        /// The tool's name.
        name: String,
        /// Whether it worked.
        success: bool,
    },
    /// A note from the runtime.
    Note(String),
    /// Something went wrong.
    Problem {
        /// Whether the run stopped.
        fatal: bool,
        /// What the runtime said.
        message: String,
    },
    /// The run finished with this full answer.
    Finished {
        /// The full final text.
        response: String,
        /// How long the run took, by Zavora's clock.
        duration_ms: Option<u64>,
        /// The model that answered.
        model: Option<String>,
        /// The provider that answered (`vultr`).
        provider: Option<String>,
    },
}

fn text_of(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .map(str::to_owned)
}

/// Parses one stdout line; `None` for anything that isn't a JSON object worth showing.
pub fn parse_line(line: &str) -> Option<StreamEvent> {
    let line = line.trim();
    if !line.starts_with('{') {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let data = v.get("data").cloned().unwrap_or(Value::Null);
    let kind = v.get("type").and_then(Value::as_str).unwrap_or_default();
    Some(match kind {
        "init" => StreamEvent::Started,
        "agent" => StreamEvent::Agent(text_of(&data, &["name"]).unwrap_or_default()),
        "message" => StreamEvent::Text(text_of(&data, &["delta", "text", "content"])?),
        "tool_use" => StreamEvent::Tool {
            name: text_of(&data, &["name"]).unwrap_or_else(|| "a tool".into()),
        },
        "tool_result" => StreamEvent::ToolDone {
            name: text_of(&data, &["name"]).unwrap_or_default(),
            success: data
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        "system" => StreamEvent::Note(text_of(&data, &["message"]).unwrap_or_default()),
        "error" => {
            let message = text_of(&data, &["message"])
                .or_else(|| data.get("error").and_then(|e| text_of(e, &["message"])))
                .or_else(|| v.get("error").and_then(|e| text_of(e, &["message"])))
                .unwrap_or_else(|| "The agent stopped with an error.".into());
            let fatal = data
                .get("fatal")
                .and_then(Value::as_bool)
                .unwrap_or_else(|| v.get("success") == Some(&Value::Bool(false)));
            StreamEvent::Problem { fatal, message }
        }
        "result" => {
            let body = if data.is_null() { &v } else { &data };
            StreamEvent::Finished {
                response: text_of(body, &["response", "text", "content"]).unwrap_or_default(),
                duration_ms: body
                    .get("stats")
                    .and_then(|s| s.get("duration_ms"))
                    .and_then(Value::as_u64),
                model: text_of(body, &["model"]),
                provider: text_of(body, &["provider"]),
            }
        }
        _ => {
            let delta = text_of(&data, &["delta", "text", "content"])
                .or_else(|| text_of(&v, &["delta", "text", "content"]))?;
            StreamEvent::Text(delta)
        }
    })
}

/// The HTML document in an agent's answer: from `<!doctype html` (or `<html`) to the last
/// `</html>`, or to the end when the page is still being written. `None` when there's none.
pub fn extract_html(text: &str) -> Option<&str> {
    let lower = text.to_ascii_lowercase();
    let start = lower
        .find("<!doctype html")
        .or_else(|| lower.find("<html"))?;
    let end = lower
        .rfind("</html")
        .filter(|e| *e > start)
        .map_or(text.len(), |e| {
            lower[e..].find('>').map_or(text.len(), |g| e + g + 1)
        });
    Some(&text[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_real_shapes_parse() {
        let lines = [
            r#"{"schema_version":"zavora.headless.v1","sequence":0,"timestamp":"t","type":"init","command":"ask","session_id":"s","data":{"capabilities":["message"]}}"#,
            r#"{"schema_version":"zavora.headless.v1","sequence":1,"type":"agent","data":{"name":"assistant"}}"#,
            r#"{"schema_version":"zavora.headless.v1","sequence":2,"type":"message","data":{"role":"assistant","author":"assistant","delta":"<!doctype html>"}}"#,
            r#"{"schema_version":"zavora.headless.v1","sequence":3,"type":"tool_use","data":{"call_id":"c1","name":"fs_read","detail":"{\"path\":\"kit.css\"}"}}"#,
            r#"{"schema_version":"zavora.headless.v1","sequence":4,"type":"tool_result","data":{"call_id":"c1","name":"execute_bash","success":false,"detail":"requires approval in headless mode"}}"#,
            r#"{"schema_version":"zavora.headless.v1","sequence":5,"type":"result","data":{"success":true,"response":"<!doctype html><html></html>","provider":"vultr","model":"deepseek-v4.1-flash-1017","stats":{"duration_ms":81234,"response_chars":28,"tool_calls":1}}}"#,
        ];
        let events: Vec<_> = lines.iter().filter_map(|l| parse_line(l)).collect();
        assert_eq!(
            events,
            vec![
                StreamEvent::Started,
                StreamEvent::Agent("assistant".into()),
                StreamEvent::Text("<!doctype html>".into()),
                StreamEvent::Tool {
                    name: "fs_read".into()
                },
                StreamEvent::ToolDone {
                    name: "execute_bash".into(),
                    success: false
                },
                StreamEvent::Finished {
                    response: "<!doctype html><html></html>".into(),
                    duration_ms: Some(81234),
                    model: Some("deepseek-v4.1-flash-1017".into()),
                    provider: Some("vultr".into()),
                },
            ]
        );
    }

    #[test]
    fn errors_and_odd_lines_are_lenient() {
        assert_eq!(parse_line("\u{1b}[2K progress"), None);
        assert_eq!(parse_line("not json {"), None);
        assert_eq!(
            parse_line(
                r#"{"type":"error","data":{"fatal":true,"success":false,"error":{"message":"no key"}}}"#
            ),
            Some(StreamEvent::Problem {
                fatal: true,
                message: "no key".into()
            })
        );
        assert_eq!(
            parse_line(r#"{"text":"hello"}"#),
            Some(StreamEvent::Text("hello".into()))
        );
        assert_eq!(
            parse_line(r#"{"type":"content_block_delta","delta":"x"}"#),
            Some(StreamEvent::Text("x".into()))
        );
    }

    #[test]
    fn the_page_is_cut_from_the_answer() {
        let answer =
            "Here it is:\n```html\n<!DOCTYPE html><html><body>Hi</body></html>\n```\nDone.";
        assert_eq!(
            extract_html(answer),
            Some("<!DOCTYPE html><html><body>Hi</body></html>")
        );
        assert_eq!(
            extract_html("<html><body>still writ"),
            Some("<html><body>still writ")
        );
        assert_eq!(extract_html("no page here"), None);
    }
}
