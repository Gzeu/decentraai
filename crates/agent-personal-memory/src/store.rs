//! Personal memory store — file-based Markdown persistence
use serde::Deserialize;

use crate::{
    AgentId,
    schema::{
        PersonalMemory as SchemaMemory,
        IdentityMemory, GoalsMemory, CapabilitiesMemory,
        PeopleMemory, TasksMemory, RelationshipsMemory,
        ExperiencesMemory, DecisionsMemory, LessonsMemory,
        PersonMemory, TaskMemory, RelationshipMemory,
        ExperienceEntry, DecisionEntry, LessonEntry,
        MemoryFrontmatter,
        PersonalMemorySnapshot,
    },
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
#[derive(Debug, thiserror::Error)]
pub enum PersonalMemoryError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("Agent memory not found: {0}")]
    NotFound(String),
    #[error("Invalid memory structure: {0}")]
    InvalidStructure(String),
    #[error("Agent ID mismatch: expected {expected}, found {found}")]
    AgentIdMismatch { expected: String, found: String },
}



/// Per-agent personal memory store
#[derive(Debug)]
pub struct PersonalMemoryStore {
    data_dir: PathBuf,
    caches: Arc<RwLock<HashMap<AgentId, Arc<RwLock<CachedMemory>>>>>,
}

#[derive(Debug)]
pub struct CachedMemory {
    pub memory: SchemaMemory,
    pub dirty: bool,
    pub last_saved: u64,
}

impl PersonalMemoryStore {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            data_dir: data_dir.as_ref().to_path_buf(),
            caches: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get or create personal memory for an agent
    pub async fn get_or_create(&self, agent_id: &AgentId) -> Arc<RwLock<CachedMemory>> {
        let mut caches = self.caches.write().await;
        
        if let Some(cached) = caches.get(agent_id) {
            return cached.clone();
        }

        // Load from disk or create new
        let memory = self.load_from_disk(agent_id).await.unwrap_or_else(|_| {
            SchemaMemory::new(agent_id.clone())
        });

        let cached = Arc::new(RwLock::new(CachedMemory {
            memory,
            dirty: false,
            last_saved: 0,
        }));
        
        caches.insert(agent_id.clone(), cached.clone());
        cached
    }

    /// Load personal memory from Markdown files
    async fn load_from_disk(&self, agent_id: &AgentId) -> Result<SchemaMemory, PersonalMemoryError> {
        let agent_dir = self.agent_dir(agent_id);
        
        if !agent_dir.exists() {
            return Err(PersonalMemoryError::NotFound(agent_id.clone()));
        }

        let mut memory = SchemaMemory::default();
        memory.identity.agent_id = agent_id.clone();

        // Load each category
        memory.identity = self.load_identity(&agent_dir).await?;
        memory.goals = self.load_goals(&agent_dir).await?;
        memory.capabilities = self.load_capabilities(&agent_dir).await?;
        memory.people = self.load_people(&agent_dir).await?;
        memory.tasks = self.load_tasks(&agent_dir).await?;
        memory.relationships = self.load_relationships(&agent_dir).await?;
        memory.experiences = self.load_experiences(&agent_dir).await?;
        memory.decisions = self.load_decisions(&agent_dir).await?;
        memory.lessons = self.load_lessons(&agent_dir).await?;

        Ok(memory)
    }

    fn agent_dir(&self, agent_id: &AgentId) -> PathBuf {
        self.data_dir.join("agents").join(agent_id)
    }

    #[allow(dead_code)]
    fn category_dir(&self, agent_id: &AgentId, category: &str) -> PathBuf {
        self.agent_dir(agent_id).join(category)
    }

    #[allow(dead_code)]
    fn file_path(&self, agent_id: &AgentId, filename: &str) -> PathBuf {
        self.agent_dir(agent_id).join(filename)
    }

    // ─── LOADERS ───

