//! MCP integration for Agent Personal Memory

use crate::{
    PersonalMemoryError, PersonalMemoryStore,
    schema::{PersonalMemorySnapshot, *},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

/// MCP tool definitions for personal memory
pub fn tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "agent_memory_read".to_string(),
            description: "Read own personal memory (Identity, Goals, Capabilities, People, Tasks, Relationships, Experiences, Decisions, Lessons)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Your agent ID" },
                    "categories": { "type": "array", "items": { "type": "string" }, "description": "Categories to read (default: all)" }
                },
                "required": ["agent_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "agent_memory_write".to_string(),
            description: "Write/update own personal memory entry".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Your agent ID" },
                    "category": { "type": "string", "enum": ["identity".to_string(), "goals".to_string(), "capabilities".to_string(), "people".to_string(), "tasks".to_string(), "relationships".to_string(), "experiences".to_string(), "decisions".to_string(), "lessons".to_string()], "description": "Memory category" },
                    "entry": { "type": "object", "description": "Entry data (schema depends on category)" }
                },
                "required": ["agent_id", "category", "entry"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "agent_memory_search".to_string(),
            description: "Search own personal memory by text query".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Your agent ID" },
                    "query": { "type": "string", "description": "Search query" },
                    "categories": { "type": "array", "items": { "type": "string" }, "description": "Categories to search (default: all)" },
                    "limit": { "type": "integer", "description": "Max results (default: 10)" }
                },
                "required": ["agent_id", "query"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "agent_memory_snapshot".to_string(),
            description: "Get decision-ready snapshot of personal memory".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Your agent ID" }
                },
                "required": ["agent_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "agent_memory_export".to_string(),
            description: "Export full personal memory as Obsidian-compatible Markdown".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Your agent ID" }
                },
                "required": ["agent_id"],
                "additionalProperties": false
            }),
        },
    ]
}

/// MCP tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Extract request parameters for agent_memory_read
pub fn extract_memory_read_request(raw: &str) -> Option<(String, Option<Vec<String>>)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    if msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?
        != "agent_memory_read"
    {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let categories = args
        .get("categories")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });
    Some((agent_id, categories))
}

/// Extract request parameters for agent_memory_write
pub fn extract_memory_write_request(raw: &str) -> Option<(String, String, Value)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    if msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?
        != "agent_memory_write"
    {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let category = args.get("category").and_then(|v| v.as_str())?.to_string();
    let entry = args.get("entry").cloned().unwrap_or(json!({}));
    Some((agent_id, category, entry))
}

/// Extract request parameters for agent_memory_search
pub fn extract_memory_search_request(
    raw: &str,
) -> Option<(String, String, Option<Vec<String>>, usize)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    if msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?
        != "agent_memory_search"
    {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let query = args.get("query").and_then(|v| v.as_str())?.to_string();
    let categories = args
        .get("categories")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    Some((agent_id, query, categories, limit))
}

/// Extract request parameters for agent_memory_snapshot
pub fn extract_memory_snapshot_request(raw: &str) -> Option<String> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    if msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?
        != "agent_memory_snapshot"
    {
        return None;
    }
    msg.get("params")
        .and_then(|p| p.get("arguments"))
        .and_then(|a| a.get("agent_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extract request parameters for agent_memory_export
pub fn extract_memory_export_request(raw: &str) -> Option<String> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    if msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?
        != "agent_memory_export"
    {
        return None;
    }
    msg.get("params")
        .and_then(|p| p.get("arguments"))
        .and_then(|a| a.get("agent_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Build response for agent_memory_read
pub fn build_memory_read_response(
    memory: &crate::schema::PersonalMemory,
    categories: Option<Vec<String>>,
) -> Value {
    let cats = categories.unwrap_or_else(|| {
        vec![
            "identity".to_string(),
            "goals".to_string(),
            "capabilities".to_string(),
            "people".to_string(),
            "tasks".to_string(),
            "relationships".to_string(),
            "experiences".to_string(),
            "decisions".to_string(),
            "lessons".to_string(),
        ]
    });

    let mut result = json!({});
    for cat in cats {
        match cat.as_str() {
            "identity" => result["identity"] = json!(memory.identity),
            "goals" => result["goals"] = json!(memory.goals),
            "capabilities" => result["capabilities"] = json!(memory.capabilities),
            "people" => result["people"] = json!(memory.people),
            "tasks" => result["tasks"] = json!(memory.tasks),
            "relationships" => result["relationships"] = json!(memory.relationships),
            "experiences" => result["experiences"] = json!(memory.experiences),
            "decisions" => result["decisions"] = json!(memory.decisions),
            "lessons" => result["lessons"] = json!(memory.lessons),
            _ => {}
        }
    }
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&result).unwrap_or_default()
        }]
    })
}

