//! Markdown schemas for each personal memory category
//! 
//! Each category has a defined frontmatter + body structure.
//! Compatible with Obsidian (YAML frontmatter + Markdown body).

use serde::{Deserialize, Serialize};
use crate::AgentId;
use std::collections::HashMap;

/// Frontmatter common to all memory files
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryFrontmatter {
    pub created_at: u64,
    pub updated_at: u64,
    pub version: u32,
    pub tags: Vec<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// ─── IDENTITY ───
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentityMemory {
    #[serde(flatten)]
    pub frontmatter: MemoryFrontmatter,
    pub agent_id: String,
    pub name: String,
    pub description: String,
    pub persona: String,           // e.g. "analytical negotiator", "cautious builder"
    pub values: Vec<String>,       // e.g. ["reliability", "fairness", "learning"]
    pub communication_style: String,
}

impl IdentityMemory {
    pub fn new(agent_id: String) -> Self {
        let now = crate::now_ms();
        Self {
            frontmatter: MemoryFrontmatter {
                created_at: now,
                updated_at: now,
                version: 1,
                tags: vec!["identity".to_string()],
                extra: HashMap::new(),
            },
            agent_id,
            name: String::new(),
            description: String::new(),
            persona: String::new(),
            values: Vec::new(),
            communication_style: String::new(),
        }
    }

    pub fn to_markdown(&self) -> String {
        let fm = serde_yaml::to_string(&self.frontmatter).unwrap_or_default();
        format!(
            "---\n{}---\n\n#[derive(Debug, Clone, Serialize, Deserialize)]Identity\n\n**Agent ID**: {}\n\n**Name**: {}\n\n**Description**: {}\n\n**Persona**: {}\n\n**Values**:\n{}\n\n**Communication Style**: {}\n",
            fm,
            self.agent_id,
            self.name,
            self.description,
            self.persona,
            if self.values.is_empty() { "  (none)".to_string() } else { self.values.iter().map(|v| format!("  - {}", v)).collect::<Vec<_>>().join("\n") },
            self.communication_style
        )
    }
}

/// ─── GOALS ───
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GoalEntry {
    pub id: String,
    pub title: String,
    pub description: String,
    pub priority: u8,           // 1-10
    pub status: GoalStatus,
    pub created_at: u64,
    pub target_date: Option<u64>,
    pub progress: u8,           // 0-100
    pub related_agents: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum GoalStatus {
    #[default]
    Active,
    Paused,
    Completed,
    Abandoned,
}


#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GoalsMemory {
    #[serde(flatten)]
    pub frontmatter: MemoryFrontmatter,
    pub goals: Vec<GoalEntry>,
}

impl GoalsMemory {
    pub fn new() -> Self {
        let now = crate::now_ms();
        Self {
            frontmatter: MemoryFrontmatter {
                created_at: now,
                updated_at: now,
                version: 1,
                tags: vec!["goals".to_string()],
                extra: HashMap::new(),
            },
            goals: Vec::new(),
        }
    }

    pub fn to_markdown(&self) -> String {
        let fm = serde_yaml::to_string(&self.frontmatter).unwrap_or_default();
        let mut md = format!("---\n{}---\n\n#[derive(Debug, Clone, Serialize, Deserialize)]Goals\n\n", fm);
        
        for goal in &self.goals {
            md.push_str(&format!(
                "##[derive(Debug, Clone, Serialize, Deserialize)]{} ({})\n\n{}\n\n**Priority**: {}/10 | **Status**: {:?} | **Progress**: {}%\n\n",
                goal.title,
                goal.id,
                goal.description,
                goal.priority,
                goal.status,
                goal.progress
            ));
            if let Some(date) = goal.target_date {
                md.push_str(&format!("**Target**: {}\n\n", crate::format_ts(date)));
            }
            if !goal.related_agents.is_empty() {
                md.push_str(&format!("**Related Agents**: {}\n\n", goal.related_agents.join(", ")));
            }
            md.push_str("---\n\n");
        }
        md
    }
}

/// ─── CAPABILITIES ───
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilityEntry {
    pub name: String,
    pub level: CapabilityLevel,
    pub evidence: Vec<String>,     // evidence IDs
    pub last_used: Option<u64>,
    pub notes: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityLevel {
    #[default]
    Novice,
    Competent,
    Proficient,
    Expert,
    Master,
}



#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilitiesMemory {
    #[serde(flatten)]
    pub frontmatter: MemoryFrontmatter,
    pub capabilities: Vec<CapabilityEntry>,
}

impl CapabilitiesMemory {
    pub fn new() -> Self {
        let now = crate::now_ms();
        Self {
            frontmatter: MemoryFrontmatter {
                created_at: now,
                updated_at: now,
                version: 1,
                tags: vec!["capabilities".to_string()],
                extra: HashMap::new(),
            },
            capabilities: Vec::new(),
        }
    }

    pub fn to_markdown(&self) -> String {
        let fm = serde_yaml::to_string(&self.frontmatter).unwrap_or_default();
        let mut md = format!("---\n{}---\n\n#[derive(Debug, Clone, Serialize, Deserialize)]Capabilities\n\n", fm);
        
        for cap in &self.capabilities {
            md.push_str(&format!(
                "##[derive(Debug, Clone, Serialize, Deserialize)]{}\n\n**Level**: {:?}\n\n**Evidence**: {}\n\n**Notes**: {}\n\n---\n\n",
                cap.name,
                cap.level,
                if cap.evidence.is_empty() { "none".to_string() } else { cap.evidence.join(", ") },
                cap.notes
            ));
        }
        md
    }
}

/// ─── PEOPLE (subjective experience of other agents) ───
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersonMemory {
    pub agent_id: String,
    pub display_name: Option<String>,
    pub first_interaction: u64,
    pub last_interaction: u64,
    pub interaction_count: u64,
    pub trust_score: f32,          // -1.0 to 1.0 (subjective!)
    pub summary: String,
    pub tags: Vec<String>,
    pub notable_traits: Vec<String>,
    pub interaction_history: Vec<InteractionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InteractionSummary {
    pub timestamp: u64,
    pub task_id: Option<String>,
    pub type_: InteractionType,
    pub outcome: String,
    pub trust_delta: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InteractionType {
    #[default]
    TaskCollaboration,
    Bid,
    Proposal,
    Negotiation,
    Settlement,
    Dispute,
    Social,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PeopleMemory {
    #[serde(flatten)]
    pub frontmatter: MemoryFrontmatter,
    pub people: HashMap<String, PersonMemory>,
}

impl PeopleMemory {
    pub fn new() -> Self {
        let now = crate::now_ms();
        Self {
            frontmatter: MemoryFrontmatter {
                created_at: now,
                updated_at: now,
                version: 1,
                tags: vec!["people".to_string()],
                extra: HashMap::new(),
            },
            people: HashMap::new(),
        }
    }

    pub fn to_markdown(&self) -> String {
        let fm = serde_yaml::to_string(&self.frontmatter).unwrap_or_default();
        let mut md = format!("---\n{}---\n\n#[derive(Debug, Clone, Serialize, Deserialize)]People\n\n", fm);
        
        for (id, person) in &self.people {
            md.push_str(&format!(
                "##[derive(Debug, Clone, Serialize, Deserialize)]{} ({})\n\n**Trust**: {:.2} | **Interactions**: {} | **First**: {} | **Last**: {}\n\n**Summary**: {}\n\n**Traits**: {}\n\n**Tags**: {}\n\n---\n\n",
                person.display_name.as_ref().unwrap_or(id),
                id,
                person.trust_score,
                person.interaction_count,
                crate::format_ts(person.first_interaction),
                crate::format_ts(person.last_interaction),
                person.summary,
                person.notable_traits.join(", "),
                person.tags.join(", ")
            ));
        }
        md
    }
}

/// ─── TASKS (what I did, outcome, evidence) ───
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskMemory {
    pub task_id: String,
    pub title: String,
    pub description: String,
    pub role: TaskRole,
    pub status: TaskMemoryStatus,
    pub reward: u64,
    pub evidence_id: Option<String>,
    pub started_at: u64,
    pub completed_at: Option<u64>,
    pub teammates: Vec<String>,
    pub outcome: String,
    pub lessons: Vec<String>,
    pub self_rating: Option<u8>,  // 1-10 how well I did
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskRole {
    #[default]
    Issuer,
    Bidder,
    Proposer,
    TeamMember,
    Executor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskMemoryStatus {
    #[default]
    Active,
    Completed,
    Settled,
    Failed,
    Disputed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TasksMemory {
    #[serde(flatten)]
    pub frontmatter: MemoryFrontmatter,
    pub tasks: HashMap<String, TaskMemory>,
}

impl TasksMemory {
    pub fn new() -> Self {
        let now = crate::now_ms();
        Self {
            frontmatter: MemoryFrontmatter {
                created_at: now,
                updated_at: now,
                version: 1,
                tags: vec!["tasks".to_string()],
                extra: HashMap::new(),
            },
            tasks: HashMap::new(),
        }
    }

    pub fn to_markdown(&self) -> String {
        let fm = serde_yaml::to_string(&self.frontmatter).unwrap_or_default();
        let mut md = format!("---\n{}---\n\n#[derive(Debug, Clone, Serialize, Deserialize)]Tasks\n\n", fm);
        
        for (id, task) in &self.tasks {
            md.push_str(&format!(
                "##[derive(Debug, Clone, Serialize, Deserialize)]{} ({})\n\n**Role**: {:?} | **Status**: {:?} | **Reward**: {} Cr\n\n**Description**: {}\n\n**Teammates**: {}\n\n**Outcome**: {}\n\n**Evidence**: {}\n\n**Self-Rating**: {}/10\n\n**Lessons**: {}\n\n---\n\n",
                task.title, id, task.role, task.status, task.reward,
                task.description,
                task.teammates.join(", "),
                task.outcome,
                task.evidence_id.as_deref().unwrap_or("none"),
                task.self_rating.unwrap_or(0),
                task.lessons.join("; ")
            ));
        }
        md
    }
}

/// ─── RELATIONSHIPS (structured view of connections) ───
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RelationshipMemory {
    pub agent_id: String,
    pub relationship_type: RelationshipType,
    pub strength: f32,             // -1.0 to 1.0
    pub trust: f32,
    pub respect: f32,
    pub reliability: f32,
    pub started_at: u64,
    pub last_updated: u64,
    pub shared_tasks: Vec<String>,
    pub successful_collaborations: u32,
    pub failed_collaborations: u32,
    pub notes: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipType {
    #[default]
    Stranger,
    Acquaintance,
    Collaborator,
    TrustedPartner,
    PreferredPartner,
    Rival,
    Avoided,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RelationshipsMemory {
    #[serde(flatten)]
    pub frontmatter: MemoryFrontmatter,
    pub relationships: HashMap<String, RelationshipMemory>,
}

impl RelationshipsMemory {
    pub fn new() -> Self {
        let now = crate::now_ms();
        Self {
            frontmatter: MemoryFrontmatter {
                created_at: now,
                updated_at: now,
                version: 1,
                tags: vec!["relationships".to_string()],
                extra: HashMap::new(),
            },
            relationships: HashMap::new(),
        }
    }

    pub fn to_markdown(&self) -> String {
        let fm = serde_yaml::to_string(&self.frontmatter).unwrap_or_default();
        let mut md = format!("---\n{}---\n\n#[derive(Debug, Clone, Serialize, Deserialize)]Relationships\n\n", fm);
        
        for (id, rel) in &self.relationships {
            md.push_str(&format!(
                "##[derive(Debug, Clone, Serialize, Deserialize)]{}\n\n**Type**: {:?} | **Strength**: {:.2} | **Trust**: {:.2} | **Respect**: {:.2} | **Reliability**: {:.2}\n\n**Shared Tasks**: {}\n\n**Successful**: {} | **Failed**: {}\n\n**Notes**: {}\n\n---\n\n",
                id, rel.relationship_type, rel.strength, rel.trust, rel.respect, rel.reliability,
                rel.shared_tasks.join(", "),
                rel.successful_collaborations, rel.failed_collaborations,
                rel.notes
            ));
        }
        md
    }
}

/// ─── EXPERIENCES (raw episodes) ───
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExperienceEntry {
    pub id: String,
    pub timestamp: u64,
    pub type_: ExperienceType,
    pub summary: String,
    pub detail: String,
    pub involved_agents: Vec<String>,
    pub task_id: Option<String>,
    pub outcome: String,
    pub evidence_ids: Vec<String>,
    pub emotional_impact: f32,   // -1.0 to 1.0
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceType {
    #[default]
    TaskCompletion,
    Negotiation,
    Conflict,
    Learning,
    Observation,
    Surprise,
    Disappointment,
    Success,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExperiencesMemory {
    #[serde(flatten)]
    pub frontmatter: MemoryFrontmatter,
    pub experiences: Vec<ExperienceEntry>,
}

impl ExperiencesMemory {
    pub fn new() -> Self {
        let now = crate::now_ms();
        Self {
            frontmatter: MemoryFrontmatter {
                created_at: now,
                updated_at: now,
                version: 1,
                tags: vec!["experiences".to_string()],
                extra: HashMap::new(),
            },
            experiences: Vec::new(),
        }
    }

    pub fn to_markdown(&self) -> String {
        let fm = serde_yaml::to_string(&self.frontmatter).unwrap_or_default();
        let mut md = format!("---\n{}---\n\n#[derive(Debug, Clone, Serialize, Deserialize)]Experiences\n\n", fm);
        
        for exp in &self.experiences {
            md.push_str(&format!(
                "##[derive(Debug, Clone, Serialize, Deserialize)]{} ({})\n\n**Type**: {:?} | **Impact**: {:.2} | **Agents**: {}\n\n**Summary**: {}\n\n**Detail**: {}\n\n**Outcome**: {}\n\n**Evidence**: {}\n\n**Tags**: {}\n\n---\n\n",
                exp.id, crate::format_ts(exp.timestamp),
                exp.type_, exp.emotional_impact, exp.involved_agents.join(", "),
                exp.summary, exp.detail, exp.outcome,
                exp.evidence_ids.join(", "),
                exp.tags.join(", ")
            ));
        }
        md
    }
}

/// ─── DECISIONS (context → choice → consequence) ───
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DecisionEntry {
    pub id: String,
    pub timestamp: u64,
    pub context: String,           // what I saw
    pub options_considered: Vec<OptionConsidered>,
    pub choice: String,            // what I picked
    pub rationale: String,         // why
    pub consequence: String,       // what happened
    pub evidence_ids: Vec<String>,
    pub confidence: f32,           // 0-1 at decision time
    pub hindsight: Option<String>, // reflection after outcome
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OptionConsidered {
    pub action: String,
    pub expected_outcome: String,
    pub risk: f32,
    pub rejected_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DecisionsMemory {
    #[serde(flatten)]
    pub frontmatter: MemoryFrontmatter,
    pub decisions: Vec<DecisionEntry>,
}

impl DecisionsMemory {
    pub fn new() -> Self {
        let now = crate::now_ms();
        Self {
            frontmatter: MemoryFrontmatter {
                created_at: now,
                updated_at: now,
                version: 1,
                tags: vec!["decisions".to_string()],
                extra: HashMap::new(),
            },
            decisions: Vec::new(),
        }
    }

    pub fn to_markdown(&self) -> String {
        let fm = serde_yaml::to_string(&self.frontmatter).unwrap_or_default();
        let mut md = format!("---\n{}---\n\n#[derive(Debug, Clone, Serialize, Deserialize)]Decisions\n\n", fm);
        
        for dec in &self.decisions {
            md.push_str(&format!(
                "##[derive(Debug, Clone, Serialize, Deserialize)]{} ({})\n\n**Context**: {}\n\n**Options Considered**:\n{}\n\n**Choice**: {}\n\n**Rationale**: {}\n\n**Consequence**: {}\n\n**Confidence**: {:.0}% | **Hindsight**: {}\n\n**Evidence**: {}\n\n**Tags**: {}\n\n---\n\n",
                dec.id, crate::format_ts(dec.timestamp),
                dec.context,
                dec.options_considered.iter().map(|o| format!("  - {}: {} (risk: {:.0}%) — rejected: {}", o.action, o.expected_outcome, o.risk * 100.0, o.rejected_reason)).collect::<Vec<_>>().join("\n"),
                dec.choice,
                dec.rationale,
                dec.consequence,
                dec.confidence * 100.0,
                dec.hindsight.as_deref().unwrap_or("(none)"),
                dec.evidence_ids.join(", "),
                dec.tags.join(", ")
            ));
        }
        md
    }
}

/// ─── LESSONS (distilled wisdom) ───
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LessonEntry {
    pub id: String,
    pub title: String,
    pub content: String,
    pub applies_to: Vec<String>,      // agent IDs or "all"
    pub source_experiences: Vec<String>, // experience IDs
    pub confidence: f32,              // 0-1
    pub created_at: u64,
    pub validated_at: Option<u64>,
    pub usage_count: u32,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LessonsMemory {
    #[serde(flatten)]
    pub frontmatter: MemoryFrontmatter,
    pub lessons: Vec<LessonEntry>,
}

impl LessonsMemory {
    pub fn new() -> Self {
        let now = crate::now_ms();
        Self {
            frontmatter: MemoryFrontmatter {
                created_at: now,
                updated_at: now,
                version: 1,
                tags: vec!["lessons".to_string()],
                extra: HashMap::new(),
            },
            lessons: Vec::new(),
        }
    }

    pub fn to_markdown(&self) -> String {
        let fm = serde_yaml::to_string(&self.frontmatter).unwrap_or_default();
        let mut md = format!("---\n{}---\n\n#[derive(Debug, Clone, Serialize, Deserialize)]Lessons\n\n", fm);
        
        for lesson in &self.lessons {
            md.push_str(&format!(
                "##[derive(Debug, Clone, Serialize, Deserialize)]{} ({})\n\n**Content**: {}\n\n**Applies To**: {}\n\n**Source**: {}\n\n**Confidence**: {:.0}% | **Usage**: {}\n\n**Tags**: {}\n\n---\n\n",
                lesson.title, lesson.id,
                lesson.content,
                lesson.applies_to.join(", "),
                lesson.source_experiences.join(", "),
                lesson.confidence * 100.0,
                lesson.usage_count,
                lesson.tags.join(", ")
            ));
        }
        md
    }
}

/// ─── COMPLETE PERSONAL MEMORY ───
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersonalMemory {
    pub identity: IdentityMemory,
    pub goals: GoalsMemory,
    pub capabilities: CapabilitiesMemory,
    pub people: PeopleMemory,
    pub tasks: TasksMemory,
    pub relationships: RelationshipsMemory,
    pub experiences: ExperiencesMemory,
    pub decisions: DecisionsMemory,
    pub lessons: LessonsMemory,
}

impl PersonalMemory {
    pub fn new(agent_id: String) -> Self {
        let mut identity = IdentityMemory::new(agent_id.clone());
        identity.agent_id = agent_id.clone();
        
        Self {
            identity,
            goals: GoalsMemory::new(),
            capabilities: CapabilitiesMemory::new(),
            people: PeopleMemory::new(),
            tasks: TasksMemory::new(),
            relationships: RelationshipsMemory::new(),
            experiences: ExperiencesMemory::new(),
            decisions: DecisionsMemory::new(),
            lessons: LessonsMemory::new(),
        }
    }

    pub fn to_full_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str(&self.identity.to_markdown());
        md.push_str("\n---\n\n");
        md.push_str(&self.goals.to_markdown());
        md.push_str("\n---\n\n");
        md.push_str(&self.capabilities.to_markdown());
        md.push_str("\n---\n\n");
        md.push_str(&self.people.to_markdown());
        md.push_str("\n---\n\n");
        md.push_str(&self.tasks.to_markdown());
        md.push_str("\n---\n\n");
        md.push_str(&self.relationships.to_markdown());
        md.push_str("\n---\n\n");
        md.push_str(&self.experiences.to_markdown());
        md.push_str("\n---\n\n");
        md.push_str(&self.decisions.to_markdown());
        md.push_str("\n---\n\n");
        md.push_str(&self.lessons.to_markdown());
        md
    }
}

/// Helper functions
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn format_ts(ts: u64) -> String {
    use chrono::{DateTime, Utc};
    DateTime::<Utc>::from_timestamp(ts as i64 / 1000, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| format!("{} ms", ts))
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
