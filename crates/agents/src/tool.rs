//! Tool descriptors — first-class tool capabilities for agents (P0).
//!
//! Tools are a NEW first-class concept in the fabric: the execution
//! capability of an agent is models *and* tools, not models alone. The
//! descriptor is deliberately a structured, extensible shape (kind is a
//! string, not a closed enum) so new tool families (MCP servers, OCR
//! services, embedding endpoints, custom scripts) can be advertised without
//! recompiling every node — matching the "extensible, never a fixed list"
//! rule of the collective-intelligence architecture.

use serde::{Deserialize, Serialize};

/// A tool exposed by an agent.
///
/// The kind is a free-form string with well-known constants below; unknown
/// kinds remain valid (forward-compatible on the wire).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// Unique tool name within the agent, e.g. `"mcp.filesystem"`.
    pub name: String,
    /// Tool family. Well-known values: `mcp`, `builtin`, `http`, `custom`.
    pub kind: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// Optional JSON schema hint for the tool's input (opaque string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<String>,
}

/// MCP (Model Context Protocol) server tool.
pub const TOOL_KIND_MCP: &str = "mcp";
/// Tool implemented inside the node (e.g. registry lookup, manifest scan).
pub const TOOL_KIND_BUILTIN: &str = "builtin";
/// Tool that calls an external HTTP service.
pub const TOOL_KIND_HTTP: &str = "http";
/// Operator-defined custom tool.
pub const TOOL_KIND_CUSTOM: &str = "custom";

impl ToolDescriptor {
    /// A tool with a name and a kind; description defaults to empty.
    pub fn new(name: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            description: String::new(),
            input_schema: None,
        }
    }

    /// Sets the human-readable description.
    pub fn described(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Sets an optional JSON-schema input hint.
    pub fn with_input_schema(mut self, schema: impl Into<String>) -> Self {
        self.input_schema = Some(schema.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_round_trips_over_wire() {
        let tool = ToolDescriptor::new("mcp.filesystem", TOOL_KIND_MCP)
            .described("read and write files in the sandbox")
            .with_input_schema(r#"{"type":"object"}"#);
        let json = serde_json::to_string(&tool).unwrap();
        let back: ToolDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(tool, back);
    }

    #[test]
    fn unknown_kind_stays_valid() {
        // Forward compatibility: a future node may advertise a kind this
        // node has never seen — it must not break deserialization.
        let tool = ToolDescriptor::new("x.newthing", "quantum_planner");
        let json = serde_json::to_string(&tool).unwrap();
        let back: ToolDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, "quantum_planner");
    }

    #[test]
    fn missing_input_schema_defaults_to_none() {
        let json = r#"{"name":"a","kind":"builtin","description":"d"}"#;
        let tool: ToolDescriptor = serde_json::from_str(json).unwrap();
        assert_eq!(tool.input_schema, None);
    }
}