    async fn load_identity(&self, agent_dir: &Path) -> Result<IdentityMemory, PersonalMemoryError> {
        let path = agent_dir.join("Identity.md");
        if !path.exists() {
            return Ok(IdentityMemory::default());
        }
        let content = tokio::fs::read_to_string(&path).await?;
        parse_markdown_file(&content)
    }

    async fn load_goals(&self, agent_dir: &Path) -> Result<GoalsMemory, PersonalMemoryError> {
        let path = agent_dir.join("Goals.md");
        if !path.exists() {
            return Ok(GoalsMemory::new());
        }
        let content = tokio::fs::read_to_string(&path).await?;
        parse_markdown_file(&content)
    }

    async fn load_capabilities(&self, agent_dir: &Path) -> Result<CapabilitiesMemory, PersonalMemoryError> {
        let path = agent_dir.join("Capabilities.md");
        if !path.exists() {
            return Ok(CapabilitiesMemory::new());
        }
        let content = tokio::fs::read_to_string(&path).await?;
        parse_markdown_file(&content)
    }

    async fn load_people(&self, agent_dir: &Path) -> Result<PeopleMemory, PersonalMemoryError> {
        let people_dir = agent_dir.join("People");
        if !people_dir.exists() {
            return Ok(PeopleMemory::new());
        }

        let mut people = PeopleMemory::new();
        let mut entries = tokio::fs::read_dir(&people_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let content = tokio::fs::read_to_string(&path).await?;
                if let Ok(person) = parse_markdown_file::<PersonMemory>(&content) {
                    people.people.insert(person.agent_id.clone(), person);
                }
            }
        }
        Ok(people)
    }

    async fn load_tasks(&self, agent_dir: &Path) -> Result<TasksMemory, PersonalMemoryError> {
        let tasks_dir = agent_dir.join("Tasks");
        if !tasks_dir.exists() {
            return Ok(TasksMemory::new());
        }

        let mut tasks = TasksMemory::new();
        let mut entries = tokio::fs::read_dir(&tasks_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let content = tokio::fs::read_to_string(&path).await?;
                if let Ok(task) = parse_markdown_file::<TaskMemory>(&content) {
                    tasks.tasks.insert(task.task_id.clone(), task);
                }
            }
        }
        Ok(tasks)
    }

    async fn load_relationships(&self, agent_dir: &Path) -> Result<RelationshipsMemory, PersonalMemoryError> {
        let rel_dir = agent_dir.join("Relationships");
        if !rel_dir.exists() {
            return Ok(RelationshipsMemory::new());
        }

        let mut relationships = RelationshipsMemory::new();
        let mut entries = tokio::fs::read_dir(&rel_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let content = tokio::fs::read_to_string(&path).await?;
                if let Ok(rel) = parse_markdown_file::<RelationshipMemory>(&content) {
                    relationships.relationships.insert(rel.agent_id.clone(), rel);
                }
            }
        }
        Ok(relationships)
    }

    async fn load_experiences(&self, agent_dir: &Path) -> Result<ExperiencesMemory, PersonalMemoryError> {
        let exp_dir = agent_dir.join("Experiences");
        if !exp_dir.exists() {
            return Ok(ExperiencesMemory::new());
        }

        let mut experiences = ExperiencesMemory::new();
        let mut entries = tokio::fs::read_dir(&exp_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let content = tokio::fs::read_to_string(&path).await?;
                if let Ok(exp) = parse_markdown_file::<ExperienceEntry>(&content) {
                    experiences.experiences.push(exp);
                }
            }
        }
        // Sort by timestamp descending
        experiences.experiences.sort_by_key(|b| std::cmp::Reverse(b.timestamp));
        Ok(experiences)
    }

    async fn load_decisions(&self, agent_dir: &Path) -> Result<DecisionsMemory, PersonalMemoryError> {
        let dec_dir = agent_dir.join("Decisions");
        if !dec_dir.exists() {
            return Ok(DecisionsMemory::new());
        }

        let mut decisions = DecisionsMemory::new();
        let mut entries = tokio::fs::read_dir(&dec_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let content = tokio::fs::read_to_string(&path).await?;
                if let Ok(dec) = parse_markdown_file::<DecisionEntry>(&content) {
                    decisions.decisions.push(dec);
                }
            }
        }
        decisions.decisions.sort_by_key(|b| std::cmp::Reverse(b.timestamp));
        Ok(decisions)
    }

    async fn load_lessons(&self, agent_dir: &Path) -> Result<LessonsMemory, PersonalMemoryError> {
        let les_dir = agent_dir.join("Lessons");
        if !les_dir.exists() {
            return Ok(LessonsMemory::new());
        }

        let mut lessons = LessonsMemory::new();
        let mut entries = tokio::fs::read_dir(&les_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let content = tokio::fs::read_to_string(&path).await?;
                if let Ok(lesson) = parse_markdown_file::<LessonEntry>(&content) {
                    lessons.lessons.push(lesson);
                }
            }
        }
        lessons.lessons.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        Ok(lessons)
    }

    /// Save all dirty caches to disk
    pub async fn save_all(&self) -> Result<(), PersonalMemoryError> {
        let caches = self.caches.read().await;
        for cached in caches.values() {
            let mut cached = cached.write().await;
            if cached.dirty {
                self.save_to_disk(&cached.memory).await?;
                cached.dirty = false;
                cached.last_saved = crate::now_ms();
            }
        }
        Ok(())
    }

    /// Save specific agent's memory
    pub async fn save_agent(&self, agent_id: &AgentId) -> Result<(), PersonalMemoryError> {
        let caches = self.caches.read().await;
        if let Some(cached) = caches.get(agent_id) {
            let mut cached = cached.write().await;
            if cached.dirty {
                self.save_to_disk(&cached.memory).await?;
                cached.dirty = false;
                cached.last_saved = crate::now_ms();
            }
        }
        Ok(())
    }

    async fn save_to_disk(&self, memory: &SchemaMemory) -> Result<(), PersonalMemoryError> {
        let agent_id = &memory.identity.agent_id;
        let agent_dir = self.agent_dir(agent_id);
        
        // Create directories
        tokio::fs::create_dir_all(&agent_dir).await?;
        tokio::fs::create_dir_all(agent_dir.join("People")).await?;
        tokio::fs::create_dir_all(agent_dir.join("Tasks")).await?;
        tokio::fs::create_dir_all(agent_dir.join("Relationships")).await?;
        tokio::fs::create_dir_all(agent_dir.join("Experiences")).await?;
        tokio::fs::create_dir_all(agent_dir.join("Decisions")).await?;
        tokio::fs::create_dir_all(agent_dir.join("Lessons")).await?;

        // Write singleton files
        tokio::fs::write(agent_dir.join("Identity.md"), memory.identity.to_markdown()).await?;
        tokio::fs::write(agent_dir.join("Goals.md"), memory.goals.to_markdown()).await?;
        tokio::fs::write(agent_dir.join("Capabilities.md"), memory.capabilities.to_markdown()).await?;

        // Write People (one file per person)
        for (id, person) in &memory.people.people {
            let content = person_to_markdown(person);
            tokio::fs::write(agent_dir.join("People").join(format!("{}.md", sanitize_filename(id))), content).await?;
        }

        // Write Tasks
        for (id, task) in &memory.tasks.tasks {
            let content = task_to_markdown(task);
            tokio::fs::write(agent_dir.join("Tasks").join(format!("{}.md", sanitize_filename(id))), content).await?;
        }

        // Write Relationships
        for (id, rel) in &memory.relationships.relationships {
            let content = relationship_to_markdown(rel);
            tokio::fs::write(agent_dir.join("Relationships").join(format!("{}.md", sanitize_filename(id))), content).await?;
        }

        // Write Experiences
        for exp in &memory.experiences.experiences {
            let content = experience_to_markdown(exp);
            tokio::fs::write(agent_dir.join("Experiences").join(format!("{}.md", sanitize_filename(&exp.id))), content).await?;
        }

        // Write Decisions
        for dec in &memory.decisions.decisions {
            let content = decision_to_markdown(dec);
            tokio::fs::write(agent_dir.join("Decisions").join(format!("{}.md", sanitize_filename(&dec.id))), content).await?;
        }

        // Write Lessons
        for lesson in &memory.lessons.lessons {
            let content = lesson_to_markdown(lesson);
            tokio::fs::write(agent_dir.join("Lessons").join(format!("{}.md", sanitize_filename(&lesson.id))), content).await?;
        }

        Ok(())
    }

    /// Mark agent's memory as dirty (needs saving)
    pub async fn mark_dirty(&self, agent_id: &AgentId) {
        let caches = self.caches.read().await;
        if let Some(cached) = caches.get(agent_id) {
            let mut cached = cached.write().await;
            cached.dirty = true;
        }
    }

    /// Get a snapshot for decision context
    pub async fn snapshot(&self, agent_id: &AgentId) -> Result<PersonalMemorySnapshot, PersonalMemoryError> {
        let cached = self.get_or_create(agent_id).await;
        let memory = cached.read().await.memory.clone();
        Ok(crate::mcp::personal_memory_to_snapshot(&memory))
    }

    /// Write a memory entry (updates cache, marks dirty)
    pub async fn write_entry<F>(&self, agent_id: &AgentId, f: F) -> Result<(), PersonalMemoryError>
    where
        F: FnOnce(&mut SchemaMemory) -> Result<(), PersonalMemoryError>,
    {
        let cached = self.get_or_create(agent_id).await;
        {
            let mut cached = cached.write().await;
            f(&mut cached.memory)?;
            cached.dirty = true;
        }
        // Auto-save periodically
        if cached.read().await.last_saved + 5000 < crate::now_ms() {
            self.save_agent(agent_id).await?;
        }
        Ok(())
    }
}