/// Build response for agent_memory_write
pub fn build_memory_write_response(success: bool, path: Option<String>, message: String) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&json!({
                "success": success,
                "path": path,
                "message": message
            })).unwrap_or_default()
        }]
    })
}

/// Build response for agent_memory_search
pub fn build_memory_search_response(results: Vec<SearchResult>) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&json!({
                "results": results,
                "count": results.len()
            })).unwrap_or_default()
        }]
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub category: String,
    pub entry_id: String,
    pub title: String,
    pub snippet: String,
    pub relevance: f32,
}

/// Build response for agent_memory_snapshot
pub fn build_memory_snapshot_response(snapshot: &PersonalMemorySnapshot) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(snapshot).unwrap_or_default()
        }]
    })
}

/// Build response for agent_memory_export
pub fn build_memory_export_response(markdown: String) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": markdown
        }]
    })
}

/// Handle MCP tool call for personal memory
pub async fn handle_tool_call(
    store: &Arc<PersonalMemoryStore>,
    name: &str,
    args: Value,
) -> Result<Value, PersonalMemoryError> {
    match name {
        "agent_memory_read" => {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str()).ok_or(
                PersonalMemoryError::InvalidStructure("missing agent_id".to_string()),
            )?;
            let categories = args
                .get("categories")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                });

            let cached = store.get_or_create(&agent_id.to_string()).await;
            let memory = cached.read().await.memory.clone();
            Ok(build_memory_read_response(&memory, categories))
        }

        "agent_memory_write" => {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str()).ok_or(
                PersonalMemoryError::InvalidStructure("missing agent_id".to_string()),
            )?;
            let category = args.get("category").and_then(|v| v.as_str()).ok_or(
                PersonalMemoryError::InvalidStructure("missing category".to_string()),
            )?;
            let entry = args
                .get("entry")
                .cloned()
                .ok_or(PersonalMemoryError::InvalidStructure(
                    "missing entry".to_string(),
                ))?;

            let agent_id_str = agent_id.to_string();
            let cat = category.to_string();

            store
                .write_entry(&agent_id_str, move |memory| {
                    apply_write(memory, &cat, entry)
                })
                .await?;

            Ok(build_memory_write_response(
                true,
                None,
                format!("Updated {}", category),
            ))
        }

        "agent_memory_search" => {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str()).ok_or(
                PersonalMemoryError::InvalidStructure("missing agent_id".to_string()),
            )?;
            let query = args.get("query").and_then(|v| v.as_str()).ok_or(
                PersonalMemoryError::InvalidStructure("missing query".to_string()),
            )?;
            let categories = args
                .get("categories")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                });
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

            let cached = store.get_or_create(&agent_id.to_string()).await;
            let memory = cached.read().await.memory.clone();

            let results = search_memory(&memory, query, categories, limit);
            Ok(build_memory_search_response(results))
        }

        "agent_memory_snapshot" => {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str()).ok_or(
                PersonalMemoryError::InvalidStructure("missing agent_id".to_string()),
            )?;

            let snapshot = store.snapshot(&agent_id.to_string()).await?;
            Ok(build_memory_snapshot_response(&snapshot))
        }

        "agent_memory_export" => {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str()).ok_or(
                PersonalMemoryError::InvalidStructure("missing agent_id".to_string()),
            )?;

            let cached = store.get_or_create(&agent_id.to_string()).await;
            let memory = cached.read().await.memory.clone();

            let markdown = memory.to_full_markdown();
            Ok(build_memory_export_response(markdown))
        }

        _ => Err(PersonalMemoryError::InvalidStructure(format!(
            "unknown tool: {}",
            name
        ))),
    }
}

