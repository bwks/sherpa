//! What one line of a Claude Code transcript says, with no I/O: an API response with its
//! tokens and the tool calls it made, tool results, or a prompt. Every other line means nothing
//! here.
//!
//! A response line counts when its `type` is `assistant` and it has `message.id`,
//! `message.usage` and a real model (`<synthetic>` is not one). Claude Code writes one line per
//! content block, each with the same `message.id` and usage: the importer keeps one row per id.

use anyhow::{Context, Result};
use jiff::Timestamp;
use serde_json::Value;

use crate::price::{STANDARD_SPEED, Tokens};
use crate::timing::{Class, classify};

/// The model of the lines Claude Code makes up itself, such as an API error.
const SYNTHETIC_MODEL: &str = "<synthetic>";
/// The `origin.kind` of prompts Claude Code wrote itself, not the user.
const NOT_USER_ORIGINS: &[&str] = &["task-notification"];
/// What a prompt Claude Code wrote for a background task starts with.
const NOTIFICATION_START: &str = "<task-notification>";

/// What a transcript line says.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// An API response, and the tool calls it started.
    Response(Response),
    /// The results of tool calls, by their IDs.
    Results {
        session: String,
        agent_id: Option<String>,
        at_ms: u64,
        ids: Vec<String>,
    },
    /// A prompt that starts a turn.
    Prompt {
        id: String,
        session: String,
        agent_id: Option<String>,
        at_ms: u64,
        from_user: bool,
    },
    /// Nothing the import keeps.
    Nothing,
}

/// One API response line.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub message_id: String,
    pub session: String,
    /// The subagent whose transcript it is in; `None` in the main transcript.
    pub agent_id: Option<String>,
    pub at_ms: u64,
    pub model: String,
    pub speed: String,
    pub tokens: Tokens,
    /// The tool calls the line starts, by ID.
    pub calls: Vec<(String, Class)>,
}

/// What `line` says, with `session` and `agent_id` the file's, taken when the line names none.
///
/// # Errors
///
/// Fails when the line is not JSON, or a line that should have a time has none that parses.
pub fn parse(line: &str, session: &str, agent_id: Option<&str>) -> Result<Entry> {
    let value: Value = serde_json::from_str(line).context("Not JSON")?;
    let kind = value["type"].as_str().unwrap_or_default();
    if kind != "assistant" && kind != "user" {
        return Ok(Entry::Nothing);
    }
    let session = value["sessionId"].as_str().unwrap_or(session).to_owned();
    let agent_id = value["agentId"]
        .as_str()
        .or(agent_id)
        .filter(|_| value["isSidechain"].as_bool().unwrap_or(agent_id.is_some()))
        .map(str::to_owned);
    let at_ms = || timestamp(&value);
    let content = &value["message"]["content"];
    if kind == "assistant" {
        let message = &value["message"];
        let (Some(id), Some(model), Some(usage)) = (
            message["id"].as_str(),
            message["model"].as_str(),
            message.get("usage").filter(|usage| usage.is_object()),
        ) else {
            return Ok(Entry::Nothing);
        };
        if model == SYNTHETIC_MODEL {
            return Ok(Entry::Nothing);
        }
        let calls = content
            .as_array()
            .into_iter()
            .flatten()
            .filter(|block| block["type"] == "tool_use")
            .filter_map(|block| {
                let id = block["id"].as_str()?;
                let name = block["name"].as_str().unwrap_or_default();
                Some((id.to_owned(), classify(name, &block["input"])))
            })
            .collect();
        return Ok(Entry::Response(Response {
            message_id: id.to_owned(),
            session,
            agent_id,
            at_ms: at_ms()?,
            model: model.to_owned(),
            speed: usage["speed"].as_str().unwrap_or(STANDARD_SPEED).to_owned(),
            tokens: tokens(usage),
            calls,
        }));
    }
    let results: Vec<String> = content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "tool_result")
        .filter_map(|block| block["tool_use_id"].as_str().map(str::to_owned))
        .collect();
    if !results.is_empty() {
        return Ok(Entry::Results {
            session,
            agent_id,
            at_ms: at_ms()?,
            ids: results,
        });
    }
    if value["isMeta"].as_bool().unwrap_or(false) {
        return Ok(Entry::Nothing);
    }
    let Some(id) = value["uuid"].as_str() else {
        return Ok(Entry::Nothing);
    };
    let origin = value["origin"]["kind"]
        .as_str()
        .or_else(|| value["origin"].as_str());
    let text = content.as_str().map_or_else(
        || {
            content
                .as_array()
                .into_iter()
                .flatten()
                .find_map(|block| block["text"].as_str())
                .unwrap_or_default()
        },
        |text| text,
    );
    let from_user = !origin.is_some_and(|origin| NOT_USER_ORIGINS.contains(&origin))
        && !text.trim_start().starts_with(NOTIFICATION_START);
    Ok(Entry::Prompt {
        id: id.to_owned(),
        session,
        agent_id,
        at_ms: at_ms()?,
        from_user,
    })
}

