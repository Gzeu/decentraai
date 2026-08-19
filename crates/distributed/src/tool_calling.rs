//! Real tool calling for the agent executor (fabric-wide, no runtime deps).
//!
//! The InferenceAgentExecutor runs a delegated LLM task and, when the node
//! attached local tool bindings (OCR, STT, HF skills), the model may emit a
//! tool-call block instead of a plain answer. This module owns the pure
//! decisions (prompt construction, tool-call parsing) plus the single HTTP
//! round-trip to the tool's local backend.
//!
//! Protocol: the model is instructed (in the prompt) to emit, when it needs a
//! tool, exactly one JSON object on its own line inside a fenced block:
//!
//! ```text
//! [TOOL_CALL]{"name":"sentiment","arguments":{"text":"..."}}[/TOOL_CALL]
//! ```
//!
//! The executor parses that, executes the tool over loopback HTTP, injects
//! the result into the prompt, and asks the model to continue (bounded
//! iterations — one tool call per round). A malformed block is treated as
//! plain text: the executor never fails a task because the model tried to
//! call a tool it does not know (it would only confuse the answer).

use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// A local tool the executor can call: name + description (fed to the model)
/// + the loopback URL the node's Tool Runtime exposes (`/v1/...`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBinding {
    /// Tool id sent to the model (`ocr`, `sentiment`, …).
    pub name: String,
    /// Human description shown to the model (what it does, input/output).
    pub description: String,
    /// Local backend URL to POST the tool-call arguments to.
    pub url: String,
}

impl ToolBinding {
    pub fn new(name: impl Into<String>, description: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            url: url.into(),
        }
    }
}

/// The tool-call the model requested (parsed from `[TOOL_CALL]…[/TOOL_CALL]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// Pure: builds the prompt for a task that may use the given tools.
///
/// When `tools` is empty the prompt is returned verbatim (no tool ceremony
/// ever reaches a model that has no tools). Otherwise a compact tool catalog
/// is prepended and the model is told how to request a call.
pub fn tool_prompt(tools: &[ToolBinding], user_prompt: &str) -> String {
    if tools.is_empty() {
        return user_prompt.to_string();
    }
    let catalog: String = tools
        .iter()
        .map(|t| {
            format!(
                "- {name}: {description} (arguments JSON object)\n",
                name = t.name,
                description = t.description
            )
        })
        .collect();
    format!(
        "You have access to these local tools:\n{catalog}\n\
         To use a tool, answer with EXACTLY one line:\n\
         [TOOL_CALL]{{\"name\":\"<tool>\",\"arguments\":{{...}}}}[/TOOL_CALL]\n\
         and nothing else. Otherwise answer normally.\n\n{user_prompt}"
    )
}

/// Pure: extracts a `[TOOL_CALL]…[/TOOL_CALL]` block from a model response.
///
/// Returns `Ok(Some(call))` when a well-formed block is present, `Ok(None)`
/// when the model answered plainly, and `Err` only for a block that cannot be
/// parsed as JSON (a model protocol violation — the caller decides whether to
/// surface it).
pub fn parse_tool_call(response: &str) -> Result<Option<ToolCall>> {
    let start = response.find("[TOOL_CALL]").map(|i| i + "[TOOL_CALL]".len());
    let Some(start) = start else {
        return Ok(None);
    };
    let rest = &response[start..];
    let end = rest.find("[/TOOL_CALL]").ok_or_else(|| {
        anyhow::anyhow!("unterminated [TOOL_CALL] block (opening found but no closing tag)")
    })?;
    let json = &rest[..end];
    let call: ToolCall = serde_json::from_str(json).with_context(|| {
        format!("tool call is not valid JSON: {json:?}")
    })?;
    Ok(Some(call))
}

/// Executes one tool call against the matching binding's loopback URL.
/// Returns the raw JSON body as a string (the caller injects it verbatim).
pub async fn execute_tool_call(
    client: &reqwest::Client,
    bindings: &[ToolBinding],
    call: &ToolCall,
) -> Result<String> {
    let binding = bindings
        .iter()
        .find(|b| b.name == call.name)
        .ok_or_else(|| anyhow::anyhow!("model called unknown tool '{}'", call.name))?;
    let args = if call.arguments.is_null() {
        serde_json::json!({})
    } else {
        call.arguments.clone()
    };
    let resp = client
        .post(&binding.url)
        .json(&args)
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .with_context(|| format!("calling tool '{}'", call.name))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .with_context(|| format!("reading tool '{}' response", call.name))?;
    if !status.is_success() {
        anyhow::bail!("tool '{}' returned {status}: {body}", call.name);
    }
    Ok(body)
}

/// Pure: wraps a tool result so the model sees it as tool output.
pub fn tool_result_block(tool: &str, result: &str) -> String {
    format!(
        "[TOOL_RESULT tool=\"{tool}\"]\n{result}\n[/TOOL_RESULT]\n\nNow answer the user's original question using that tool result."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tools_leave_prompt_untouched() {
        let prompt = tool_prompt(&[], "hello");
        assert_eq!(prompt, "hello");
    }

    #[test]
    fn tools_are_cataloged_into_the_prompt() {
        let tools = vec![ToolBinding::new(
            "ocr",
            "extracts text from an image (input: image_b64)",
            "http://127.0.0.1:9/v1/ocr",
        )];
        let prompt = tool_prompt(&tools, "read this image");
        assert!(prompt.contains("ocr"));
        assert!(prompt.contains("[TOOL_CALL]"));
        assert!(prompt.ends_with("read this image"));
    }

    #[test]
    fn plain_answer_has_no_tool_call() {
        assert!(parse_tool_call("I can see the text: it says hello.").unwrap().is_none());
    }

    #[test]
    fn well_formed_tool_call_parses() {
        let raw = "[TOOL_CALL]{\"name\":\"sentiment\",\"arguments\":{\"text\":\"wow\"}}[/TOOL_CALL]";
        let call = parse_tool_call(raw).unwrap().unwrap();
        assert_eq!(call.name, "sentiment");
        assert_eq!(call.arguments["text"], "wow");
    }

    #[test]
    fn tool_call_embedded_in_text_parses() {
        let raw = "Let me check.\n[TOOL_CALL]{\"name\":\"ocr\",\"arguments\":{}}[/TOOL_CALL]\n";
        let call = parse_tool_call(raw).unwrap().unwrap();
        assert_eq!(call.name, "ocr");
    }

    #[test]
    fn unterminated_block_is_an_error() {
        assert!(parse_tool_call("[TOOL_CALL]{\"name\":\"x\"}").is_err());
    }

    #[test]
    fn malformed_json_is_an_error() {
        assert!(parse_tool_call("[TOOL_CALL]not-json[/TOOL_CALL]").is_err());
    }

    #[test]
    fn result_block_contains_tool_name_and_output() {
        let block = tool_result_block("ocr", "{\"text\":\"HELLO\"}");
        assert!(block.contains("ocr"));
        assert!(block.contains("HELLO"));
        assert!(block.contains("Now answer"));
    }

    #[test]
    fn unknown_tool_is_rejected_at_execution_time() {
        let tools = vec![ToolBinding::new("ocr", "d", "http://127.0.0.1:1/v1/ocr")];
        let call = ToolCall {
            name: "magic".into(),
            arguments: serde_json::json!({}),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = reqwest::Client::new();
        let err = rt
            .block_on(execute_tool_call(&client, &tools, &call))
            .unwrap_err();
        assert!(err.to_string().contains("unknown tool 'magic'"));
    }
}