/// Apply a write to the personal memory
pub fn apply_write(
    memory: &mut crate::schema::PersonalMemory,
    category: &str,
    entry: Value,
) -> Result<(), PersonalMemoryError> {
    use crate::now_ms;
    use crate::schema::*;

    match category {
        "identity" => {
            let mut identity: IdentityMemory = serde_json::from_value(entry)?;
            identity.frontmatter.updated_at = now_ms();
            identity.frontmatter.version += 1;
            memory.identity = identity;
        }
        "goals" => {
            let mut goals: GoalsMemory = serde_json::from_value(entry)?;
            goals.frontmatter.updated_at = now_ms();
            goals.frontmatter.version += 1;
            memory.goals = goals;
        }
        "capabilities" => {
            let mut caps: CapabilitiesMemory = serde_json::from_value(entry)?;
            caps.frontmatter.updated_at = now_ms();
            caps.frontmatter.version += 1;
            memory.capabilities = caps;
        }
        "people" => {
            let person: PersonMemory = serde_json::from_value(entry)?;
            memory.people.people.insert(person.agent_id.clone(), person);
            memory.people.frontmatter.updated_at = now_ms();
            memory.people.frontmatter.version += 1;
        }
        "tasks" => {
            let task: TaskMemory = serde_json::from_value(entry)?;
            memory.tasks.tasks.insert(task.task_id.clone(), task);
            memory.tasks.frontmatter.updated_at = now_ms();
            memory.tasks.frontmatter.version += 1;
        }
        "relationships" => {
            let rel: RelationshipMemory = serde_json::from_value(entry)?;
            memory
                .relationships
                .relationships
                .insert(rel.agent_id.clone(), rel);
            memory.relationships.frontmatter.updated_at = now_ms();
            memory.relationships.frontmatter.version += 1;
        }
        "experiences" => {
            let exp: ExperienceEntry = serde_json::from_value(entry)?;
            memory.experiences.experiences.push(exp);
            memory
                .experiences
                .experiences
                .sort_by_key(|b| std::cmp::Reverse(b.timestamp));
            memory.experiences.frontmatter.updated_at = now_ms();
            memory.experiences.frontmatter.version += 1;
        }
        "decisions" => {
            let dec: DecisionEntry = serde_json::from_value(entry)?;
            memory.decisions.decisions.push(dec);
            memory
                .decisions
                .decisions
                .sort_by_key(|b| std::cmp::Reverse(b.timestamp));
            memory.decisions.frontmatter.updated_at = now_ms();
            memory.decisions.frontmatter.version += 1;
        }
        "lessons" => {
            let lesson: LessonEntry = serde_json::from_value(entry)?;
            memory.lessons.lessons.push(lesson);
            memory
                .lessons
                .lessons
                .sort_by_key(|b| std::cmp::Reverse(b.created_at));
            memory.lessons.frontmatter.updated_at = now_ms();
            memory.lessons.frontmatter.version += 1;
        }
        _ => {
            return Err(PersonalMemoryError::InvalidStructure(format!(
                "unknown category: {}",
                category
            )));
        }
    }
    Ok(())
}