/// The tokens of a response's `usage`. The cache writes are split by `cache_creation`; without
/// the split, all of them are taken as 5-minute writes, Claude Code's default.
fn tokens(usage: &Value) -> Tokens {
    let count = |value: &Value| value.as_u64().unwrap_or_default();
    let written = count(&usage["cache_creation_input_tokens"]);
    let split = &usage["cache_creation"];
    let (five_minute, one_hour) = if split.is_object() {
        (
            count(&split["ephemeral_5m_input_tokens"]),
            count(&split["ephemeral_1h_input_tokens"]),
        )
    } else {
        (written, 0)
    };
    Tokens {
        input: count(&usage["input_tokens"]),
        output: count(&usage["output_tokens"]),
        cache_read: count(&usage["cache_read_input_tokens"]),
        cache_write_5m: five_minute,
        cache_write_1h: one_hour,
    }
}

/// The line's `timestamp`, in milliseconds since the Unix epoch.
fn timestamp(value: &Value) -> Result<u64> {
    let text = value["timestamp"]
        .as_str()
        .context("The line has no timestamp")?;
    let at: Timestamp = text
        .parse()
        .with_context(|| format!("The timestamp '{text}' is not one"))?;
    u64::try_from(at.as_millisecond()).context("The timestamp is before 1970")
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    const AT: &str = "2026-11-14T09:47:45.978Z";
    const AT_MS: u64 = 1_794_649_665_978;

    fn assistant(id: &str, model: &str, content: &Value) -> Value {
        json!({
            "type": "assistant",
            "sessionId": "s1",
            "isSidechain": false,
            "timestamp": AT,
            "uuid": "u1",
            "message": {
                "id": id,
                "model": model,
                "role": "assistant",
                "content": content,
                "usage": {
                    "input_tokens": 56,
                    "output_tokens": 18_340,
                    "cache_read_input_tokens": 1_476_182,
                    "cache_creation_input_tokens": 85_432,
                    "cache_creation": {
                        "ephemeral_5m_input_tokens": 0,
                        "ephemeral_1h_input_tokens": 85_432
                    },
                    "speed": "standard"
                }
            }
        })
    }

    #[test]
    fn a_response_line_gives_its_tokens_and_tool_calls() {
        let line = assistant(
            "msg_1",
            "claude-opus-5-5",
            &json!([
                { "type": "text", "text": "Testing." },
                { "type": "tool_use", "id": "toolu_1", "name": "Bash",
                  "input": { "command": "cargo test" } },
                { "type": "tool_use", "id": "toolu_2", "name": "Read",
                  "input": { "file_path": "/x" } }
            ]),
        );
        let entry = parse(&line.to_string(), "file-session", None).unwrap();
        assert_eq!(
            entry,
            Entry::Response(Response {
                message_id: "msg_1".to_owned(),
                session: "s1".to_owned(),
                agent_id: None,
                at_ms: AT_MS,
                model: "claude-opus-5-5".to_owned(),
                speed: "standard".to_owned(),
                tokens: Tokens {
                    input: 56,
                    output: 18_340,
                    cache_read: 1_476_182,
                    cache_write_5m: 0,
                    cache_write_1h: 85_432,
                },
                calls: vec![
                    ("toolu_1".to_owned(), Class::Testing),
                    ("toolu_2".to_owned(), Class::Work),
                ],
            })
        );
    }

    #[test]
    fn synthetic_lines_and_lines_without_usage_mean_nothing() {
        let synthetic = assistant("msg_2", "<synthetic>", &json!([]));
        assert_eq!(
            parse(&synthetic.to_string(), "s", None).unwrap(),
            Entry::Nothing
        );
        let mut no_usage = assistant("msg_3", "claude-opus-5-5", &json!([]));
        no_usage["message"].as_object_mut().unwrap().remove("usage");
        assert_eq!(
            parse(&no_usage.to_string(), "s", None).unwrap(),
            Entry::Nothing
        );
        let no_id = json!({ "type": "assistant", "timestamp": AT, "message": { "usage": {} } });
        assert_eq!(
            parse(&no_id.to_string(), "s", None).unwrap(),
            Entry::Nothing
        );
        let cost = json!({ "type": "cost-state", "totalCostUSD": 1.0 });
        assert_eq!(parse(&cost.to_string(), "s", None).unwrap(), Entry::Nothing);
    }

    #[test]
    fn a_garbage_line_is_an_error() {
        assert!(parse("{\"type\":\"assistant\"", "s", None).is_err());
        assert!(parse("not json", "s", None).is_err());
    }

    #[test]
    fn without_the_split_cache_writes_are_five_minute_writes() {
        let mut line = assistant("msg_4", "claude-sonnet-5-5", &json!([]));
        line["message"]["usage"]
            .as_object_mut()
            .unwrap()
            .remove("cache_creation");
        line["message"]["usage"]
            .as_object_mut()
            .unwrap()
            .remove("speed");
        let Entry::Response(response) = parse(&line.to_string(), "s", None).unwrap() else {
            panic!("not a response");
        };
        assert_eq!(response.tokens.cache_write_5m, 85_432);
        assert_eq!(response.tokens.cache_write_1h, 0);
        assert_eq!(response.speed, "standard");
    }

    #[test]
    fn tool_results_and_prompts_are_told_apart() {
        let results = json!({
            "type": "user", "sessionId": "s1", "timestamp": AT, "uuid": "u2",
            "message": { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "toolu_1", "content": "ok" }
            ] }
        });
        assert_eq!(
            parse(&results.to_string(), "s", None).unwrap(),
            Entry::Results {
                session: "s1".to_owned(),
                agent_id: None,
                at_ms: AT_MS,
                ids: vec!["toolu_1".to_owned()],
            }
        );
        let prompt = json!({
            "type": "user", "sessionId": "s1", "timestamp": AT, "uuid": "u3",
            "message": { "role": "user", "content": "Implement it" }
        });
        assert_eq!(
            parse(&prompt.to_string(), "s", None).unwrap(),
            Entry::Prompt {
                id: "u3".to_owned(),
                session: "s1".to_owned(),
                agent_id: None,
                at_ms: AT_MS,
                from_user: true,
            }
        );
        let notification = json!({
            "type": "user", "sessionId": "s1", "timestamp": AT, "uuid": "u4",
            "origin": { "kind": "task-notification" },
            "message": { "role": "user", "content": "<task-notification>done</task-notification>" }
        });
        assert!(matches!(
            parse(&notification.to_string(), "s", None).unwrap(),
            Entry::Prompt {
                from_user: false,
                ..
            }
        ));
        let meta = json!({
            "type": "user", "sessionId": "s1", "timestamp": AT, "uuid": "u5", "isMeta": true,
            "message": { "role": "user", "content": [{ "type": "text", "text": "skill" }] }
        });
        assert_eq!(parse(&meta.to_string(), "s", None).unwrap(), Entry::Nothing);
    }

    #[test]
    fn a_subagent_line_carries_the_parents_session_and_its_agent() {
        let mut line = assistant("msg_5", "claude-haiku-5-5", &json!([]));
        line["isSidechain"] = json!(true);
        line["agentId"] = json!("a1");
        let Entry::Response(response) = parse(&line.to_string(), "other", Some("a1")).unwrap()
        else {
            panic!("not a response");
        };
        assert_eq!(response.session, "s1");
        assert_eq!(response.agent_id.as_deref(), Some("a1"));
    }
}