/// Parse a Markdown file with YAML frontmatter
fn parse_markdown_file<T: for<'de> Deserialize<'de>>(
    content: &str,
) -> Result<T, PersonalMemoryError> {
    // Split frontmatter and body
    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return Err(PersonalMemoryError::InvalidStructure("No frontmatter found".to_string()));
    }
    
    let frontmatter: serde_yaml::Value = serde_yaml::from_str(parts[1])?;
    let _body = parts[2].trim();
    
    // For now, we reconstruct from frontmatter only
    // Full body parsing would require custom parsers per type
    let json = serde_json::to_value(&frontmatter)?;
    serde_json::from_value(json).map_err(Into::into)
}

fn person_to_markdown(person: &PersonMemory) -> String {
    let _fm = serde_yaml::to_string(&MemoryFrontmatter {
        created_at: person.first_interaction,
        updated_at: person.last_interaction,
        version: 1,
        tags: person.tags.clone(),
        extra: HashMap::new(),
    }).unwrap_or_default();

    format!(
        "---\n{}---\n\n# {}\n\n**Agent ID**: {}\n\n**Trust**: {:.2}\n\n**Interactions**: {}\n\n**Summary**: {}\n\n**Traits**: {}\n\n**Tags**: {}\n\n**History**:\n{}\n",
        serde_yaml::to_string(&MemoryFrontmatter::default()).unwrap_or_default(),
        person.display_name.as_ref().unwrap_or(&person.agent_id),
        person.agent_id,
        person.trust_score,
        person.interaction_count,
        person.summary,
        person.notable_traits.join(", "),
        person.tags.join(", "),
        person.interaction_history.iter().map(|i| format!("- {} [{:?}]: {} (Δtrust: {:.2})", crate::format_ts(i.timestamp), i.type_, i.outcome, i.trust_delta)).collect::<Vec<_>>().join("\n")
    )
}

