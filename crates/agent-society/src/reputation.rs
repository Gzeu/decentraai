//! Social reputation: signals derived from verified outcomes and relationships

use crate::state::{ReputationEvent, ReputationEventType};
use crate::{AgentId, Tick};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Social reputation for an agent, scoped by capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialReputation {
    pub agent_id: AgentId,
    pub capability: Option<String>, // None = general
    pub signals: BTreeMap<ReputationSignal, SignalScore>,
    pub overall: f32, // -1.0 to 1.0
    pub updated_at: Tick,
    pub sample_count: u64,
}

impl SocialReputation {
    pub fn new(agent_id: AgentId, capability: Option<String>, tick: Tick) -> Self {
        Self {
            agent_id,
            capability,
            signals: BTreeMap::new(),
            overall: 0.0,
            updated_at: tick,
            sample_count: 0,
        }
    }

    /// Update from a reputation event
    pub fn apply_event(&mut self, event: &ReputationEvent) {
        let signal = match event.event_type {
            ReputationEventType::TaskCompleted => ReputationSignal::Reliability,
            ReputationEventType::TaskFailed => ReputationSignal::Reliability,
            ReputationEventType::QualityHigh => ReputationSignal::Quality,
            ReputationEventType::QualityLow => ReputationSignal::Quality,
            ReputationEventType::SlaMet => ReputationSignal::Latency,
            ReputationEventType::SlaMissed => ReputationSignal::Latency,
            ReputationEventType::ContributionVerified => ReputationSignal::Contribution,
            ReputationEventType::ContributionMissing => ReputationSignal::Contribution,
            ReputationEventType::ProposalAccepted => ReputationSignal::Collaboration,
            ReputationEventType::ProposalRejected => ReputationSignal::Collaboration,
            ReputationEventType::BidAccepted => ReputationSignal::MarketCompetence,
            ReputationEventType::BidRejected => ReputationSignal::MarketCompetence,
        };

        let score = self
            .signals
            .entry(signal)
            .or_insert(SignalScore::new(0.0, 0, event.tick));
        score.apply_delta(event.delta, event.tick);

        self.recalculate_overall();
        self.updated_at = event.tick;
        self.sample_count += 1;
    }

    fn recalculate_overall(&mut self) {
        let weights = Self::default_weights();
        let mut weighted_sum = 0.0;
        let mut total_weight = 0.0;

        for (signal, score) in &self.signals {
            if score.is_meaningful() {
                let weight = weights.get(signal).copied().unwrap_or(0.1);
                weighted_sum += score.value * weight;
                total_weight += weight;
            }
        }

        self.overall = if total_weight > 0.0 {
            weighted_sum / total_weight
        } else {
            0.0
        };
    }

    fn default_weights() -> BTreeMap<ReputationSignal, f32> {
        [
            (ReputationSignal::Reliability, 0.30),
            (ReputationSignal::Quality, 0.25),
            (ReputationSignal::Latency, 0.15),
            (ReputationSignal::Contribution, 0.15),
            (ReputationSignal::Collaboration, 0.10),
            (ReputationSignal::MarketCompetence, 0.05),
        ]
        .into_iter()
        .collect()
    }

    /// Get score for a specific signal
    pub fn signal_score(&self, signal: ReputationSignal) -> Option<f32> {
        self.signals
            .get(&signal)
            .filter(|s| s.is_meaningful())
            .map(|s| s.value)
    }
}

/// Reputation signal types
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReputationSignal {
    Reliability,      // Task completion rate
    Quality,          // Output quality
    Latency,          // SLA adherence
    Contribution,     // Verified contribution vs planned
    Collaboration,    // Proposal/team success
    MarketCompetence, // Bidding/pricing skill
}

/// Signal score with samples
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalScore {
    pub value: f32, // -1.0 to 1.0
    pub samples: u64,
    pub updated_at: Tick,
}

impl SignalScore {
    pub fn new(value: f32, samples: u64, updated_at: Tick) -> Self {
        Self {
            value: value.clamp(-1.0, 1.0),
            samples,
            updated_at,
        }
    }

    pub fn apply_delta(&mut self, delta: f32, tick: Tick) {
        // EMA-style update
        let alpha = if self.samples == 0 { 1.0 } else { 0.3 };
        self.value = (self.value * (1.0 - alpha) + delta.clamp(-1.0, 1.0) * alpha).clamp(-1.0, 1.0);
        self.samples += 1;
        self.updated_at = tick;
    }

    pub fn is_meaningful(&self) -> bool {
        self.samples >= 1
    }
}

/// Reputation store for all agents
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReputationStore {
    pub reputations: HashMap<AgentId, HashMap<Option<String>, SocialReputation>>,
    pub min_samples_for_meaningful: u64,
}

impl ReputationStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, agent: &AgentId, capability: Option<&str>) -> Option<&SocialReputation> {
        self.reputations
            .get(agent)?
            .get(&capability.map(|s| s.to_string()))
    }

    pub fn get_or_create(
        &mut self,
        agent: AgentId,
        capability: Option<String>,
        tick: Tick,
    ) -> &mut SocialReputation {
        let cap_key = capability.clone();
        self.reputations
            .entry(agent.clone())
            .or_default()
            .entry(cap_key)
            .or_insert_with(|| SocialReputation::new(agent, capability, tick))
    }

    pub fn apply_event(&mut self, event: &ReputationEvent) {
        let cap = None; // For now, general reputation
        let rep = self.get_or_create(event.agent_id.clone(), cap.clone(), event.tick);
        rep.apply_event(event);
    }

    /// Get all reputations for an agent
    pub fn for_agent(&self, agent: &AgentId) -> Vec<&SocialReputation> {
        self.reputations
            .get(agent)
            .map(|m| m.values().collect())
            .unwrap_or_default()
    }

    /// Get top agents by overall reputation for a capability
    pub fn top_agents(
        &self,
        capability: Option<&str>,
        limit: usize,
    ) -> Vec<(AgentId, &SocialReputation)> {
        let cap_key = capability.map(|s| s.to_string());
        let mut agents: Vec<_> = self
            .reputations
            .iter()
            .filter_map(|(id, caps)| caps.get(&cap_key).map(|r| (id.clone(), r)))
            .filter(|(_, r)| r.sample_count > 0)
            .collect();

        agents.sort_by(|a, b| b.1.overall.partial_cmp(&a.1.overall).unwrap());
        agents.into_iter().take(limit).collect()
    }
}
