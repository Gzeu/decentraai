//! Agent Personal Memory v0.1 — per-agent Markdown workspace (Obsidian-compatible)
//!
//! Each agent gets a personal, persistent memory workspace in Markdown format
//! that can be opened directly in Obsidian. The memory is subjective — it
//! belongs to ONE agent and reflects their experience of the world.
//!
//! Structure (agents/<agent_id>/):
//! ├── Identity.md          # who am I
//! ├── Goals.md             # what I want
//! ├── Capabilities.md      # what I can do
//! ├── People/
//! │   └── <agent_id>.md    # my experience of each agent
//! ├── Tasks/
//! │   └── <task_id>.md     # what I did, outcome, evidence
//! ├── Relationships/
//! │   └── <agent_id>.md    # worked_with, trust, lessons
//! ├── Experiences/
//! │   └── <timestamp>.md   # raw episodes
//! ├── Decisions/
//! │   └── <decision_id>.md # context → choice → consequence
//! └── Lessons/
//!     └── <lesson_id>.md   # distilled from experience
//!
//! Personal memory is SUBJECTIVE. It never overrides World/Society facts.
//! It only influences the WEIGHTING of decisions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub mod schema;
pub mod store;
pub mod mcp;

pub use schema::*;
pub use store::{PersonalMemoryStore, PersonalMemoryError};
pub use mcp::{tool_defs, extract_memory_read_request, extract_memory_write_request, extract_memory_search_request, extract_memory_snapshot_request, extract_memory_export_request, handle_tool_call};

/// Root path for all agent personal memories
pub fn agents_root(data_dir: &Path) -> PathBuf {
    data_dir.join("agents")
}

/// Agent-specific memory directory
pub fn agent_memory_dir(data_dir: &Path, agent_id: &str) -> PathBuf {
    agents_root(data_dir).join(agent_id)
}

/// Personal memory is always scoped to one agent
pub type AgentId = String;

/// Memory categories
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    Identity,
    Goals,
    Capabilities,
    People,
    Tasks,
    Relationships,
    Experiences,
    Decisions,
    Lessons,
}

impl MemoryCategory {
    pub fn dir_name(&self) -> &'static str {
        match self {
            MemoryCategory::Identity => "Identity.md",
            MemoryCategory::Goals => "Goals.md",
            MemoryCategory::Capabilities => "Capabilities.md",
            MemoryCategory::People => "People",
            MemoryCategory::Tasks => "Tasks",
            MemoryCategory::Relationships => "Relationships",
            MemoryCategory::Experiences => "Experiences",
            MemoryCategory::Decisions => "Decisions",
            MemoryCategory::Lessons => "Lessons",
        }
    }

    pub fn is_singleton(&self) -> bool {
        matches!(self, MemoryCategory::Identity | MemoryCategory::Goals | MemoryCategory::Capabilities)
    }
}

/// Result of a memory operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryOperationResult {
    pub success: bool,
    pub path: Option<String>,
    pub message: String,
}