fn task_to_markdown(task: &TaskMemory) -> String {
    format!(
        "---\ncreated_at: {}\nupdated_at: {}\nversion: 1\ntags: [tasks]\n---\n\n# {}\n\n**Task ID**: {}\n\n**Role**: {:?} | **Status**: {:?} | **Reward**: {} Cr\n\n**Description**: {}\n\n**Teammates**: {}\n\n**Outcome**: {}\n\n**Evidence**: {}\n\n**Self-Rating**: {}/10\n\n**Lessons**: {}\n",
        task.started_at, crate::now_ms(),
        task.title, task.task_id,
        task.role, task.status, task.reward,
        task.description,
        task.teammates.join(", "),
        task.outcome,
        task.evidence_id.as_deref().unwrap_or("none"),
        task.self_rating.unwrap_or(0),
        task.lessons.join("; ")
    )
}

fn relationship_to_markdown(rel: &RelationshipMemory) -> String {
    format!(
        "---\ncreated_at: {}\nupdated_at: {}\nversion: 1\ntags: [relationships]\n---\n\n# {}\n\n**Type**: {:?} | **Strength**: {:.2} | **Trust**: {:.2} | **Respect**: {:.2} | **Reliability**: {:.2}\n\n**Shared Tasks**: {}\n\n**Successful**: {} | **Failed**: {}\n\n**Notes**: {}\n",
        rel.started_at, rel.last_updated,
        rel.agent_id,
        rel.relationship_type, rel.strength, rel.trust, rel.respect, rel.reliability,
        rel.shared_tasks.join(", "),
        rel.successful_collaborations, rel.failed_collaborations,
        rel.notes
    )
}