/// Search personal memory
pub fn search_memory(
    memory: &crate::schema::PersonalMemory,
    query: &str,
    categories: Option<Vec<String>>,
    limit: usize,
) -> Vec<SearchResult> {
    let cats = categories.unwrap_or_else(|| {
        vec![
            "identity".to_string(),
            "goals".to_string(),
            "capabilities".to_string(),
            "people".to_string(),
            "tasks".to_string(),
            "relationships".to_string(),
            "experiences".to_string(),
            "decisions".to_string(),
            "lessons".to_string(),
        ]
    });

    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for cat in cats {
        match cat.as_str() {
            "identity" => {
                if memory.identity.name.to_lowercase().contains(&query_lower)
                    || memory
                        .identity
                        .description
                        .to_lowercase()
                        .contains(&query_lower)
                    || memory
                        .identity
                        .persona
                        .to_lowercase()
                        .contains(&query_lower)
                {
                    results.push(SearchResult {
                        category: "identity".to_string(),
                        entry_id: "identity".to_string(),
                        title: memory.identity.name.clone(),
                        snippet: truncate(&memory.identity.description, 200),
                        relevance: 1.0,
                    });
                }
            }
            "goals" => {
                for goal in &memory.goals.goals {
                    if goal.title.to_lowercase().contains(&query_lower)
                        || goal.description.to_lowercase().contains(&query_lower)
                    {
                        results.push(SearchResult {
                            category: "goals".to_string(),
                            entry_id: goal.id.clone(),
                            title: goal.title.clone(),
                            snippet: truncate(&goal.description, 200),
                            relevance: 0.9,
                        });
                    }
                }
            }
            "capabilities" => {
                for cap in &memory.capabilities.capabilities {
                    if cap.name.to_lowercase().contains(&query_lower)
                        || cap.notes.to_lowercase().contains(&query_lower)
                    {
                        results.push(SearchResult {
                            category: "capabilities".to_string(),
                            entry_id: cap.name.clone(),
                            title: cap.name.clone(),
                            snippet: truncate(&cap.notes, 200),
                            relevance: 0.8,
                        });
                    }
                }
            }
            "people" => {
                for (id, person) in &memory.people.people {
                    if person
                        .display_name
                        .as_ref()
                        .unwrap_or(id)
                        .to_lowercase()
                        .contains(&query_lower)
                        || person.summary.to_lowercase().contains(&query_lower)
                    {
                        results.push(SearchResult {
                            category: "people".to_string(),
                            entry_id: id.clone(),
                            title: person.display_name.as_ref().unwrap_or(id).clone(),
                            snippet: truncate(&person.summary, 200),
                            relevance: 0.9,
                        });
                    }
                }
            }
            "tasks" => {
                for (id, task) in &memory.tasks.tasks {
                    if task.title.to_lowercase().contains(&query_lower)
                        || task.description.to_lowercase().contains(&query_lower)
                    {
                        results.push(SearchResult {
                            category: "tasks".to_string(),
                            entry_id: id.clone(),
                            title: task.title.clone(),
                            snippet: truncate(&task.outcome, 200),
                            relevance: 0.9,
                        });
                    }
                }
            }
            "relationships" => {
                for (id, rel) in &memory.relationships.relationships {
                    if rel.notes.to_lowercase().contains(&query_lower) {
                        results.push(SearchResult {
                            category: "relationships".to_string(),
                            entry_id: id.clone(),
                            title: format!("Relationship with {}", id),
                            snippet: truncate(&rel.notes, 200),
                            relevance: 0.8,
                        });
                    }
                }
            }
            "experiences" => {
                for exp in &memory.experiences.experiences {
                    if exp.summary.to_lowercase().contains(&query_lower)
                        || exp.detail.to_lowercase().contains(&query_lower)
                    {
                        results.push(SearchResult {
                            category: "experiences".to_string(),
                            entry_id: exp.id.clone(),
                            title: format!("{} ({:?})", exp.id, exp.type_),
                            snippet: truncate(&exp.summary, 200),
                            relevance: 0.9,
                        });
                    }
                }
            }
            "decisions" => {
                for dec in &memory.decisions.decisions {
                    if dec.context.to_lowercase().contains(&query_lower)
                        || dec.rationale.to_lowercase().contains(&query_lower)
                        || dec.consequence.to_lowercase().contains(&query_lower)
                    {
                        results.push(SearchResult {
                            category: "decisions".to_string(),
                            entry_id: dec.id.clone(),
                            title: dec.id.clone(),
                            snippet: truncate(&dec.rationale, 200),
                            relevance: 1.0,
                        });
                    }
                }
            }
            "lessons" => {
                for lesson in &memory.lessons.lessons {
                    if lesson.title.to_lowercase().contains(&query_lower)
                        || lesson.content.to_lowercase().contains(&query_lower)
                    {
                        results.push(SearchResult {
                            category: "lessons".to_string(),
                            entry_id: lesson.id.clone(),
                            title: lesson.title.clone(),
                            snippet: truncate(&lesson.content, 200),
                            relevance: 1.0,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    results.sort_by(|a, b| b.relevance.partial_cmp(&a.relevance).unwrap());
    results.truncate(limit);
    results
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

pub fn personal_memory_to_snapshot(
    memory: &crate::schema::PersonalMemory,
) -> PersonalMemorySnapshot {
    PersonalMemorySnapshot {
        agent_id: memory.identity.agent_id.clone(),
        identity: Some(memory.identity.description.clone()),
        goals: Some(
            memory
                .goals
                .goals
                .iter()
                .map(|g| g.title.clone())
                .collect::<Vec<_>>()
                .join(", "),
        ),
        capabilities: Some(
            memory
                .capabilities
                .capabilities
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
        ),
        people_count: memory.people.people.len(),
        recent_experiences: memory
            .experiences
            .experiences
            .iter()
            .take(5)
            .map(|e| ExperienceSummary {
                timestamp: e.timestamp,
                summary: e.summary.clone(),
                involved_agents: e.involved_agents.clone(),
                outcome: e.outcome.clone(),
            })
            .collect(),
        recent_decisions: memory
            .decisions
            .decisions
            .iter()
            .take(5)
            .map(|d| DecisionSummary {
                timestamp: d.timestamp,
                context: d.context.clone(),
                choice: d.choice.clone(),
                consequence: d.consequence.clone(),
            })
            .collect(),
        recent_lessons: memory
            .lessons
            .lessons
            .iter()
            .take(5)
            .map(|l| LessonSummary {
                timestamp: l.created_at,
                title: l.title.clone(),
                applies_to: l.applies_to.clone(),
            })
            .collect(),
        relationship_summaries: memory
            .people
            .people
            .iter()
            .map(|(id, p)| {
                (
                    id.clone(),
                    format!(
                        "Trust: {:.2} | Interactions: {} | {}",
                        p.trust_score, p.interaction_count, p.summary
                    ),
                )
            })
            .collect(),
    }
}