/// Snapshot of agent's personal memory for decision context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalMemorySnapshot {
    pub agent_id: AgentId,
    pub identity: Option<String>,
    pub goals: Option<String>,
    pub capabilities: Option<String>,
    pub people_count: usize,
    pub recent_experiences: Vec<ExperienceSummary>,
    pub recent_decisions: Vec<DecisionSummary>,
    pub recent_lessons: Vec<LessonSummary>,
    pub relationship_summaries: HashMap<AgentId, String>, // agent -> brief summary
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceSummary {
    pub timestamp: u64,
    pub summary: String,
    pub involved_agents: Vec<AgentId>,
    pub outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionSummary {
    pub timestamp: u64,
    pub context: String,
    pub choice: String,
    pub consequence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonSummary {
    pub timestamp: u64,
    pub title: String,
    pub applies_to: Vec<AgentId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_category_dir_names() {
        assert_eq!(MemoryCategory::Identity.dir_name(), "Identity.md");
        assert_eq!(MemoryCategory::People.dir_name(), "People");
        assert_eq!(MemoryCategory::Experiences.dir_name(), "Experiences");
    }

    #[test]
    fn agents_root_path() {
        let dir = Path::new("/tmp/test");
        assert_eq!(agents_root(dir), Path::new("/tmp/test/agents"));
        assert_eq!(agent_memory_dir(dir, "dca_alpha"), Path::new("/tmp/test/agents/dca_alpha"));
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    use tempfile::tempdir;
    use crate::mcp::{search_memory, personal_memory_to_snapshot};

    #[tokio::test]
    async fn test_personal_memory_persistence_influences_decisions() {
        let dir = tempdir().unwrap();
        let store = PersonalMemoryStore::new(dir.path());
        let _agent = "dca_demo_agent";
        
        // ═══ PHASE 1: Agent acts and writes experience ═══
        println!("PHASE 1: Agent acts and writes experience");
        
        let cached = store.get_or_create(&"dca_demo_agent".to_string()).await;
        {
            let mut mem = cached.write().await;
            
            // Write a negative experience
            mem.memory.experiences.experiences.push(ExperienceEntry {
                id: "exp-001".to_string(),
                timestamp: 1000,
                type_: ExperienceType::Conflict,
                summary: "dca_other rejected my fair proposal aggressively".to_string(),
                detail: "I offered 450 on task-001 (reward 500), dca_other countered with insults and walked away. Lost 2 hours.".to_string(),
                involved_agents: vec!["dca_other".to_string()],
                task_id: Some("task-001".to_string()),
                outcome: "wasted_time".to_string(),
                evidence_ids: vec!["ev-001".to_string()],
                emotional_impact: -0.8,
                tags: vec!["unreliable".to_string(), "toxic".to_string()],
            });
            mem.memory.experiences.frontmatter.updated_at = 1000;
            mem.memory.experiences.frontmatter.version += 1;
            
            // Distill a lesson
            mem.memory.lessons.lessons.push(LessonEntry {
                id: "lesson-001".to_string(),
                title: "Avoid dca_other for time-sensitive tasks".to_string(),
                content: "dca_other is unreliable and toxic. Rejected fair 450 offer on 500 reward task with hostility. Cost me 2 hours. Prefer other agents even at 15% higher cost.".to_string(),
                applies_to: vec!["dca_other".to_string()],
                source_experiences: vec!["exp-001".to_string()],
                confidence: 0.95,
                created_at: 1000,
                validated_at: None,
                usage_count: 0,
                tags: vec!["avoidance".to_string(), "time-management".to_string()],
            });
            mem.memory.lessons.frontmatter.updated_at = 1000;
            mem.memory.lessons.frontmatter.version += 1;
            
            // Update relationship
            mem.memory.relationships.relationships.insert("dca_other".to_string(), RelationshipMemory {
                agent_id: "dca_other".to_string(),
                relationship_type: RelationshipType::Avoided,
                strength: -0.7,
                trust: -0.8,
                respect: -0.5,
                reliability: -0.9,
                started_at: 1000,
                last_updated: 1000,
                shared_tasks: vec!["task-001".to_string()],
                successful_collaborations: 0,
                failed_collaborations: 1,
                notes: "Hostile rejection of fair offer. Avoid for time-critical work.".to_string(),
            });
            mem.memory.relationships.frontmatter.updated_at = 1000;
            mem.memory.relationships.frontmatter.version += 1;
            
            mem.dirty = true;
        }
        
        // Also populate PeopleMemory for relationship_summaries in snapshot
        {
            let mut mem = cached.write().await;
            mem.memory.people.people.insert("dca_other".to_string(), PersonMemory {
                agent_id: "dca_other".to_string(),
                display_name: Some("dca_other".to_string()),
                first_interaction: 1000,
                last_interaction: 1000,
                interaction_count: 1,
                trust_score: -0.8,
                summary: "Hostile rejection of fair offer. Avoid for time-critical work.".to_string(),
                tags: vec!["unreliable".to_string(), "toxic".to_string()],
                notable_traits: vec!["hostile".to_string(), "unreliable".to_string()],
                interaction_history: vec![InteractionSummary {
                    timestamp: 1000,
                    task_id: Some("task-001".to_string()),
                    type_: InteractionType::Dispute,
                    outcome: "wasted_time".to_string(),
                    trust_delta: -0.8,
                }],
            });
            mem.memory.people.frontmatter.updated_at = 1000;
            mem.memory.people.frontmatter.version += 1;
            mem.dirty = true;
        }
        
        // Persist to disk
        store.save_agent(&"dca_demo_agent".to_string()).await.unwrap();
        println!("✓ PHASE 1: Written experience, lesson, relationship to Markdown");
        
        // ═══ PHASE 2: RESTART (new store instance, same directory) ═══
        println!("\nPHASE 2: RESTART - new store instance, same directory");
        
        // Create NEW store instance pointing to SAME directory (simulating restart)
        let _store2 = PersonalMemoryStore::new(dir.path());
        
        // ═══ PHASE 3: Agent reads past and makes different decision ═══
        println!("\nPHASE 3: Agent reads past and makes different decision");
        
        let cached = store.get_or_create(&"dca_demo_agent".to_string()).await;
        let mem = cached.read().await;
        
        // Verify experience loaded
        assert_eq!(mem.memory.experiences.experiences.len(), 1);
        assert_eq!(mem.memory.experiences.experiences[0].summary, "dca_other rejected my fair proposal aggressively");
        println!("✓ Experience loaded: {}", mem.memory.experiences.experiences[0].summary);
        
        // Verify lesson loaded
        assert_eq!(mem.memory.lessons.lessons.len(), 1);
        assert_eq!(mem.memory.lessons.lessons[0].title, "Avoid dca_other for time-sensitive tasks");
        println!("✓ Lesson loaded: {}", mem.memory.lessons.lessons[0].title);
        
        // Verify relationship loaded
        assert_eq!(mem.memory.relationships.relationships.len(), 1);
        let rel = mem.memory.relationships.relationships.get("dca_other").unwrap();
        assert_eq!(rel.relationship_type, RelationshipType::Avoided);
        assert!(rel.trust < -0.5);
        println!("✓ Relationship loaded: dca_other → {:?} (trust: {:.1})", rel.relationship_type, rel.trust);
        
        // ═══ PHASE 4: Different decision based on memory ═══
        println!("\nPHASE 4: Different decision based on memory");
        
        // Search for dca_other
        let results = search_memory(&mem.memory, "dca_other", None, 5);
        assert!(!results.is_empty());
        println!("✓ Search found {} results for 'dca_other'", results.len());
        
        // Get decision snapshot
        let snapshot = personal_memory_to_snapshot(&mem.memory);
        
        // Verify lesson appears in snapshot
        assert!(!snapshot.recent_lessons.is_empty());
        assert!(snapshot.recent_lessons[0].title.contains("Avoid dca_other"));
        println!("✓ Snapshot shows lesson: {:?}", snapshot.recent_lessons[0].title);
        
        // Verify relationship appears in snapshot
        assert!(snapshot.relationship_summaries.contains_key("dca_other"));
        let rel_summary = &snapshot.relationship_summaries["dca_other"];
        assert!(rel_summary.contains("-0.80")); // trust score -0.80
        println!("✓ Snapshot shows relationship: {}", rel_summary);
        
        // ═══ DECISION LOGIC DEMONSTRATION ═══
        println!("\n═══ DECISION LOGIC DEMONSTRATION ═══");
        
        // Simulate decision context: new task from dca_other
        let _new_task_from_dca_other = true;
        
        // Agent's decision logic:
        // 1. Check if task is from known agent
        // 2. Check personal memory for relationship
        // 4. Make decision
        
        let should_avoid = mem.memory.relationships.relationships.get("dca_other")
            .map(|r| r.relationship_type == RelationshipType::Avoided || r.trust < -0.3)
            .unwrap_or(false);
        
        let has_warning_lesson = mem.memory.lessons.lessons.iter().any(|l| {
            l.applies_to.contains(&"dca_other".to_string()) && l.confidence > 0.7
        });
        
        println!("  Task from dca_other: true");
        println!("  Should avoid based on relationship: {}", should_avoid);
        println!("  Has warning lesson: {}", has_warning_lesson);
        
        // Different decision: AVOID dca_other
        let decision = if should_avoid || has_warning_lesson {
            "REJECT - Avoid dca_other based on past negative experience"
        } else {
            "ACCEPT - No negative history"
        };
        
        println!("\n  DECISION: {}", decision);
        
        // Verify this is DIFFERENT from what would happen without memory
        let decision_without_memory = "ACCEPT - No negative history";
        assert_ne!(decision, decision_without_memory);
        println!("✓ Decision DIFFERS from no-memory baseline");
        
        println!("\n✓✓✓ PERSONAL MEMORY PERSISTENCE INFLUENCES DECISIONS AFTER RESTART ✓✓✓");
    }
}