fn experience_to_markdown(exp: &ExperienceEntry) -> String {
    format!(
        "---\ncreated_at: {}\nupdated_at: {}\nversion: 1\ntags: [{}]\n---\n\n# {}\n\n**Type**: {:?} | **Impact**: {:.2} | **Agents**: {}\n\n**Summary**: {}\n\n**Detail**: {}\n\n**Outcome**: {}\n\n**Evidence**: {}\n\n**Tags**: {}\n",
        exp.timestamp, crate::now_ms(),
        exp.tags.join(", "),
        exp.id, exp.type_, exp.emotional_impact, exp.involved_agents.join(", "),
        exp.summary, exp.detail, exp.outcome,
        exp.evidence_ids.join(", "),
        exp.tags.join(", ")
    )
}

fn decision_to_markdown(dec: &DecisionEntry) -> String {
    format!(
        "---\ncreated_at: {}\nupdated_at: {}\nversion: 1\ntags: [{}]\n---\n\n# {}\n\n**Context**: {}\n\n**Options Considered**:\n{}\n\n**Choice**: {}\n\n**Rationale**: {}\n\n**Consequence**: {}\n\n**Confidence**: {:.0}% | **Hindsight**: {}\n\n**Evidence**: {}\n\n**Tags**: {}\n",
        dec.timestamp, crate::now_ms(),
        dec.tags.join(", "),
        dec.id, dec.context,
        dec.options_considered.iter().map(|o| format!("  - {}: {} (risk: {:.0}%) — rejected: {}", o.action, o.expected_outcome, o.risk * 100.0, o.rejected_reason)).collect::<Vec<_>>().join("\n"),
        dec.choice, dec.rationale, dec.consequence,
        dec.confidence * 100.0,
        dec.hindsight.as_deref().unwrap_or("(none)"),
        dec.evidence_ids.join(", "),
        dec.tags.join(", ")
    )
}

fn lesson_to_markdown(lesson: &LessonEntry) -> String {
    format!(
        "---\ncreated_at: {}\nupdated_at: {}\nversion: 1\ntags: [{}]\n---\n\n# {}\n\n**Content**: {}\n\n**Applies To**: {}\n\n**Source**: {}\n\n**Confidence**: {:.0}% | **Usage**: {}\n\n**Tags**: {}\n",
        lesson.created_at, crate::now_ms(),
        lesson.tags.join(", "),
        lesson.title,
        lesson.content,
        lesson.applies_to.join(", "),
        lesson.source_experiences.join(", "),
        lesson.confidence * 100.0,
        lesson.usage_count,
        lesson.tags.join(", ")
    )
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

