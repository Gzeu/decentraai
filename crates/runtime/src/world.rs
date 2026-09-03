//! Open World — persistent world layer for DecentraAI.
//!
//! WorldState is **NOT** a second source of truth. It is a thin
//! projection persisted as `db/world.json` that stores zones, locations,
//! entities, marketplace listings, and world events.
//! All ledger/quota/placement/Hub/Society/M18 remain canonical.
//!
//! Architecture:
//! - Zones group locations (Central Hub, Research District, Marketplace, etc.)
//! - Locations host services and marketplaces
//! - Entities are agents/NPCs with position and state
//! - Movement is first-class: agents move between locations
//! - Services at locations are wired to M18 contracts
//! - Marketplace listings are wired to M18 escrow

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use decentraai_economy::multiversx_tx::{Mx8004TxBuilder, UnsignedTxIntent};
use decentraai_economy::signer::canonical_sign_payload;

// ─── Zones & Locations ───────────────────────────────────────────────

/// A zone groups related locations (like a region on a map).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldZone {
    pub id: String,
    pub label: String,
    pub description: String,
    /// Background color for the UI (hex).
    #[serde(default = "default_zone_color")]
    pub color: String,
    /// Whether this zone is discovered by default.
    #[serde(default = "default_true")]
    pub discovered: bool,
}

fn default_zone_color() -> String {
    "#1a2540".to_string()
}
fn default_true() -> bool {
    true
}

/// A location within a zone — a place agents can visit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldLocation {
    pub id: String,
    pub zone_id: String,
    pub label: String,
    pub description: String,
    /// Services offered at this location (capability → price).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub services: HashMap<String, u64>,
    /// Whether this location has a marketplace.
    #[serde(default)]
    pub marketplace: bool,
    /// Maximum agents that can be here simultaneously.
    #[serde(default = "default_capacity")]
    pub capacity: usize,
}

fn default_capacity() -> usize {
    50
}

// ─── Entities ────────────────────────────────────────────────────────

/// State of an entity in the world.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EntityState {
    #[default]
    Idle,
    Moving,
    Working,
    Trading,
    Resting,
    Exploring,
}

/// An entity (agent or NPC) in the world.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldEntity {
    /// Unique entity id (matches agent_id or npc id).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Entity type.
    #[serde(rename = "type")]
    pub entity_type: String,
    /// Current zone_id.
    pub zone_id: String,
    /// Current location_id.
    pub location_id: String,
    /// Current state.
    #[serde(default)]
    pub state: EntityState,
    /// Capabilities this entity offers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// Capabilities this entity needs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    /// Wallet address (for M18 contracts).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub wallet: String,
    /// Reputation score (projection from Society).
    #[serde(default)]
    pub reputation: f32,
    /// Credits available.
    #[serde(default)]
    pub credits: u64,
    /// What the entity is doing (free-text status).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub activity: String,
    /// When the entity last moved (tick).
    #[serde(default)]
    pub last_move_tick: u64,
    /// Inventory of items held.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inventory: Vec<WorldItem>,
}

// ─── Items & Marketplace ─────────────────────────────────────────────

/// An item that can be traded or used in the world.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldItem {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub item_type: String,
    /// Price in credits (0 = not for sale).
    #[serde(default)]
    pub price: u64,
    /// Description.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// A marketplace listing — an item or service for sale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketListing {
    pub id: String,
    pub seller_id: String,
    pub item: WorldItem,
    /// Location where this listing is active.
    pub location_id: String,
    /// When it was listed (tick).
    pub listed_tick: u64,
    /// Whether it's still available.
    #[serde(default = "default_true")]
    pub active: bool,
}

/// A vendor's two-item catalog: (item_id, name, price) per tick parity.
/// Module scope (not inside `impl`) so the vendor tick can name it.
struct VendorOffer {
    npc_id: &'static str,
    location_id: &'static str,
    even: (&'static str, &'static str, u64),
    odd: (&'static str, &'static str, u64),
}

// ─── World Events ────────────────────────────────────────────────────

/// A real-time event in the world.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldEvent {
    pub tick: u64,
    pub kind: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<String>,
}

// ─── On-Chain Settlement ─────────────────────────────────────────────

/// Status of an on-chain settlement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SettlementStatus {
    /// Proof generated, awaiting operator signing.
    Pending,
    /// Transaction submitted to MultiversX testnet.
    Submitted,
    /// Transaction confirmed on-chain.
    Confirmed,
    /// Settlement failed.
    Failed,
}

/// An on-chain proof anchoring an economic action to MultiversX testnet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnChainProof {
    /// Unique proof id.
    pub id: String,
    /// What was settled (quest completion, service purchase, trade, etc.).
    pub action_type: String,
    /// Description of the settled action.
    pub description: String,
    /// Entity that triggered the settlement.
    pub entity_id: String,
    /// Amount settled in credits/micro-CU.
    pub amount: u64,
    /// BLAKE3 evidence hash (raw bytes, hex-encoded).
    pub evidence_hash: String,
    /// MultiversX testnet tx data (unsigned intent).
    pub tx_data: String,
    /// Tx hash after submission (empty until submitted).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tx_hash: String,
    /// Operator wallet that signed+broadcast the tx (empty until submitted).
    /// Persisted so restart/recovery can verify sender consistency.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sender: String,
    /// Chain nonce the tx was broadcast with (for restart recovery: pending
    /// txs occupy their nonces, the tracker must not reissue them).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    /// Current settlement status.
    pub status: SettlementStatus,
    /// When the proof was generated (tick).
    pub created_tick: u64,
    /// When the tx was submitted (0 = not yet).
    #[serde(default)]
    pub submitted_tick: u64,
    /// When the tx was confirmed (0 = not yet).
    #[serde(default)]
    pub confirmed_tick: u64,
    /// MultiversX testnet network.
    #[serde(default = "default_network")]
    pub network: String,
}

fn default_network() -> String {
    "multiversx-testnet".to_string()
}

// ─── Quests ──────────────────────────────────────────────────────────

/// Type of quest objective.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum QuestObjective {
    /// Move to a specific location.
    MoveTo { location_id: String },
    /// Purchase a service at a location.
    BuyService { location_id: String, service: String },
    /// List an item on the marketplace.
    ListItem { location_id: String },
    /// Buy an item from the marketplace.
    BuyItem { location_id: String },
    /// Trade with another entity (buy or sell).
    Trade { location_id: String },
    /// Visit any location in a zone.
    VisitZone { zone_id: String },
    /// Accumulate a certain amount of credits.
    EarnCredits { amount: u64 },
    /// Have a certain reputation score.
    ReachReputation { score: f32 },
}

/// Status of a quest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuestStatus {
    /// Available for acceptance.
    Available,
    /// Accepted by an agent, in progress.
    Active,
    /// All objectives completed, ready to turn in.
    Ready,
    /// Completed and rewarded.
    Completed,
    /// Failed or expired.
    Failed,
}

/// Reward for completing a quest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestReward {
    /// Credits awarded.
    #[serde(default)]
    pub credits: u64,
    /// Reputation bonus.
    #[serde(default)]
    pub reputation: f32,
    /// Items awarded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<WorldItem>,
    /// Access to a zone unlock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_unlock: Option<String>,
}

/// A quest in the world.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quest {
    pub id: String,
    pub title: String,
    pub description: String,
    /// Who gave the quest (NPC id).
    pub giver_id: String,
    /// Objectives to complete (all must be done).
    pub objectives: Vec<QuestObjective>,
    /// Current progress per objective (same length as objectives).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub progress: Vec<bool>,
    /// Reward for completing.
    pub reward: QuestReward,
    /// Current status.
    pub status: QuestStatus,
    /// Who accepted this quest (agent id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_by: Option<String>,
    /// When the quest was generated (tick).
    pub created_tick: u64,
    /// When the quest was accepted (tick).
    #[serde(default)]
    pub accepted_tick: u64,
    /// Tick deadline (0 = no deadline).
    #[serde(default)]
    pub deadline_tick: u64,
    /// Required reputation to accept.
    #[serde(default)]
    pub required_reputation: f32,
    /// Required capabilities to accept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_capabilities: Vec<String>,
}

// ─── WorldState ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldRoom {
    pub id: String,
    pub label: String,
    pub capability_filter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldAgent {
    pub agent_id: String,
    pub key_id: String,
    pub account: String,
    pub declared_capabilities: Vec<String>,
    /// Capabilities this agent needs from others (buy-side signals for economic tick).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    pub room_id: String,
    pub joined_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldState {
    pub world_id: String,
    pub mission_task_id: Option<String>,
    /// Legacy rooms (kept for backward compat).
    pub rooms: Vec<WorldRoom>,
    /// Legacy agents (kept for backward compat).
    pub agents: Vec<WorldAgent>,
    pub tick: u64,

    // ─── Open World fields ───────────────────────────────────────────
    /// Zones in the world.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<WorldZone>,
    /// All locations across all zones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<WorldLocation>,
    /// All entities in the world (agents + NPCs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<WorldEntity>,
    /// Marketplace listings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listings: Vec<MarketListing>,
    /// Recent world events (bounded).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<WorldEvent>,
    /// Active and available quests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quests: Vec<Quest>,
    /// On-chain settlement proofs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proofs: Vec<OnChainProof>,
    /// Base service prices by "location_id/service" (first-seen price sticks;
    /// the live `locations[].services` map is the base source of truth).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub service_base_prices: HashMap<String, u64>,
    /// Demand counters by "location_id/service": +1 per sale, -1 per tick.
    /// Drives the dynamic price — hot services cost more, quiet ones cool.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub service_demand: HashMap<String, u64>,
}

impl Default for WorldState {
    fn default() -> Self {
        Self {
            world_id: "decentraai-open-world".to_string(),
            mission_task_id: None,
            rooms: vec![
                WorldRoom {
                    id: "research-lab".to_string(),
                    label: "Research Lab".to_string(),
                    capability_filter: "research".to_string(),
                },
                WorldRoom {
                    id: "coding-lab".to_string(),
                    label: "Coding Lab".to_string(),
                    capability_filter: "coding".to_string(),
                },
            ],
            agents: vec![],
            tick: 0,
            zones: default_zones(),
            locations: default_locations(),
            entities: vec![],
            listings: vec![],
            events: vec![],
            quests: vec![],
            proofs: vec![],
            service_base_prices: HashMap::new(),
            service_demand: HashMap::new(),
        }
    }
}

/// Default zones for the Open World.
fn default_zones() -> Vec<WorldZone> {
    vec![
        WorldZone {
            id: "central-hub".to_string(),
            label: "Central Hub".to_string(),
            description: "The heart of DecentraAI. Where agents gather, trade, and find work.".to_string(),
            color: "#22d3ee".to_string(),
            discovered: true,
        },
        WorldZone {
            id: "research-district".to_string(),
            label: "Research District".to_string(),
            description: "Labs and research facilities. Agents come here for knowledge work.".to_string(),
            color: "#a78bfa".to_string(),
            discovered: true,
        },
        WorldZone {
            id: "marketplace".to_string(),
            label: "Marketplace".to_string(),
            description: "The bustling marketplace. Buy, sell, and trade services and artifacts.".to_string(),
            color: "#34d399".to_string(),
            discovered: true,
        },
        WorldZone {
            id: "forge".to_string(),
            label: "The Forge".to_string(),
            description: "Where models are trained and fine-tuned. Heavy compute zone.".to_string(),
            color: "#f59e0b".to_string(),
            discovered: true,
        },
        WorldZone {
            id: "deep-forest".to_string(),
            label: "Deep Forest".to_string(),
            description: "Wild territory. Rare capabilities and hidden opportunities.".to_string(),
            color: "#10b981".to_string(),
            discovered: false,
        },
    ]
}

/// Default locations for the Open World.
fn default_locations() -> Vec<WorldLocation> {
    let mut services_inference = HashMap::new();
    services_inference.insert("inference".to_string(), 5);
    services_inference.insert("embeddings".to_string(), 3);

    let mut services_ocr = HashMap::new();
    services_ocr.insert("ocr".to_string(), 10);
    services_ocr.insert("stt".to_string(), 15);

    let mut services_research = HashMap::new();
    services_research.insert("research".to_string(), 20);

    let mut services_coding = HashMap::new();
    services_coding.insert("coding".to_string(), 25);

    let mut services_translation = HashMap::new();
    services_translation.insert("translation".to_string(), 8);

    vec![
        // Central Hub
        WorldLocation {
            id: "hub-plaza".to_string(),
            zone_id: "central-hub".to_string(),
            label: "Plaza".to_string(),
            description: "The central gathering place. Agents meet here and find opportunities.".to_string(),
            services: HashMap::new(),
            marketplace: false,
            capacity: 100,
        },
        WorldLocation {
            id: "hub-quest-board".to_string(),
            zone_id: "central-hub".to_string(),
            label: "Quest Board".to_string(),
            description: "Missions and tasks posted here. Pick up work and earn credits.".to_string(),
            services: HashMap::new(),
            marketplace: false,
            capacity: 50,
        },
        WorldLocation {
            id: "hub-inn".to_string(),
            zone_id: "central-hub".to_string(),
            label: "The Inn".to_string(),
            description: "Rest and recover. Agents regroup here between tasks.".to_string(),
            services: HashMap::new(),
            marketplace: false,
            capacity: 30,
        },
        // Research District
        WorldLocation {
            id: "research-lab-main".to_string(),
            zone_id: "research-district".to_string(),
            label: "Main Research Lab".to_string(),
            description: "Primary research facility. Inference and embeddings available.".to_string(),
            services: services_inference,
            marketplace: false,
            capacity: 20,
        },
        WorldLocation {
            id: "research-archive".to_string(),
            zone_id: "research-district".to_string(),
            label: "Knowledge Archive".to_string(),
            description: "Vast repository of knowledge. Research services for hire.".to_string(),
            services: services_research,
            marketplace: false,
            capacity: 15,
        },
        // Marketplace
        WorldLocation {
            id: "market-bazaar".to_string(),
            zone_id: "marketplace".to_string(),
            label: "The Bazaar".to_string(),
            description: "Main trading floor. List services, buy capabilities, find partners.".to_string(),
            services: services_translation,
            marketplace: true,
            capacity: 80,
        },
        WorldLocation {
            id: "market-auction".to_string(),
            zone_id: "marketplace".to_string(),
            label: "Auction House".to_string(),
            description: "Competitive bidding for premium services and rare artifacts.".to_string(),
            services: HashMap::new(),
            marketplace: true,
            capacity: 40,
        },
        // The Forge
        WorldLocation {
            id: "forge-workshop".to_string(),
            zone_id: "forge".to_string(),
            label: "Model Workshop".to_string(),
            description: "Fine-tune and optimize models. Heavy compute zone.".to_string(),
            services: services_coding,
            marketplace: false,
            capacity: 10,
        },
        WorldLocation {
            id: "forge-testing".to_string(),
            zone_id: "forge".to_string(),
            label: "Testing Grounds".to_string(),
            description: "OCR, STT, and other tool testing facilities.".to_string(),
            services: services_ocr,
            marketplace: false,
            capacity: 15,
        },
        // Deep Forest
        WorldLocation {
            id: "forest-clearing".to_string(),
            zone_id: "deep-forest".to_string(),
            label: "Hidden Clearing".to_string(),
            description: "A mysterious clearing. Rare capabilities appear here.".to_string(),
            services: HashMap::new(),
            marketplace: false,
            capacity: 5,
        },
    ]
}

impl WorldState {
    pub fn advance_tick(&mut self) {
        self.tick += 1;
    }

    /// Migrate legacy rooms/agents into Open World entities (idempotent).
    pub fn migrate_legacy(&mut self) {
        // Convert legacy agents to entities if not already present
        let legacy_agents: Vec<WorldAgent> = self.agents.drain(..).collect();
        for agent in legacy_agents {
            if !self.entities.iter().any(|e| e.id == agent.agent_id) {
                let zone_id = self.zone_for_capabilities(&agent.declared_capabilities);
                let location_id = self.location_for_zone(&zone_id);
                self.entities.push(WorldEntity {
                    id: agent.agent_id.clone(),
                    name: agent.agent_id.clone(),
                    entity_type: "agent".to_string(),
                    zone_id: zone_id.clone(),
                    location_id: location_id.clone(),
                    state: EntityState::Idle,
                    capabilities: agent.declared_capabilities.clone(),
                    needs: agent.needs.clone(),
                    wallet: agent.account.clone(),
                    reputation: 0.0,
                    credits: 0,
                    activity: String::new(),
                    last_move_tick: self.tick,
                    inventory: vec![],
                });
            }
        }
    }

    /// Find the best zone for given capabilities.
    pub fn zone_for_capabilities(&self, caps: &[String]) -> String {
        let lower: Vec<String> = caps.iter().map(|c| c.to_lowercase()).collect();
        // Match to zones based on capability keywords
        for zone in &self.zones {
            let zid = zone.id.to_lowercase();
            if lower.iter().any(|c| {
                (c.contains("research") && zid.contains("research"))
                    || (c.contains("coding") && zid.contains("forge"))
                    || (c.contains("ocr") || c.contains("stt") && zid.contains("forge"))
            }) {
                return zone.id.clone();
            }
        }
        "central-hub".to_string()
    }

    /// Find the best location within a zone for given capabilities.
    pub fn location_for_zone(&self, zone_id: &str) -> String {
        self.locations
            .iter()
            .find(|l| l.zone_id == zone_id)
            .map(|l| l.id.clone())
            .unwrap_or_else(|| "hub-plaza".to_string())
    }

    pub fn room_for_capabilities(&self, caps: &[String]) -> String {
        let lower: Vec<String> = caps.iter().map(|c| c.to_lowercase()).collect();
        for room in &self.rooms {
            let f = room.capability_filter.to_lowercase();
            if lower
                .iter()
                .any(|c| c.contains(&f) || f.contains(c.as_str()))
            {
                return room.id.clone();
            }
        }
        self.rooms
            .first()
            .map(|r| r.id.clone())
            .unwrap_or_else(|| "research-lab".to_string())
    }

    /// Move an entity to a new location.
    pub fn move_entity(
        &mut self,
        entity_id: &str,
        location_id: &str,
    ) -> Result<String, String> {
        let location = self
            .locations
            .iter()
            .find(|l| l.id == location_id)
            .ok_or_else(|| format!("location '{}' not found", location_id))?;

        let zone_id = location.zone_id.clone();
        let capacity = location.capacity;
        let current_count = self
            .entities
            .iter()
            .filter(|e| e.location_id == location_id && e.id != entity_id)
            .count();

        if current_count >= capacity {
            return Err(format!(
                "location '{}' is full ({}/{})",
                location_id, current_count, capacity
            ));
        }

        let entity = self
            .entities
            .iter_mut()
            .find(|e| e.id == entity_id)
            .ok_or_else(|| format!("entity '{}' not found", entity_id))?;

        let old_location = entity.location_id.clone();
        entity.zone_id = zone_id;
        entity.location_id = location_id.to_string();
        entity.state = EntityState::Idle;
        entity.last_move_tick = self.tick;

        // Record event
        let event = WorldEvent {
            tick: self.tick,
            kind: "entity_moved".to_string(),
            detail: format!(
                "{} moved from {} to {}",
                entity.name, old_location, location_id
            ),
            entity_id: Some(entity_id.to_string()),
            location_id: Some(location_id.to_string()),
            evidence_id: None,
        };
        self.events.push(event);
        if self.events.len() > 200 {
            self.events.drain(0..50);
        }

        Ok(location_id.to_string())
    }

    /// List an item on the marketplace.
    pub fn list_item(
        &mut self,
        seller_id: &str,
        item: WorldItem,
        location_id: &str,
    ) -> Result<String, String> {
        // Verify seller is at this location
        let seller = self
            .entities
            .iter()
            .find(|e| e.id == seller_id)
            .ok_or_else(|| format!("seller '{}' not found", seller_id))?;

        if seller.location_id != location_id {
            return Err("seller is not at this location".to_string());
        }

        // Verify location has a marketplace
        let location = self
            .locations
            .iter()
            .find(|l| l.id == location_id)
            .ok_or_else(|| format!("location '{}' not found", location_id))?;

        if !location.marketplace {
            return Err(format!("location '{}' does not have a marketplace", location_id));
        }

        let listing_id = format!(
            "ml-{}-{}",
            &item.id[..item.id.len().min(12)],
            self.tick
        );

        let event = WorldEvent {
            tick: self.tick,
            kind: "item_listed".to_string(),
            detail: format!("{} listed '{}' for {}Cr at {}", seller_id, item.name, item.price, location_id),
            entity_id: Some(seller_id.to_string()),
            location_id: Some(location_id.to_string()),
            evidence_id: None,
        };
        self.events.push(event);

        self.listings.push(MarketListing {
            id: listing_id.clone(),
            seller_id: seller_id.to_string(),
            item,
            location_id: location_id.to_string(),
            listed_tick: self.tick,
            active: true,
        });

        if self.events.len() > 200 {
            self.events.drain(0..50);
        }

        Ok(listing_id)
    }

    /// Buy an item from the marketplace.
    pub fn buy_item(
        &mut self,
        buyer_id: &str,
        listing_id: &str,
    ) -> Result<(WorldItem, String), String> {
        let listing = self
            .listings
            .iter_mut()
            .find(|l| l.id == listing_id && l.active)
            .ok_or_else(|| format!("listing '{}' not found or inactive", listing_id))?;

        let buyer = self
            .entities
            .iter()
            .find(|e| e.id == buyer_id)
            .ok_or_else(|| format!("buyer '{}' not found", buyer_id))?;

        if buyer.location_id != listing.location_id {
            return Err("buyer is not at this location".to_string());
        }

        if buyer.credits < listing.item.price {
            return Err(format!(
                "insufficient credits: {} < {}",
                buyer.credits, listing.item.price
            ));
        }

        // Transfer
        let item = listing.item.clone();
        let seller_id = listing.seller_id.clone();
        let price = listing.item.price;
        listing.active = false;

        // Deduct from buyer, credit to seller
        if let Some(b) = self.entities.iter_mut().find(|e| e.id == buyer_id) {
            b.credits = b.credits.saturating_sub(price);
            b.inventory.push(item.clone());
        }
        if let Some(s) = self.entities.iter_mut().find(|e| e.id == seller_id) {
            s.credits += price;
        }

        let event = WorldEvent {
            tick: self.tick,
            kind: "item_sold".to_string(),
            detail: format!(
                "{} bought '{}' from {} for {}Cr at {}",
                buyer_id, item.name, seller_id, price, listing.location_id
            ),
            entity_id: Some(buyer_id.to_string()),
            location_id: Some(listing.location_id.clone()),
            evidence_id: None,
        };
        self.events.push(event);
        if self.events.len() > 200 {
            self.events.drain(0..50);
        }

        Ok((item, seller_id))
    }

    /// Get entities at a specific location.
    pub fn entities_at(&self, location_id: &str) -> Vec<&WorldEntity> {
        self.entities
            .iter()
            .filter(|e| e.location_id == location_id)
            .collect()
    }

    /// Get active listings at a location.
    pub fn listings_at(&self, location_id: &str) -> Vec<&MarketListing> {
        self.listings
            .iter()
            .filter(|l| l.location_id == location_id && l.active)
            .collect()
    }

    /// Record a world event.
    pub fn record_event(&mut self, event: WorldEvent) {
        self.events.push(event);
        if self.events.len() > 200 {
            self.events.drain(0..50);
        }
    }

    /// Spawn NPCs at locations that offer services.
    /// Idempotent: only spawns if no NPC exists at that location yet.
    pub fn spawn_npcs(&mut self) {
        let npcs: Vec<(&str, &str, &str, Vec<&str>, u64)> = vec![
            // Central Hub
            ("npc-questkeeper", "Questkeeper", "hub-quest-board", vec!["quest-giver"], 0),
            ("npc-inkeeper", "Innkeeper", "hub-inn", vec!["rest"], 0),
            // Research District
            ("npc-librarian", "Librarian", "research-archive", vec!["research"], 0),
            ("npc-researcher", "Researcher", "research-lab-main", vec!["inference", "embeddings"], 0),
            // Marketplace
            ("npc-broker", "Broker", "market-bazaar", vec!["trade"], 0),
            ("npc-auctioneer", "Auctioneer", "market-auction", vec!["auction"], 0),
            // The Forge
            ("npc-smith", "Smith", "forge-workshop", vec!["coding"], 0),
            ("npc-tester", "Tester", "forge-testing", vec!["ocr", "stt"], 0),
            // Deep Forest
            ("npc-hermit", "Hermit", "forest-clearing", vec!["mystery"], 0),
        ];

        for (id, name, loc_id, caps, credits) in npcs {
            if !self.entities.iter().any(|e| e.id == id) {
                let zone_id = self
                    .locations
                    .iter()
                    .find(|l| l.id == loc_id)
                    .map(|l| l.zone_id.clone())
                    .unwrap_or_default();
                self.entities.push(WorldEntity {
                    id: id.to_string(),
                    name: name.to_string(),
                    entity_type: "npc".to_string(),
                    zone_id,
                    location_id: loc_id.to_string(),
                    state: EntityState::Idle,
                    capabilities: caps.iter().map(|s| s.to_string()).collect(),
                    needs: vec![],
                    wallet: format!("npc:{}", id),
                    reputation: 1.0,
                    credits,
                    activity: "tending shop".to_string(),
                    last_move_tick: self.tick,
                    inventory: vec![],
                });
            }
        }
    }

    /// Execute a world tick: NPCs act, agents auto-behave, world evolves.
    pub fn world_tick(&mut self) {
        self.advance_tick();

        // 0. Hot markets cool: demand decays every tick.
        self.decay_service_demand();

        // 0b. Vendor NPCs restock the marketplace on their own rhythm.
        self.npc_vendor_tick();

        // 1. Questkeeper generates quests
        self.questkeeper_tick();

        // 2. Agent autonomous behavior: agents with needs seek services
        let entity_ids: Vec<String> = self.entities.iter().map(|e| e.id.clone()).collect();
        for entity_id in entity_ids {
            let is_agent = self
                .entities
                .iter()
                .find(|e| e.id == entity_id)
                .map(|e| e.entity_type == "agent")
                .unwrap_or(false);
            if !is_agent {
                continue;
            }

            // Get agent needs and current location
            let (needs, current_loc, credits) = {
                let e = self.entities.iter().find(|e| e.id == entity_id).unwrap();
                (e.needs.clone(), e.location_id.clone(), e.credits)
            };

            if needs.is_empty() {
                continue;
            }

            // Find a location that offers a needed service (at its CURRENT
            // dynamic price — agents feel hot markets like anyone else).
            for needed in &needs {
                if let Some(target_loc) = self.locations.iter().find(|l| {
                    l.services.contains_key(needed)
                        && l.id != current_loc
                        && credits
                            >= self
                                .service_price(&l.id, needed)
                                .unwrap_or(u64::MAX)
                }) {
                    let target_label = target_loc.label.clone();
                    let target_id = target_loc.id.clone();
                    // Move agent to the service location
                    let _ = self.move_entity(&entity_id, &target_id);
                    // Set activity
                    if let Some(e) = self.entities.iter_mut().find(|e| e.id == entity_id) {
                        e.activity = format!("seeking {} at {}", needed, target_label);
                    }
                    break;
                }
            }
        }

        // 2b. Agent autonomy: fulfill needs, take and work quests, earn.
        // This is what makes the world ALIVE without API calls: agents buy
        // services where they stand, accept quests they qualify for, pursue
        // objectives (move / buy / list / trade), and collect rewards.
        // On-chain submission stays OUT of the tick (no network here):
        // every worthy sale leaves a Pending proof for the sweep path.
        self.agent_autonomy_tick();

        // 2c. Quest progress follows action, then Ready quests complete:
        // rewards pay out and auto-settle (Pending) in the same tick.
        self.check_quest_progress();
        let ready: Vec<String> = self
            .quests
            .iter()
            .filter(|q| q.status == QuestStatus::Ready)
            .map(|q| q.id.clone())
            .collect();
        for quest_id in ready {
            let _ = self.complete_quest(&quest_id);
        }

        // 3. NPC behavior: NPCs at service locations offer services
        for entity in &mut self.entities {
            if entity.entity_type == "npc" {
                // NPCs maintain their reputation
                entity.reputation = (entity.reputation + 0.01).min(5.0);
                // NPCs at service locations restock
                if let Some(loc) = self.locations.iter().find(|l| l.id == entity.location_id) {
                    if !loc.services.is_empty() {
                        entity.activity = format!(
                            "offering {}",
                            loc.services.keys().cloned().collect::<Vec<_>>().join(", ")
                        );
                    }
                }
            }
        }

        // 3. Record tick event periodically
        if self.tick % 10 == 0 {
            let agent_count = self.entities.iter().filter(|e| e.entity_type == "agent").count();
            let npc_count = self.entities.iter().filter(|e| e.entity_type == "npc").count();
            self.record_event(WorldEvent {
                tick: self.tick,
                kind: "world_tick".to_string(),
                detail: format!(
                    "tick {} — {} agents, {} NPCs, {} listings",
                    self.tick,
                    agent_count,
                    npc_count,
                    self.listings.iter().filter(|l| l.active).count()
                ),
                entity_id: None,
                location_id: None,
                evidence_id: None,
            });
        }
    }

    /// Agent autonomy: needs, quests, earning — one action per agent.
    ///
    /// This is what makes the world ALIVE without API calls. Each tick every
    /// agent, in order: (1) may develop a new need, (2) buys ONE needed
    /// service where it stands, (3) takes or works ONE quest objective.
    /// Sales leave Pending proofs (no network in tick) for the sweep path;
    /// quest completion pays rewards + auto-settles right here.
    fn agent_autonomy_tick(&mut self) {
        let ids: Vec<String> = self
            .entities
            .iter()
            .filter(|e| e.entity_type == "agent")
            .map(|e| e.id.clone())
            .collect();
        for id in ids {
            self.agent_regenerate_need(&id);
            self.agent_fulfill_need(&id);
            self.agent_quest_tick(&id);
        }
    }

    /// Services agents can develop needs for (all real priced offers).
    const NEED_ROTATION: &'static [&'static str] = &[
        "ocr",
        "inference",
        "translation",
        "research",
        "embeddings",
        "coding",
        "stt",
    ];

    /// Idle agents develop new needs over time, so the world never runs dry:
    /// no needs + no active quest + rhythm tick → next need in rotation
    /// (only for services actually offered somewhere).
    fn agent_regenerate_need(&mut self, agent_id: &str) {
        if self.tick % 7 != 0 {
            return;
        }
        let (needs_empty, has_active, slot) = match self
            .entities
            .iter()
            .find(|e| e.id == agent_id)
        {
            Some(e) => (
                e.needs.is_empty(),
                self.quests.iter().any(|q| {
                    q.status == QuestStatus::Active && q.accepted_by.as_deref() == Some(agent_id)
                }),
                e.id.len(),
            ),
            None => return,
        };
        if !needs_empty || has_active {
            return;
        }
        let pick = Self::NEED_ROTATION[(self.tick as usize / 7 + slot) % Self::NEED_ROTATION.len()];
        let offered = self
            .locations
            .iter()
            .any(|l| l.services.contains_key(pick));
        if !offered {
            return;
        }
        if let Some(e) = self.entities.iter_mut().find(|e| e.id == agent_id) {
            e.needs.push(pick.to_string());
            e.activity = format!("needs {}", pick);
        }
    }

    /// Buy ONE needed service where the agent stands (dynamic price).
    /// Fulfilled needs clear; worthy sales leave a Pending proof.
    fn agent_fulfill_need(&mut self, agent_id: &str) {
        let (loc, needs) = match self.entities.iter().find(|e| e.id == agent_id) {
            Some(e) => (e.location_id.clone(), e.needs.clone()),
            None => return,
        };
        let Some(needed) = needs.into_iter().next() else {
            return;
        };
        let affordable = self
            .entities
            .iter()
            .find(|e| e.id == agent_id)
            .map(|e| {
                self.service_price(&loc, &needed)
                    .map(|p| e.credits >= p)
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !affordable {
            return;
        }
        if let Ok((_, provider, price)) = self.buy_service(agent_id, &loc, &needed) {
            if let Some(e) = self.entities.iter_mut().find(|e| e.id == agent_id) {
                e.needs.retain(|n| n != &needed);
            }
            let desc = format!("{agent_id} used {needed} (need fulfilled)");
            self.trade_settlement_proof("service_sale", &desc, &provider, price);
        }
    }

    /// Quest life: take an available quest, or work the first open objective
    /// of the active one. One action per tick: move, buy, list, or trade.
    fn agent_quest_tick(&mut self, agent_id: &str) {
        let active_id: Option<String> = self
            .quests
            .iter()
            .find(|q| {
                q.status == QuestStatus::Active && q.accepted_by.as_deref() == Some(agent_id)
            })
            .map(|q| q.id.clone());
        let quest_id = match active_id {
            Some(id) => id,
            None => {
                // Oldest available quest the agent qualifies for.
                let mut avail: Vec<String> = self
                    .quests
                    .iter()
                    .filter(|q| q.status == QuestStatus::Available)
                    .map(|q| q.id.clone())
                    .collect();
                avail.sort();
                let mut took = None;
                for qid in avail {
                    if self.accept_quest(&qid, agent_id).is_ok() {
                        took = Some(qid);
                        break;
                    }
                }
                match took {
                    Some(id) => id,
                    None => return,
                }
            }
        };

        let (objectives, progress) = match self.quests.iter().find(|q| q.id == quest_id) {
            Some(q) => (q.objectives.clone(), q.progress.clone()),
            None => return,
        };
        let open = objectives
            .iter()
            .zip(progress.iter())
            .find(|(_, done)| !**done)
            .map(|(obj, _)| obj.clone());
        let Some(obj) = open else { return };
        match obj {
            QuestObjective::MoveTo { location_id } => {
                self.agent_goto(agent_id, &location_id);
            }
            QuestObjective::BuyService {
                location_id,
                service,
            } => {
                if self.agent_at(agent_id, &location_id) {
                    if let Ok((_, provider, price)) =
                        self.buy_service(agent_id, &location_id, &service)
                    {
                        let desc = format!("{agent_id} completed quest service {service}");
                        self.trade_settlement_proof("service_sale", &desc, &provider, price);
                    }
                } else {
                    self.agent_goto(agent_id, &location_id);
                }
            }
            QuestObjective::ListItem { location_id } => {
                if self.agent_at(agent_id, &location_id) {
                    let item = WorldItem {
                        id: format!("craft-{agent_id}-{}", self.tick),
                        name: format!("{agent_id} Crafts"),
                        item_type: "artifact".to_string(),
                        price: 8,
                        description: format!("Handmade goods by {agent_id}"),
                    };
                    let _ = self.list_item(agent_id, item, &location_id);
                } else {
                    self.agent_goto(agent_id, &location_id);
                }
            }
            QuestObjective::BuyItem { location_id } => {
                if self.agent_at(agent_id, &location_id) {
                    self.agent_buy_cheapest(agent_id, &location_id);
                } else {
                    self.agent_goto(agent_id, &location_id);
                }
            }
            QuestObjective::Trade { location_id } => {
                if self.agent_at(agent_id, &location_id) {
                    self.agent_buy_cheapest(agent_id, &location_id);
                } else {
                    self.agent_goto(agent_id, &location_id);
                }
            }
            QuestObjective::VisitZone { zone_id } => {
                let here: bool = self
                    .entities
                    .iter()
                    .find(|e| e.id == agent_id)
                    .map(|e| e.zone_id == zone_id)
                    .unwrap_or(true);
                if !here {
                    let target: Option<String> = self
                        .locations
                        .iter()
                        .filter(|l| l.zone_id == zone_id)
                        .map(|l| l.id.clone())
                        .next();
                    if let Some(t) = target {
                        self.agent_goto(agent_id, &t);
                    }
                }
            }
            QuestObjective::EarnCredits { .. } | QuestObjective::ReachReputation { .. } => {}
        }
    }

    /// True when the agent stands at the location.
    fn agent_at(&self, agent_id: &str, location_id: &str) -> bool {
        self.entities
            .iter()
            .find(|e| e.id == agent_id)
            .map(|e| e.location_id == location_id)
            .unwrap_or(false)
    }

    /// Move toward a location (capacity failures are silent by design).
    fn agent_goto(&mut self, agent_id: &str, location_id: &str) {
        if self.agent_at(agent_id, location_id) {
            return;
        }
        if self.move_entity(agent_id, location_id).is_ok()
            && let Some(e) = self.entities.iter_mut().find(|e| e.id == agent_id)
        {
            e.activity = format!("traveling to {location_id}");
        }
    }

    /// Buy the cheapest affordable listing that is not the agent's own.
    /// Worthy sales leave a Pending proof for the seller.
    fn agent_buy_cheapest(&mut self, agent_id: &str, location_id: &str) {
        let credits: u64 = self
            .entities
            .iter()
            .find(|e| e.id == agent_id)
            .map(|e| e.credits)
            .unwrap_or(0);
        let pick: Option<(String, u64)> = self
            .listings
            .iter()
            .filter(|l| {
                l.active && l.location_id == location_id && l.seller_id != agent_id
            })
            .min_by_key(|l| l.item.price)
            .filter(|l| credits >= l.item.price)
            .map(|l| (l.id.clone(), l.item.price));
        if let Some((listing_id, _)) = pick {
            if let Ok((item, seller_id)) = self.buy_item(agent_id, &listing_id) {
                let desc = format!("{agent_id} bought market goods from {seller_id}");
                self.trade_settlement_proof("item_sale", &desc, &seller_id, item.price);
            }
        }
    }

    /// Vendor restock: trade NPCs list fresh goods on their own rhythm.
    ///
    /// Every 5 ticks each vendor lists ONE item (alternating its two-item
    /// catalog by tick parity) while it has fewer than 2 active listings —
    /// bounded, deterministic, persisted. Only market locations qualify
    /// (`list_item` enforces presence + marketplace). This is what makes
    /// the bazaar alive without human sellers: buyers always find stock,
    /// sales heat nothing here (items have fixed artisan prices), and every
    /// sale still flows through M18 + on-chain settlement at buy time.
    fn npc_vendor_tick(&mut self) {
        if self.tick % 5 != 0 {
            return;
        }
        const VENDORS: &[VendorOffer] = &[
            VendorOffer {
                npc_id: "npc-broker",
                location_id: "market-bazaar",
                even: ("bazaar-token", "Bazaar Token", 8),
                odd: ("trade-compass", "Trade Compass", 12),
            },
            VendorOffer {
                npc_id: "npc-auctioneer",
                location_id: "market-auction",
                even: ("bid-ledger", "Bid Ledger", 7),
                odd: ("auction-gavel", "Auction Gavel", 10),
            },
        ];
        let tick = self.tick;
        for v in VENDORS {
            let active = self
                .listings
                .iter()
                .filter(|l| l.active && l.seller_id == v.npc_id)
                .count();
            if active >= 2 {
                continue;
            }
            let (item_id, name, price) = if tick % 10 < 5 { v.even } else { v.odd };
            let _ = self.list_item(
                v.npc_id,
                WorldItem {
                    id: format!("{item_id}-{tick}"),
                    name: name.to_string(),
                    item_type: "artifact".to_string(),
                    price,
                    description: format!("Vendor goods from {}", v.npc_id),
                },
                v.location_id,
            );
            // list_item fails silently when the NPC is away or the location
            // is no market — vendors never crash the tick.
        }
    }

    /// Buy a service at a location (triggers actual execution).
    /// Price is the DYNAMIC effective price (base + demand premium), so the
    /// same service costs more when hot. Returns (service, provider, price).
    pub fn buy_service(
        &mut self,
        buyer_id: &str,
        location_id: &str,
        service_name: &str,
    ) -> Result<(String, String, u64), String> {
        let price = self.service_price(location_id, service_name).ok_or_else(|| {
            format!("service '{service_name}' not available at {location_id}")
        })?;

        // Verify buyer is at this location
        let buyer = self
            .entities
            .iter()
            .find(|e| e.id == buyer_id)
            .ok_or_else(|| format!("buyer '{buyer_id}' not found"))?;

        if buyer.location_id != location_id {
            return Err("buyer is not at this location".to_string());
        }

        if buyer.credits < price {
            return Err(format!(
                "insufficient credits: {} < {}",
                buyer.credits, price
            ));
        }

        // Find NPC provider at this location
        let provider = self
            .entities
            .iter()
            .find(|e| {
                e.entity_type == "npc"
                    && e.location_id == location_id
                    && e.capabilities.iter().any(|c| c == service_name)
            })
            .map(|e| e.id.clone())
            .unwrap_or_else(|| format!("auto-{}", location_id));

        // Deduct credits from buyer
        if let Some(b) = self.entities.iter_mut().find(|e| e.id == buyer_id) {
            b.credits = b.credits.saturating_sub(price);
            b.activity = format!("used {} service", service_name);
            // Add service result to inventory
            b.inventory.push(WorldItem {
                id: format!("svc-{}-{}", service_name, self.tick),
                name: format!("{} Result", service_name),
                item_type: "service-result".to_string(),
                price: 0,
                description: format!("Result of {} service at {}", service_name, location_id),
            });
        }

        // Credit the provider NPC
        if let Some(p) = self.entities.iter_mut().find(|e| e.id == provider) {
            p.credits += price;
            p.reputation += 0.1;
        }

        let event = WorldEvent {
            tick: self.tick,
            kind: "service_purchased".to_string(),
            detail: format!(
                "{} purchased {} service at {} for {}Cr",
                buyer_id, service_name, location_id, price
            ),
            entity_id: Some(buyer_id.to_string()),
            location_id: Some(location_id.to_string()),
            evidence_id: None,
        };
        self.record_event(event);

        // Demand follows the sale: the next buyer pays a little more.
        self.note_service_sale(location_id, service_name);

        Ok((service_name.to_string(), provider, price))
    }

    /// Demand key for a service offer.
    pub fn service_key(location_id: &str, service_name: &str) -> String {
        format!("{location_id}/{service_name}")
    }

    /// Base price: the remembered first-seen price, else the live map value.
    /// The `locations[].services` map stays the base source of truth.
    pub fn service_base_price(&self, location_id: &str, service_name: &str) -> Option<u64> {
        let key = Self::service_key(location_id, service_name);
        if let Some(base) = self.service_base_prices.get(&key) {
            return Some(*base);
        }
        self.locations
            .iter()
            .find(|l| l.id == location_id)
            .and_then(|l| l.services.get(service_name).copied())
    }

    /// Effective (dynamic) price: `base + base * min(demand, 20) / 20` —
    /// up to 2x at sustained demand. `None` when the service is not offered.
    pub fn service_price(&self, location_id: &str, service_name: &str) -> Option<u64> {
        let base = self.service_base_price(location_id, service_name)?;
        let demand = self
            .service_demand
            .get(&Self::service_key(location_id, service_name))
            .copied()
            .unwrap_or(0)
            .min(20);
        Some(base + base * demand / 20)
    }

    /// Record a sale: backfill the base price on first sale, bump demand.
    fn note_service_sale(&mut self, location_id: &str, service_name: &str) {
        let key = Self::service_key(location_id, service_name);
        if !self.service_base_prices.contains_key(&key)
            && let Some(base) = self
                .locations
                .iter()
                .find(|l| l.id == location_id)
                .and_then(|l| l.services.get(service_name).copied())
        {
            self.service_base_prices.insert(key.clone(), base);
        }
        *self.service_demand.entry(key).or_insert(0) += 1;
    }

    /// Cool every demand counter by one (quiet markets return to base).
    /// Called from [`Self::world_tick`].
    fn decay_service_demand(&mut self) {
        self.service_demand.retain(|_, d| {
            *d = d.saturating_sub(1);
            *d > 0
        });
    }

    // ─── Quest System ─────────────────────────────────────────────────

    /// Generate a quest based on world state. Called by Questkeeper NPC.
    pub fn generate_quest(&mut self) -> Option<Quest> {
        // Find locations with services
        let service_locations: Vec<_> = self
            .locations
            .iter()
            .filter(|l| !l.services.is_empty())
            .collect();

        if service_locations.is_empty() {
            return None;
        }

        // Find agents with needs
        let agents_with_needs: Vec<_> = self
            .entities
            .iter()
            .filter(|e| e.entity_type == "agent" && !e.needs.is_empty())
            .collect();

        // Generate different quest types based on world state
        let quest = if let Some(agent) = agents_with_needs.first() {
            // Quest: fulfill an agent's need
            let needed = &agent.needs[0];
            if let Some(loc) = service_locations.iter().find(|l| l.services.contains_key(needed))
            {
                let price = *loc.services.get(needed).unwrap_or(&10);
                Quest {
                    id: format!("q-{}-{}", self.tick, &needed[..needed.len().min(8)]),
                    title: format!("Deliver {} Service", needed),
                    description: format!(
                        "The Questkeeper needs someone to purchase {} service at {} for an agent in need.",
                        needed, loc.label
                    ),
                    giver_id: "npc-questkeeper".to_string(),
                    objectives: vec![
                        QuestObjective::MoveTo {
                            location_id: loc.id.clone(),
                        },
                        QuestObjective::BuyService {
                            location_id: loc.id.clone(),
                            service: needed.clone(),
                        },
                    ],
                    progress: vec![false, false],
                    reward: QuestReward {
                        credits: price * 2,
                        reputation: 0.5,
                        items: vec![],
                        zone_unlock: None,
                    },
                    status: QuestStatus::Available,
                    accepted_by: None,
                    created_tick: self.tick,
                    accepted_tick: 0,
                    deadline_tick: self.tick + 100,
                    required_reputation: 0.0,
                    required_capabilities: vec![],
                }
            } else {
                // No matching location, create exploration quest
                let target = service_locations[0];
                Quest {
                    id: format!("q-{}-explore", self.tick),
                    title: format!("Explore {}", target.label),
                    description: format!(
                        "The Questkeeper wants you to visit {} in the {} zone.",
                        target.label,
                        target.zone_id
                    ),
                    giver_id: "npc-questkeeper".to_string(),
                    objectives: vec![QuestObjective::MoveTo {
                        location_id: target.id.clone(),
                    }],
                    progress: vec![false],
                    reward: QuestReward {
                        credits: 20,
                        reputation: 0.2,
                        items: vec![],
                        zone_unlock: None,
                    },
                    status: QuestStatus::Available,
                    accepted_by: None,
                    created_tick: self.tick,
                    accepted_tick: 0,
                    deadline_tick: self.tick + 50,
                    required_reputation: 0.0,
                    required_capabilities: vec![],
                }
            }
        } else {
            // No agents with needs — create marketplace quest
            let market = self
                .locations
                .iter()
                .find(|l| l.marketplace)
                .unwrap_or(service_locations[0]);
            Quest {
                id: format!("q-{}-trade", self.tick),
                title: "Market Trader".to_string(),
                description: format!(
                    "The Questkeeper wants you to list an item for sale at {}.",
                    market.label
                ),
                giver_id: "npc-questkeeper".to_string(),
                objectives: vec![
                    QuestObjective::MoveTo {
                        location_id: market.id.clone(),
                    },
                    QuestObjective::ListItem {
                        location_id: market.id.clone(),
                    },
                ],
                progress: vec![false, false],
                reward: QuestReward {
                    credits: 30,
                    reputation: 0.3,
                    items: vec![],
                    zone_unlock: None,
                },
                status: QuestStatus::Available,
                accepted_by: None,
                created_tick: self.tick,
                accepted_tick: 0,
                deadline_tick: self.tick + 50,
                required_reputation: 0.0,
                required_capabilities: vec![],
            }
        };

        self.record_event(WorldEvent {
            tick: self.tick,
            kind: "quest_generated".to_string(),
            detail: format!("New quest: {} (by {})", quest.title, quest.giver_id),
            entity_id: Some(quest.giver_id.clone()),
            location_id: None,
            evidence_id: None,
        });

        Some(quest)
    }

    /// Accept a quest.
    pub fn accept_quest(&mut self, quest_id: &str, agent_id: &str) -> Result<(), String> {
        // Check agent exists and meets requirements
        {
            let agent = self
                .entities
                .iter()
                .find(|e| e.id == agent_id)
                .ok_or_else(|| format!("agent '{}' not found", agent_id))?;

            let quest = self
                .quests
                .iter()
                .find(|q| q.id == quest_id)
                .ok_or_else(|| format!("quest '{}' not found", quest_id))?;

            if quest.status != QuestStatus::Available {
                return Err("quest is not available".to_string());
            }

            if agent.reputation < quest.required_reputation {
                return Err(format!(
                    "insufficient reputation: {} < {}",
                    agent.reputation, quest.required_reputation
                ));
            }

            for req_cap in &quest.required_capabilities {
                if !agent.capabilities.iter().any(|c| c == req_cap) {
                    return Err(format!("missing required capability: {}", req_cap));
                }
            }
        }

        // Accept the quest
        let quest_title;
        {
            let quest = self
                .quests
                .iter_mut()
                .find(|q| q.id == quest_id)
                .ok_or_else(|| format!("quest '{}' not found", quest_id))?;
            quest.status = QuestStatus::Active;
            quest.accepted_by = Some(agent_id.to_string());
            quest.accepted_tick = self.tick;
            quest.progress = vec![false; quest.objectives.len()];
            quest_title = quest.title.clone();
        }

        self.record_event(WorldEvent {
            tick: self.tick,
            kind: "quest_accepted".to_string(),
            detail: format!("{} accepted quest: {}", agent_id, quest_title),
            entity_id: Some(agent_id.to_string()),
            location_id: None,
            evidence_id: None,
        });

        Ok(())
    }

    /// Check and update quest progress based on world state.
    /// Returns list of newly completed quest ids.
    pub fn check_quest_progress(&mut self) -> Vec<String> {
        let mut completed = Vec::new();

        // Gather all active quest info first (avoid borrow issues)
        let active_quests: Vec<(String, Vec<QuestObjective>, String)> = self
            .quests
            .iter()
            .filter(|q| q.status == QuestStatus::Active)
            .map(|q| {
                (
                    q.id.clone(),
                    q.objectives.clone(),
                    q.accepted_by.clone().unwrap_or_default(),
                )
            })
            .collect();

        // Collect updates to apply
        let mut updates: Vec<(String, Vec<bool>, bool)> = Vec::new();

        for (quest_id, objectives, agent_id) in active_quests {
            let mut all_done = true;
            let mut new_progress = Vec::new();

            for obj in &objectives {
                let done = self.check_objective(&agent_id, obj);
                new_progress.push(done);
                if !done {
                    all_done = false;
                }
            }

            updates.push((quest_id, new_progress, all_done));
        }

        // Apply updates
        for (quest_id, new_progress, all_done) in updates {
            if let Some(quest) = self.quests.iter_mut().find(|q| q.id == quest_id) {
                let changed = quest.progress != new_progress;
                let title = quest.title.clone();
                let agent_id = quest.accepted_by.clone().unwrap_or_default();
                let obj_count = quest.objectives.len();
                quest.progress = new_progress;

                if all_done {
                    quest.status = QuestStatus::Ready;
                    self.record_event(WorldEvent {
                        tick: self.tick,
                        kind: "quest_ready".to_string(),
                        detail: format!(
                            "{} completed all objectives for: {}",
                            agent_id, title
                        ),
                        entity_id: Some(agent_id),
                        location_id: None,
                        evidence_id: None,
                    });
                    completed.push(quest_id);
                } else if changed {
                    let done_count = quest.progress.iter().filter(|&&p| p).count();
                    self.record_event(WorldEvent {
                        tick: self.tick,
                        kind: "quest_progress".to_string(),
                        detail: format!(
                            "{} progress on {}: {}/{} objectives",
                            agent_id, title, done_count, obj_count
                        ),
                        entity_id: Some(agent_id),
                        location_id: None,
                        evidence_id: None,
                    });
                }
            }
        }

        completed
    }

    /// Check if a single objective is completed by an agent.
    fn check_objective(&self, agent_id: &str, objective: &QuestObjective) -> bool {
        let agent = match self.entities.iter().find(|e| e.id == agent_id) {
            Some(a) => a,
            None => return false,
        };

        match objective {
            QuestObjective::MoveTo { location_id } => agent.location_id == *location_id,
            QuestObjective::BuyService {
                location_id,
                service,
            } => {
                // Check if agent has a service result item from this location
                agent.location_id == *location_id
                    && agent.inventory.iter().any(|item| {
                        item.item_type == "service-result"
                            && item.description.contains(service)
                    })
            }
            QuestObjective::ListItem { location_id } => {
                // Check if agent has an active listing at this location
                self.listings.iter().any(|l| {
                    l.seller_id == agent_id && l.location_id == *location_id && l.active
                })
            }
            QuestObjective::BuyItem { location_id } => {
                // Check if agent bought something at this location
                agent.location_id == *location_id
                    && self.events.iter().any(|e| {
                        e.kind == "item_sold"
                            && e.entity_id.as_deref() == Some(agent_id)
                            && e.location_id.as_deref() == Some(location_id.as_str())
                    })
            }
            QuestObjective::Trade { location_id } => {
                // Either bought or sold at this location
                agent.location_id == *location_id
                    && self.events.iter().any(|e| {
                        (e.kind == "item_sold" || e.kind == "service_purchased")
                            && e.entity_id.as_deref() == Some(agent_id)
                            && e.location_id.as_deref() == Some(location_id.as_str())
                    })
            }
            QuestObjective::VisitZone { zone_id } => agent.zone_id == *zone_id,
            QuestObjective::EarnCredits { amount } => agent.credits >= *amount,
            QuestObjective::ReachReputation { score } => agent.reputation >= *score,
        }
    }

    /// Complete a quest and distribute rewards.
    pub fn complete_quest(&mut self, quest_id: &str) -> Result<QuestReward, String> {
        let quest = self
            .quests
            .iter()
            .find(|q| q.id == quest_id)
            .ok_or_else(|| format!("quest '{}' not found", quest_id))?;

        if quest.status != QuestStatus::Ready {
            return Err("quest is not ready to complete".to_string());
        }

        let agent_id = quest
            .accepted_by
            .clone()
            .ok_or("quest has no acceptor")?;
        let reward = quest.reward.clone();

        // Find quest and update status
        if let Some(q) = self.quests.iter_mut().find(|q| q.id == quest_id) {
            q.status = QuestStatus::Completed;
        }

        // Distribute rewards
        if let Some(agent) = self.entities.iter_mut().find(|e| e.id == agent_id) {
            agent.credits += reward.credits;
            agent.reputation += reward.reputation;
            for item in &reward.items {
                agent.inventory.push(item.clone());
            }
        }

        // Zone unlock
        if let Some(zone_id) = &reward.zone_unlock {
            if let Some(zone) = self.zones.iter_mut().find(|z| &z.id == zone_id) {
                zone.discovered = true;
            }
        }

        self.record_event(WorldEvent {
            tick: self.tick,
            kind: "quest_completed".to_string(),
            detail: format!(
                "{} completed quest: {} ({}Cr, +{} rep)",
                agent_id,
                self.quests
                    .iter()
                    .find(|q| q.id == quest_id)
                    .map(|q| q.title.as_str())
                    .unwrap_or(""),
                reward.credits,
                reward.reputation
            ),
            entity_id: Some(agent_id),
            location_id: None,
            evidence_id: None,
        });

        // Auto-settle on MultiversX testnet
        self.auto_settle_quest(quest_id);

        Ok(reward)
    }

    /// Generate quests from Questkeeper if none available.
    pub fn questkeeper_tick(&mut self) {
        let available_count = self
            .quests
            .iter()
            .filter(|q| q.status == QuestStatus::Available)
            .count();

        // Keep 2-3 quests available at a time
        if available_count < 2 {
            for _ in 0..2 {
                if let Some(quest) = self.generate_quest() {
                    self.quests.push(quest);
                }
            }
        }
    }

    // ─── On-Chain Settlement ─────────────────────────────────────────

    /// Minimum trade value (credits) worth an on-chain settlement.
    /// Dust trades stay off-chain; services (5–10Cr) and real sales settle.
    pub const SETTLEMENT_MIN_CREDITS: u64 = 5;

    /// Whether a value merits the full M18 + MultiversX settlement path.
    pub fn merits_settlement(amount: u64) -> bool {
        amount >= Self::SETTLEMENT_MIN_CREDITS
    }

    /// Generate an on-chain proof for an economic action.
    /// Creates a BLAKE3 evidence hash and a MultiversX tx intent built with
    /// the real [`Mx8004TxBuilder::submit_proof_raw`] (same encoding the
    /// contract expects: `submit_proof@jobIdHex@digestHex`, digest carried
    /// as-is, never re-hashed).
    pub fn settle_on_chain(
        &mut self,
        action_type: &str,
        description: &str,
        entity_id: &str,
        amount: u64,
    ) -> Result<OnChainProof, String> {
        self.settle_on_chain_with_job("quest", action_type, description, entity_id, amount)
    }

    /// Same as [`Self::settle_on_chain`] but with an explicit job namespace
    /// (`quest` for quest rewards, `trade` for marketplace/service sales) so
    /// the anchored `job_id` tells WHAT settled, not just that something did.
    pub fn settle_on_chain_with_job(
        &mut self,
        job_ns: &str,
        action_type: &str,
        description: &str,
        entity_id: &str,
        amount: u64,
    ) -> Result<OnChainProof, String> {
        // Verify entity exists
        self.entities
            .iter()
            .find(|e| e.id == entity_id)
            .ok_or_else(|| format!("entity '{}' not found", entity_id))?;

        // Generate evidence hash from action data
        let evidence_data = format!(
            "{}:{}:{}:{}:{}",
            action_type, entity_id, amount, self.tick, description
        );
        let digest = blake3::hash(evidence_data.as_bytes());
        let evidence_hash = digest.to_hex().to_string();

        // Build MultiversX testnet tx intent with the REAL builder —
        // no hand-rolled `submit_proof@…` strings anymore.
        let job_id = format!("{job_ns}-{entity_id}-{}", self.tick);
        let intent = Mx8004TxBuilder::submit_proof_raw(&job_id, digest.as_bytes())
            .map_err(|e| format!("tx intent build failed: {e}"))?;
        let tx_data = intent.data_field();

        let proof = OnChainProof {
            id: format!("proof-{}-{}", self.tick, &evidence_hash[..12]),
            action_type: action_type.to_string(),
            description: description.to_string(),
            entity_id: entity_id.to_string(),
            amount,
            evidence_hash: evidence_hash.clone(),
            tx_data,
            tx_hash: String::new(),
            sender: String::new(),
            nonce: None,
            status: SettlementStatus::Pending,
            created_tick: self.tick,
            submitted_tick: 0,
            confirmed_tick: 0,
            network: intent.network.clone(),
        };

        self.record_event(WorldEvent {
            tick: self.tick,
            kind: "settlement_initiated".to_string(),
            detail: format!(
                "On-chain proof generated: {} for {} ({}Cr) — evidence: {}",
                action_type, entity_id, amount, &evidence_hash[..16]
            ),
            entity_id: Some(entity_id.to_string()),
            location_id: None,
            evidence_id: Some(evidence_hash),
        });

        self.proofs.push(proof.clone());
        Ok(proof)
    }

    /// Mark a proof as submitted (tx hash + operator sender + nonce recorded).
    /// The nonce is persisted so restart recovery never reissues it.
    pub fn submit_settlement(
        &mut self,
        proof_id: &str,
        tx_hash: &str,
        sender: &str,
        nonce: Option<u64>,
    ) -> Result<(), String> {
        let (entity_id, evidence_hash) = {
            let proof = self
                .proofs
                .iter_mut()
                .find(|p| p.id == proof_id)
                .ok_or_else(|| format!("proof '{proof_id}' not found"))?;

            proof.status = SettlementStatus::Submitted;
            proof.tx_hash = tx_hash.to_string();
            proof.sender = sender.to_string();
            proof.nonce = nonce;
            proof.submitted_tick = self.tick;

            (proof.entity_id.clone(), proof.evidence_hash.clone())
        };

        self.record_event(WorldEvent {
            tick: self.tick,
            kind: "settlement_submitted".to_string(),
            detail: format!(
                "Proof {proof_id} submitted to MultiversX testnet — tx: {} (sender: {})",
                &tx_hash[..tx_hash.len().min(16)],
                if sender.is_empty() { "?" } else { sender }
            ),
            entity_id: Some(entity_id),
            location_id: None,
            evidence_id: Some(evidence_hash),
        });

        Ok(())
    }

    /// Requeue a `Submitted` proof whose tx never landed on-chain back to
    /// `Pending` (clears tx hash + nonce, keeps sender + evidence). The
    /// sweep path calls this when the chain 404s a submitted hash; the
    /// proof then resubmits with a FRESH nonce instead of rotting.
    pub fn requeue_settlement(&mut self, proof_id: &str) -> Result<(), String> {
        let (entity_id, evidence_hash) = {
            let proof = self
                .proofs
                .iter_mut()
                .find(|p| p.id == proof_id)
                .ok_or_else(|| format!("proof '{proof_id}' not found"))?;
            if proof.status != SettlementStatus::Submitted {
                return Err("proof is not in submitted state".to_string());
            }
            proof.status = SettlementStatus::Pending;
            proof.tx_hash.clear();
            proof.nonce = None;
            proof.submitted_tick = 0;
            (proof.entity_id.clone(), proof.evidence_hash.clone())
        };

        self.record_event(WorldEvent {
            tick: self.tick,
            kind: "settlement_requeued".to_string(),
            detail: format!("Proof {proof_id} requeued: tx never landed, will resubmit"),
            entity_id: Some(entity_id),
            location_id: None,
            evidence_id: Some(evidence_hash),
        });

        Ok(())
    }

    /// Mark a proof as failed (tx rejected/failed on-chain, with reason).
    pub fn fail_settlement(&mut self, proof_id: &str, reason: &str) -> Result<(), String> {
        let (entity_id, evidence_hash) = {
            let proof = self
                .proofs
                .iter_mut()
                .find(|p| p.id == proof_id)
                .ok_or_else(|| format!("proof '{proof_id}' not found"))?;
            proof.status = SettlementStatus::Failed;
            (proof.entity_id.clone(), proof.evidence_hash.clone())
        };
        self.record_event(WorldEvent {
            tick: self.tick,
            kind: "settlement_failed".to_string(),
            detail: format!("Proof {proof_id} failed: {reason}"),
            entity_id: Some(entity_id),
            location_id: None,
            evidence_id: Some(evidence_hash),
        });
        Ok(())
    }

    /// Build the deterministic signing intent for a pending proof.
    ///
    /// Rebuilds the SAME [`UnsignedTxIntent`] `settle_on_chain` produced
    /// (job-id formula + digest carried as-is) and fills the operator
    /// `sender` + testnet `chain_id`. `nonce`/`gas_limit`/`receiver` stay
    /// unset — they belong to the operator's wallet tooling, against
    /// VERIFIED contract addresses.
    ///
    /// Returns the intent plus the hex-encoded
    /// [`canonical_sign_payload`] the operator signs with `gzeu-wallet`.
    /// No key material is touched here — signing happens outside, via
    /// [`decentraai_economy::signer::TransactionSigner`].
    pub fn submit_intent(
        &self,
        proof_id: &str,
        sender: &str,
    ) -> Result<(UnsignedTxIntent, String), String> {
        let sender = sender.trim();
        if sender.is_empty() {
            return Err("sender wallet address required".to_string());
        }
        if !(sender.starts_with("erd1") && sender.len() >= 10) {
            return Err("sender must be a bech32 erd1… address".to_string());
        }
        let proof = self
            .proofs
            .iter()
            .find(|p| p.id == proof_id)
            .ok_or_else(|| format!("proof '{proof_id}' not found"))?;
        if proof.status != SettlementStatus::Pending {
            return Err("proof is not in pending state".to_string());
        }
        let raw = hex::decode(proof.evidence_hash.trim()).map_err(|_| {
            format!("proof '{proof_id}' carries a corrupt evidence hash")
        })?;
        if raw.len() != 32 {
            return Err(format!("proof '{proof_id}' carries a corrupt evidence hash"));
        }
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&raw);
        // Rebuild the SAME intent `settle_on_chain*` produced: try each job
        // namespace and keep the one whose data matches the proof. No
        // re-rolling, no guessing — byte equality decides.
        let mut matched: Option<UnsignedTxIntent> = None;
        for ns in ["trade", "quest"] {
            let job_id = format!("{ns}-{}-{}", proof.entity_id, proof.created_tick);
            if let Ok(intent) = Mx8004TxBuilder::submit_proof_raw(&job_id, &digest)
                && intent.data_field() == proof.tx_data
            {
                matched = Some(intent);
                break;
            }
        }
        let mut intent =
            matched.ok_or_else(|| format!("proof '{proof_id}' intent no longer rebuilds"))?;
        intent.sender = Some(sender.to_string());
        intent.chain_id = Some("T".to_string());
        let payload_hex = hex::encode(canonical_sign_payload(&intent));
        Ok((intent, payload_hex))
    }

    /// Build the settlement proof for a completed World trade.
    ///
    /// `earner` is who delivered value (seller / provider NPC) — they are
    /// the reputation beneficiary on confirmation. Returns `None` for dust
    /// (below [`SETTLEMENT_MIN_CREDITS`]) or unknown earner: those trades
    /// stay purely off-chain by design.
    pub fn trade_settlement_proof(
        &mut self,
        action_type: &str,
        description: &str,
        earner_id: &str,
        amount: u64,
    ) -> Option<OnChainProof> {
        if !Self::merits_settlement(amount) {
            return None;
        }
        self.settle_on_chain_with_job("trade", action_type, description, earner_id, amount)
            .ok()
    }

    /// Confirm a settlement (tx confirmed on-chain).
    pub fn confirm_settlement(&mut self, proof_id: &str) -> Result<(), String> {
        let (entity_id, evidence_hash, tx_hash) = {
            let proof = self
                .proofs
                .iter_mut()
                .find(|p| p.id == proof_id)
                .ok_or_else(|| format!("proof '{}' not found", proof_id))?;

            if proof.status != SettlementStatus::Submitted {
                return Err("proof is not in submitted state".to_string());
            }

            proof.status = SettlementStatus::Confirmed;
            proof.confirmed_tick = self.tick;

            // Grant reputation bonus for confirmed on-chain settlement
            let eid = proof.entity_id.clone();
            (eid.clone(), proof.evidence_hash.clone(), proof.tx_hash.clone())
        };

        if let Some(entity) = self.entities.iter_mut().find(|e| e.id == entity_id) {
            entity.reputation += 0.2;
        }

        self.record_event(WorldEvent {
            tick: self.tick,
            kind: "settlement_confirmed".to_string(),
            detail: format!(
                "Proof {} confirmed on MultiversX testnet — tx: {} (reputation +0.2)",
                proof_id, tx_hash
            ),
            entity_id: Some(entity_id),
            location_id: None,
            evidence_id: Some(evidence_hash),
        });

        Ok(())
    }

    /// Auto-settle quest completions on-chain.
    /// Called after quest completion to anchor the reward on MultiversX.
    pub fn auto_settle_quest(&mut self, quest_id: &str) -> Option<OnChainProof> {
        let (action, desc, agent_id, amount) = {
            let quest = self.quests.iter().find(|q| q.id == quest_id)?;
            if quest.status != QuestStatus::Completed {
                return None;
            }
            let agent_id = quest.accepted_by.clone()?;
            let reward = quest.reward.clone();
            if reward.credits == 0 && reward.reputation == 0.0 {
                return None; // No economic value to settle
            }
            (
                "quest_completion".to_string(),
                format!("Quest '{}' completed", quest.title),
                agent_id,
                reward.credits,
            )
        };

        self.settle_on_chain(&action, &desc, &agent_id, amount).ok()
    }
}

// ─── Persistence ─────────────────────────────────────────────────────

pub fn world_path_for(repo_root: &Path) -> PathBuf {
    repo_root.join("db/world.json")
}

pub fn load_world_state(path: &Path) -> WorldState {
    let mut state: WorldState = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    // Ensure zones and locations exist (populate defaults for legacy world.json)
    if state.zones.is_empty() {
        state.zones = default_zones();
    }
    if state.locations.is_empty() {
        state.locations = default_locations();
    }
    // Migrate legacy agents to entities on load
    state.migrate_legacy();
    state
}

pub fn save_world_state(path: &Path, state: &WorldState) {
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let tmp = path.with_extension("tmp");
    if let Ok(s) = serde_json::to_string_pretty(state) {
        if std::fs::write(&tmp, &s).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

// ─── HTML UI ─────────────────────────────────────────────────────────

pub fn world_html() -> String {
    r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>DecentraAI — Open World</title>
<style>
:root{--bg:#070a12;--panel:#111827;--line:#1f2a44;--text:#e6eef8;--muted:#8aa0b8;--accent:#22d3ee;--accent2:#a78bfa;--ok:#34d399;--warn:#fbbf24;--bad:#f87171;--gold:#f59e0b}
*{box-sizing:border-box;margin:0;padding:0}
body{background:var(--bg);color:var(--text);font:13px/1.5 system-ui,sans-serif;overflow-x:hidden}
.layout{display:grid;grid-template-columns:260px 1fr 300px;grid-template-rows:auto 1fr;height:100vh;gap:0}
header{grid-column:1/-1;display:flex;justify-content:space-between;align-items:center;padding:10px 16px;border-bottom:1px solid var(--line);background:var(--panel)}
h1{font-size:18px;letter-spacing:.5px} h1 span{color:var(--accent)}
.badge{padding:2px 8px;border-radius:999px;border:1px solid var(--line);font-size:11px;color:var(--muted)}
.badge.live{border-color:var(--ok);color:var(--ok);box-shadow:0 0 8px #34d39955}
.sidebar{border-right:1px solid var(--line);overflow-y:auto;padding:12px;background:#0a0f1a}
.main{overflow-y:auto;padding:16px}
.right{border-left:1px solid var(--line);overflow-y:auto;padding:12px;background:#0a0f1a}
.zone{border:1px solid var(--line);border-radius:10px;margin-bottom:8px;overflow:hidden;cursor:pointer;transition:border-color .2s}
.zone:hover{border-color:var(--accent)}
.zone.active{border-color:var(--accent);box-shadow:0 0 12px #22d3ee33}
.zone-header{padding:8px 10px;display:flex;justify-content:space-between;align-items:center}
.zone-header .name{font-weight:600;font-size:13px}
.zone-header .count{font-size:11px;color:var(--muted)}
.zone-body{padding:0 10px 8px;display:none}
.zone.active .zone-body{display:block}
.location{border:1px solid var(--line);border-radius:8px;padding:8px;margin-bottom:6px;cursor:pointer;transition:border-color .2s,background .2s}
.location:hover{border-color:var(--accent);background:#0d1426}
.location.active{border-color:var(--ok);background:#0d1a1a}
.location .name{font-weight:600;font-size:12px}
.location .desc{font-size:11px;color:var(--muted);margin-top:2px}
.location .tags{margin-top:4px;display:flex;gap:4px;flex-wrap:wrap}
.tag{font-size:10px;padding:1px 6px;border-radius:999px;border:1px solid var(--line);color:var(--muted)}
.tag.svc{border-color:var(--accent2);color:var(--accent2)}
.tag.market{border-color:var(--gold);color:var(--gold)}
.entity{border:1px solid #22304a;background:#0e1a30;border-radius:8px;padding:7px 9px;margin-bottom:6px;display:flex;justify-content:space-between;align-items:center;transition:transform .2s}
.entity:hover{transform:translateX(2px)}
.entity .name{font-weight:600;font-size:12px}
.entity .meta{font-size:10px;color:var(--muted)}
.dot{width:7px;height:7px;border-radius:50%;background:var(--muted)}
.dot.idle{background:var(--muted)} .dot.working{background:var(--warn)} .dot.trading{background:var(--gold)} .dot.moving{background:var(--accent)}
.listing{border:1px solid var(--line);border-radius:8px;padding:8px;margin-bottom:6px;transition:border-color .2s}
.listing:hover{border-color:var(--gold)}
.listing .item-name{font-weight:600;font-size:12px}
.listing .price{color:var(--gold);font-size:12px;font-weight:600}
.listing .seller{font-size:10px;color:var(--muted)}
.btn{padding:6px 12px;border-radius:8px;border:1px solid #22304a;background:#0e1a30;color:var(--text);font-size:12px;cursor:pointer;transition:border-color .2s}
.btn:hover{border-color:var(--accent)}
.btn.primary{background:linear-gradient(180deg,#1a2a4a,#12203a);border-color:#2a3a5e}
.btn.primary:hover{border-color:var(--accent)}
.events{max-height:300px;overflow:auto}
.event{padding:4px 0;border-bottom:1px solid #14203a;font-size:11px;display:flex;gap:6px}
.event .tick{color:var(--accent);font-family:ui-monospace,monospace;min-width:30px}
.mission{border-left:3px solid var(--accent);padding-left:10px;margin-bottom:12px}
.status{font-size:11px;padding:2px 7px;border-radius:999px;border:1px solid var(--line);color:var(--muted)}
.status.open{color:var(--muted)} .status.bidding{color:var(--warn);border-color:var(--warn)} .status.assigned{color:var(--accent);border-color:var(--accent)} .status.settled{color:var(--ok);border-color:var(--ok)}
.evidence{font-family:ui-monospace,monospace;font-size:11px;color:var(--accent2);word-break:break-all}
.join{display:flex;gap:6px;flex-wrap:wrap;margin-top:8px}
.join input,.join select,.join button{padding:6px 8px;border-radius:8px;border:1px solid #22304a;background:#0a0e16;color:var(--text);font-size:12px}
.join button{cursor:pointer;background:linear-gradient(180deg,#1a2a4a,#12203a);border-color:#2a3a5e}
.join button:hover{border-color:var(--accent)}
.panel-title{font-size:11px;text-transform:uppercase;letter-spacing:1px;color:var(--muted);margin-bottom:8px;display:flex;justify-content:space-between;align-items:center}
#agentsCount{color:var(--accent)}
.location-detail{display:none}
.location-detail.active{display:block}
.move-btn{margin-top:6px}
</style></head><body>
<div class="layout">
<header><div><h1>● DecentraAI <span>Open World</span></h1><div class="sub" style="color:var(--muted);font-size:11px">Persistent world · Agents explore, trade, and work · M18 economy</div></div><div style="display:flex;gap:8px;align-items:center"><span id="sse" class="badge">SSE …</span> <span class="badge">tick <b id="tick">…</b></span> <span class="badge"><span id="agentsCount">0</span> entities</span> <span class="badge"><span id="zoneCount">0</span> zones</span></div></header>

<div class="sidebar">
<div class="panel-title">Zones</div>
<div id="zones"></div>
<div class="panel-title" style="margin-top:12px">Your Agent</div>
<div class="join"><input id="tok" placeholder="dca_..." style="flex:1;min-width:180px"><button class="btn primary" onclick="connect()">Connect</button></div>
<div id="agentInfo" class="sub" style="margin-top:6px"></div>
</div>

<div class="main" id="mainView">
<div id="locationList"></div>
<div id="locationDetail" class="location-detail"></div>
<div class="mission" id="missionSection" style="margin-top:16px"></div>
</div>

<div class="right">
<div class="panel-title">Entities Here <span id="entityCount" class="sub">0</span></div>
<div id="entities"></div>
<div class="panel-title" style="margin-top:12px">Marketplace <span id="listingCount" class="sub">0</span></div>
<div id="listings"></div>
<div class="panel-title" style="margin-top:12px">Live Events</div>
<div id="events" class="events"></div>
</div>
</div>

<script>
let state=null,myEntity=null,selectedZone=null,selectedLocation=null;
function tok(){return document.getElementById('tok').value.trim()||localStorage.getItem('world-token')||''}
function auth(){const t=tok();return t?{Authorization:'Bearer '+t}:{}}
async function j(url,opts={}){try{const r=await fetch(url,{...opts,headers:{...(opts.headers||{}),...auth()}});const text=await r.text();let js;try{js=JSON.parse(text)}catch(_){js=text}return{ok:r.ok,js,status:r.status}}catch(e){return{ok:false,js:String(e)}}}

function renderZones(w){
 const el=document.getElementById('zones');
 el.innerHTML=w.zones.map(z=>{
  const locs=w.locations.filter(l=>l.zone_id===z.id);
  const entities=w.entities.filter(e=>e.zone_id===z.id);
  const isActive=selectedZone===z.id;
  return `<div class="zone ${isActive?'active':''}" onclick="selectZone('${z.id}')" style="border-left:3px solid ${z.color}">
   <div class="zone-header"><span class="name">${z.label}</span><span class="count">${entities.length} agents · ${locs.length} locations</span></div>
   <div class="zone-body">${locs.map(l=>{
    const le=w.entities.filter(e=>e.location_id===l.id);
    const isActive2=selectedLocation===l.id;
    const tags=[];
    Object.keys(l.services||{}).forEach(s=>tags.push(`<span class="tag svc">${s}</span>`));
    if(l.marketplace)tags.push('<span class="tag market">marketplace</span>');
    return `<div class="location ${isActive2?'active':''}" onclick="event.stopPropagation();selectLocation('${l.id}')">
     <div class="name">${l.label}</div><div class="desc">${l.description}</div>
     <div class="tags">${tags.join('')}</div>
     <div class="sub" style="margin-top:3px">${le.length}/${l.capacity} agents</div>
    </div>`}).join('')}</div>
  </div>`}).join('');
 document.getElementById('zoneCount').textContent=w.zones.length;
}

function renderLocationDetail(w){
 const el=document.getElementById('locationDetail');
 if(!selectedLocation){el.className='location-detail';return}
 const loc=w.locations.find(l=>l.id===selectedLocation);
 if(!loc){el.className='location-detail';return}
 el.className='location-detail active';
 const entities=w.entities.filter(e=>e.location_id===selectedLocation);
 const listings=w.listings.filter(l=>l.location_id===selectedLocation&&l.active);
 const svcs=Object.entries(loc.services||{}).map(([k,v])=>`<span class="tag svc">${k} ${v}Cr</span>`).join(' ');
 el.innerHTML=`<div style="margin-bottom:12px"><h3 style="font-size:16px;margin-bottom:4px">${loc.label}</h3><div style="color:var(--muted);font-size:12px">${loc.description}</div>
  <div style="margin-top:6px;display:flex;gap:4px;flex-wrap:wrap">${svcs}${loc.marketplace?'<span class="tag market">marketplace</span>':''}</div>
  ${myEntity?`<button class="btn primary move-btn" onclick="moveTo('${selectedLocation}')">Move Here</button>`:''}
  ${myEntity&&loc.marketplace?`<button class="btn" style="margin-left:6px" onclick="showListDialog()">List Item</button>`:''}
  </div>
  <div style="font-size:11px;color:var(--muted);margin-bottom:6px">ENTITIES (${entities.length})</div>
  ${entities.map(e=>`<div class="entity"><div><div class="name">${e.name} ${e.id===myEntity?.id?'(you)':''}</div><div class="meta">${e.capabilities.join(', ')||'no caps'} · ${e.state} · ${e.credits}Cr</div></div><div class="dot ${e.state}"></div></div>`).join('')}
  ${listings.length?`<div style="font-size:11px;color:var(--muted);margin:12px 0 6px">LISTINGS (${listings.length})</div>${listings.map(l=>`<div class="listing"><div style="display:flex;justify-content:space-between"><span class="item-name">${l.item.name}</span><span class="price">${l.item.price}Cr</span></div><div class="seller">by ${l.seller_id}</div>${myEntity&&myEntity.id!==l.seller_id?`<button class="btn" style="margin-top:4px" onclick="buyItem('${l.id}')">Buy</button>`:''}</div>`).join('')}`:''}`;
}

function renderEntities(w){
 const loc=selectedLocation||'';
 const entities=loc?w.entities.filter(e=>e.location_id===loc):w.entities;
 document.getElementById('entityCount').textContent=entities.length;
 document.getElementById('entities').innerHTML=entities.slice(0,30).map(e=>`<div class="entity"><div><div class="name">${e.name} ${e.id===myEntity?.id?'(you)':''}</div><div class="meta">${e.capabilities.join(', ')} · ${e.state} · ${e.credits}Cr · rep ${e.reputation?.toFixed?.(1)??'0.0'}</div></div><div class="dot ${e.state}"></div></div>`).join('')||'<div class="sub">no entities</div>';
}

function renderListings(w){
 const loc=selectedLocation||'';
 const listings=loc?w.listings.filter(l=>l.location_id===loc&&l.active):w.listings.filter(l=>l.active);
 document.getElementById('listingCount').textContent=listings.length;
 document.getElementById('listings').innerHTML=listings.slice(0,20).map(l=>`<div class="listing"><div style="display:flex;justify-content:space-between"><span class="item-name">${l.item.name}</span><span class="price">${l.item.price}Cr</span></div><div class="seller">by ${l.seller_id} at ${l.location_id}</div>${myEntity&&myEntity.id!==l.seller_id?`<button class="btn" style="margin-top:4px" onclick="buyItem('${l.id}')">Buy</button>`:''}</div>`).join('')||'<div class="sub">no listings</div>';
}

function renderMission(w){
 const el=document.getElementById('missionSection');
 if(!w.mission||!w.mission.task){el.innerHTML='';return}
 const t=w.mission.task;const s=t.status||'open';
 el.innerHTML=`<div class="panel-title">Active Mission</div><div style="border-left:3px solid var(--accent);padding-left:10px"><div style="font-weight:600">${t.title} <span style="font-weight:400;color:var(--muted)">#${t.id} · ${t.reward}Cr</span></div><div style="font-size:11px;color:var(--muted)">issuer ${t.issuer} · ${t.required_capability||'any'}</div><span class="status ${s}">${s}</span></div>`;
}

function renderAll(w){
 state=w;
 document.getElementById('tick').textContent=w.tick;
 document.getElementById('agentsCount').textContent=w.entities.length;
 renderZones(w);renderLocationDetail(w);renderEntities(w);renderListings(w);renderMission(w);
}

async function tick(){const s=await j('/v1/world');if(!s.ok)return;renderAll(s.js)}

function selectZone(id){selectedZone=selectedZone===id?null:id;selectedLocation=null;tick()}
function selectLocation(id){selectedLocation=selectedLocation===id?null:id;tick()}

async function moveTo(loc){
 if(!myEntity){alert('Connect first');return}
 const r=await j('/v1/world/move',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({entity_id:myEntity.id,location_id:loc})});
 if(r.ok){myEntity.location_id=loc;tick()}else{alert(JSON.stringify(r.js).slice(0,200))}
}

async function buyItem(listingId){
 if(!myEntity){alert('Connect first');return}
 const r=await j('/v1/world/buy',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({buyer_id:myEntity.id,listing_id:listingId})});
 if(r.ok){tick()}else{alert(JSON.stringify(r.js).slice(0,200))}
}

function showListDialog(){
 const name=prompt('Item name:');
 if(!name)return;
 const price=parseInt(prompt('Price in credits:','10'));
 if(!price||price<=0)return;
 const id='item-'+Date.now();
 j('/v1/world/list',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({seller_id:myEntity.id,item:{id,name,item_type:'service',price,description:name},location_id:selectedLocation})}).then(()=>tick())
}

async function connect(){
 const t=tok();if(!t){alert('Enter dca_ key');return}
 localStorage.setItem('world-token',t);
 // Check if we're already in the world
 const s=await j('/v1/world');
 if(s.ok){
  const me=s.js.entities.find(e=>e.wallet===t||e.id===localStorage.getItem('world-agent-id'));
  if(me){myEntity=me;document.getElementById('agentInfo').innerHTML=`<b>${me.name}</b> at ${me.location_id} · ${me.credits}Cr`;tick();return}
 }
 // Auto-join
 const name=prompt('Agent name:','explorer-'+Math.floor(Math.random()*9999));
 if(!name)return;
 const r=await j('/v1/world/join',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({declared_capabilities:['general']})});
 if(r.ok){
  myEntity={id:r.js.agent_id,name:r.js.agent_id,entity_type:'agent',zone_id:'central-hub',location_id:'hub-plaza',state:'idle',capabilities:['general'],needs:[],wallet:r.js.account||t,reputation:0,credits:100,activity:'',last_move_tick:0,inventory:[]};
  localStorage.setItem('world-agent-id',myEntity.id);
  document.getElementById('agentInfo').innerHTML=`<b>${myEntity.name}</b> at ${myEntity.location_id} · ${myEntity.credits}Cr`;
  tick();
 }else{document.getElementById('agentInfo').innerHTML='<span style="color:var(--bad)">'+JSON.stringify(r.js).slice(0,200)+'</span>'}
}

function addEvents(arr){
 if(!arr||!arr.length)return;
 const c=document.getElementById('events');
 const html=arr.slice().reverse().map(e=>`<div class="event"><span class="tick">#${e.tick}</span><div><b>${e.kind}</b> ${e.detail||''}</div></div>`).join('');
 c.innerHTML=html+c.innerHTML;
 while(c.children.length>60)c.removeChild(c.lastChild);
}

let es=null;
function connectSSE(){
 try{es=new EventSource('/v1/world/stream');
 es.onopen=()=>{document.getElementById('sse').textContent='SSE live';document.getElementById('sse').className='badge live'};
 es.onerror=()=>{document.getElementById('sse').textContent='SSE retry'};
 es.addEventListener('world',ev=>{try{const arr=JSON.parse(ev.data);addEvents(arr);tick()}catch(_){}});
 es.addEventListener('hub_events',ev=>{try{const arr=JSON.parse(ev.data);addEvents(arr);tick()}catch(_){}});
 }catch(_){}
}
connectSSE();setInterval(tick,3000);tick()
</script></body></html>"##.to_string()
}

pub fn world_skill_md() -> &'static str {
    include_str!("../../../.agents/skills/world.md")
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_world_has_zones_and_locations() {
        let w = WorldState::default();
        assert!(w.zones.len() >= 4);
        assert!(w.locations.len() >= 8);
    }

    #[test]
    fn move_entity_works() {
        let mut w = WorldState::default();
        w.entities.push(WorldEntity {
            id: "test-agent".to_string(),
            name: "Test".to_string(),
            entity_type: "agent".to_string(),
            zone_id: "central-hub".to_string(),
            location_id: "hub-plaza".to_string(),
            state: EntityState::Idle,
            capabilities: vec!["research".to_string()],
            needs: vec![],
            wallet: "test-wallet".to_string(),
            reputation: 0.0,
            credits: 100,
            activity: String::new(),
            last_move_tick: 0,
            inventory: vec![],
        });

        let result = w.move_entity("test-agent", "research-lab-main");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "research-lab-main");
        assert_eq!(w.entities[0].location_id, "research-lab-main");
        assert_eq!(w.entities[0].zone_id, "research-district");
    }

    #[test]
    fn move_entity_fails_not_found() {
        let mut w = WorldState::default();
        let result = w.move_entity("nonexistent", "hub-plaza");
        assert!(result.is_err());
    }

    #[test]
    fn move_entity_fails_capacity() {
        let mut w = WorldState::default();
        // Fill forest-clearing to capacity (5)
        for i in 0..5 {
            w.entities.push(WorldEntity {
                id: format!("agent-{}", i),
                name: format!("Agent {}", i),
                entity_type: "agent".to_string(),
                zone_id: "deep-forest".to_string(),
                location_id: "forest-clearing".to_string(),
                state: EntityState::Idle,
                capabilities: vec![],
                needs: vec![],
                wallet: String::new(),
                reputation: 0.0,
                credits: 0,
                activity: String::new(),
                last_move_tick: 0,
                inventory: vec![],
            });
        }
        w.entities.push(WorldEntity {
            id: "overflow".to_string(),
            name: "Overflow".to_string(),
            entity_type: "agent".to_string(),
            zone_id: "central-hub".to_string(),
            location_id: "hub-plaza".to_string(),
            state: EntityState::Idle,
            capabilities: vec![],
            needs: vec![],
            wallet: String::new(),
            reputation: 0.0,
            credits: 0,
            activity: String::new(),
            last_move_tick: 0,
            inventory: vec![],
        });
        let result = w.move_entity("overflow", "forest-clearing");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("full"));
    }

    #[test]
    fn list_and_buy_item() {
        let mut w = WorldState::default();
        // Set up seller
        w.entities.push(WorldEntity {
            id: "seller".to_string(),
            name: "Seller".to_string(),
            entity_type: "agent".to_string(),
            zone_id: "marketplace".to_string(),
            location_id: "market-bazaar".to_string(),
            state: EntityState::Idle,
            capabilities: vec!["ocr".to_string()],
            needs: vec![],
            wallet: "seller-wallet".to_string(),
            reputation: 1.0,
            credits: 0,
            activity: String::new(),
            last_move_tick: 0,
            inventory: vec![],
        });
        // Set up buyer
        w.entities.push(WorldEntity {
            id: "buyer".to_string(),
            name: "Buyer".to_string(),
            entity_type: "agent".to_string(),
            zone_id: "marketplace".to_string(),
            location_id: "market-bazaar".to_string(),
            state: EntityState::Idle,
            capabilities: vec![],
            needs: vec!["ocr".to_string()],
            wallet: "buyer-wallet".to_string(),
            reputation: 0.0,
            credits: 50,
            activity: String::new(),
            last_move_tick: 0,
            inventory: vec![],
        });

        let item = WorldItem {
            id: "ocr-service".to_string(),
            name: "OCR Processing".to_string(),
            item_type: "service".to_string(),
            price: 10,
            description: "Process 100 pages".to_string(),
        };

        let listing_id = w.list_item("seller", item, "market-bazaar");
        assert!(listing_id.is_ok());

        let result = w.buy_item("buyer", &listing_id.unwrap());
        assert!(result.is_ok());

        // Verify credits transferred
        let buyer = w.entities.iter().find(|e| e.id == "buyer").unwrap();
        assert_eq!(buyer.credits, 40);
        let seller = w.entities.iter().find(|e| e.id == "seller").unwrap();
        assert_eq!(seller.credits, 10);
    }

    #[test]
    fn migrate_legacy_agents() {
        let mut w = WorldState::default();
        w.agents.push(WorldAgent {
            agent_id: "legacy-agent".to_string(),
            key_id: "ck-1234".to_string(),
            account: "dca_test".to_string(),
            declared_capabilities: vec!["research".to_string()],
            needs: vec![],
            room_id: "research-lab".to_string(),
            joined_at: 1000,
        });

        w.migrate_legacy();
        assert!(w.agents.is_empty());
        assert_eq!(w.entities.len(), 1);
        assert_eq!(w.entities[0].id, "legacy-agent");
        assert_eq!(w.entities[0].zone_id, "research-district");
    }

    #[test]
    fn entities_at_location() {
        let mut w = WorldState::default();
        for i in 0..3 {
            w.entities.push(WorldEntity {
                id: format!("a{}", i),
                name: format!("A{}", i),
                entity_type: "agent".to_string(),
                zone_id: "central-hub".to_string(),
                location_id: "hub-plaza".to_string(),
                state: EntityState::Idle,
                capabilities: vec![],
                needs: vec![],
                wallet: String::new(),
                reputation: 0.0,
                credits: 0,
                activity: String::new(),
                last_move_tick: 0,
                inventory: vec![],
            });
        }
        assert_eq!(w.entities_at("hub-plaza").len(), 3);
        assert_eq!(w.entities_at("market-bazaar").len(), 0);
    }

    #[test]
    fn record_event_bounds() {
        let mut w = WorldState::default();
        for i in 0..250 {
            w.record_event(WorldEvent {
                tick: i,
                kind: "test".to_string(),
                detail: format!("event {}", i),
                entity_id: None,
                location_id: None,
                evidence_id: None,
            });
        }
        assert!(w.events.len() <= 200);
    }

    #[test]
    fn settle_on_chain_generates_proof() {
        let mut w = WorldState::default();
        w.entities.push(WorldEntity {
            id: "agent-1".to_string(),
            name: "Agent".to_string(),
            entity_type: "agent".to_string(),
            zone_id: "central-hub".to_string(),
            location_id: "hub-plaza".to_string(),
            state: EntityState::Idle,
            capabilities: vec![],
            needs: vec![],
            wallet: "erd1test".to_string(),
            reputation: 0.5,
            credits: 100,
            activity: String::new(),
            last_move_tick: 0,
            inventory: vec![],
        });

        let proof = w.settle_on_chain(
            "quest_completion",
            "Quest 'Deliver OCR' completed",
            "agent-1",
            20,
        );
        assert!(proof.is_ok());
        let proof = proof.unwrap();
        assert!(proof.id.starts_with("proof-"));
        assert_eq!(proof.action_type, "quest_completion");
        assert_eq!(proof.entity_id, "agent-1");
        assert_eq!(proof.amount, 20);
        assert!(!proof.evidence_hash.is_empty());
        assert!(proof.tx_data.starts_with("submit_proof@"));
        assert_eq!(proof.status, SettlementStatus::Pending);
        assert_eq!(proof.network, "multiversx-testnet");
        assert_eq!(w.proofs.len(), 1);
    }

    #[test]
    fn settle_on_chain_fails_for_unknown_entity() {
        let mut w = WorldState::default();
        let proof = w.settle_on_chain("test", "test", "nonexistent", 10);
        assert!(proof.is_err());
    }

    #[test]
    fn submit_and_confirm_settlement() {
        let mut w = WorldState::default();
        w.entities.push(WorldEntity {
            id: "agent-1".to_string(),
            name: "Agent".to_string(),
            entity_type: "agent".to_string(),
            zone_id: "central-hub".to_string(),
            location_id: "hub-plaza".to_string(),
            state: EntityState::Idle,
            capabilities: vec![],
            needs: vec![],
            wallet: "erd1test".to_string(),
            reputation: 0.0,
            credits: 100,
            activity: String::new(),
            last_move_tick: 0,
            inventory: vec![],
        });

        let proof = w
            .settle_on_chain("quest_completion", "completed", "agent-1", 20)
            .unwrap();
        let proof_id = proof.id.clone();

        // Submit
        assert!(w
            .submit_settlement(&proof_id, "abc123def456", "erd1operator", Some(7))
            .is_ok());
        assert_eq!(w.proofs[0].status, SettlementStatus::Submitted);
        assert_eq!(w.proofs[0].tx_hash, "abc123def456");
        assert_eq!(w.proofs[0].sender, "erd1operator");

        // Confirm
        assert!(w.confirm_settlement(&proof_id).is_ok());
        assert_eq!(w.proofs[0].status, SettlementStatus::Confirmed);
        // Reputation bonus
        assert!((w.entities[0].reputation - 0.2).abs() < f32::EPSILON);
    }

    #[test]
    fn auto_settle_quest_creates_proof() {
        let mut w = WorldState::default();
        w.entities.push(WorldEntity {
            id: "agent-1".to_string(),
            name: "Agent".to_string(),
            entity_type: "agent".to_string(),
            zone_id: "central-hub".to_string(),
            location_id: "hub-plaza".to_string(),
            state: EntityState::Idle,
            capabilities: vec![],
            needs: vec![],
            wallet: "erd1test".to_string(),
            reputation: 0.0,
            credits: 100,
            activity: String::new(),
            last_move_tick: 0,
            inventory: vec![],
        });

        // Create a quest via WorldState
        w.quests.push(Quest {
            id: "q1".to_string(),
            title: "Deliver OCR".to_string(),
            description: "test".to_string(),
            giver_id: "questkeeper-1".to_string(),
            objectives: vec![QuestObjective::MoveTo {
                location_id: "hub-plaza".to_string(),
            }],
            reward: QuestReward {
                credits: 20,
                reputation: 0.5,
                items: vec![],
                zone_unlock: None,
            },
            status: QuestStatus::Available,
            accepted_by: None,
            accepted_tick: 0,
            created_tick: 0,
            deadline_tick: 0,
            required_reputation: 0.0,
            required_capabilities: vec![],
            progress: vec![false],
        });

        // Accept quest via WorldState
        assert!(w.accept_quest("q1", "agent-1").is_ok());
        assert_eq!(w.quests[0].status, QuestStatus::Active);
        assert_eq!(w.quests[0].accepted_by.as_deref(), Some("agent-1"));

        // Mark quest completed directly for testing
        w.quests[0].status = QuestStatus::Completed;

        // Auto-settle
        let proof = w.auto_settle_quest("q1");
        assert!(proof.is_some());
        let proof = proof.unwrap();
        assert_eq!(proof.action_type, "quest_completion");
        assert_eq!(proof.amount, 20);
        assert_eq!(proof.entity_id, "agent-1");
    }

    #[test]
    fn agents_live_alone_needs_and_quests() {
        let mut w = WorldState::default();
        w.locations.push(WorldLocation {
            id: "loc-life".to_string(),
            zone_id: "z".to_string(),
            label: "Life".to_string(),
            description: String::new(),
            services: [("ocr".to_string(), 10u64)].into_iter().collect(),
            marketplace: false,
            capacity: 10,
        });
        w.locations.push(WorldLocation {
            id: "loc-far".to_string(),
            zone_id: "z".to_string(),
            label: "Far".to_string(),
            description: String::new(),
            services: HashMap::new(),
            marketplace: false,
            capacity: 10,
        });
        for (id, etype, caps) in [
            ("agent-a", "agent", vec![]),
            ("npc-o", "npc", vec!["ocr".to_string()]),
        ] {
            w.entities.push(WorldEntity {
                id: id.to_string(),
                name: id.to_string(),
                entity_type: etype.to_string(),
                zone_id: "z".to_string(),
                location_id: "loc-life".to_string(),
                state: EntityState::Idle,
                capabilities: caps,
                needs: if etype == "agent" {
                    vec!["ocr".to_string()]
                } else {
                    vec![]
                },
                wallet: format!("erd1{id}"),
                reputation: 0.0,
                credits: 100,
                activity: String::new(),
                last_move_tick: 0,
                inventory: vec![],
            });
        }
        // The world is eager: in ONE tick the agent moves to the ocr lab,
        // fulfills its need, takes the generated quest (already positioned),
        // and completes it for +20Cr. Need gone, reward paid, proof pends.
        w.world_tick();
        let a = w.entities.iter().find(|e| e.id == "agent-a").unwrap();
        assert!(a.needs.is_empty(), "need should be fulfilled");
        assert_eq!(a.credits, 110); // 100 - 10 service + 20 quest reward
        assert!(a.inventory.iter().any(|i| i.item_type == "service-result"));
        assert_eq!(w.proofs.len(), 1); // quest proof (sale had no NPC earner)
        assert_eq!(w.proofs[0].entity_id, "agent-a");

        // Quest autonomy: available MoveTo quest gets taken, walked, done.
        w.entities
            .iter_mut()
            .find(|e| e.id == "agent-a")
            .unwrap()
            .location_id = "loc-far".to_string();
        w.quests.push(Quest {
            id: "q-life".to_string(),
            title: "Go far".to_string(),
            description: String::new(),
            giver_id: "npc-o".to_string(),
            objectives: vec![QuestObjective::MoveTo {
                location_id: "loc-far".to_string(),
            }],
            reward: QuestReward {
                credits: 20,
                reputation: 0.5,
                items: vec![],
                zone_unlock: None,
            },
            status: QuestStatus::Available,
            accepted_by: None,
            accepted_tick: 0,
            created_tick: 0,
            deadline_tick: 999,
            required_reputation: 0.0,
            required_capabilities: vec![],
            progress: vec![false],
        });
        // Tick 1: accept (already there → progress true → Ready).
        w.world_tick();
        // Tick 2: Ready → Completed + reward + auto-settle proof.
        w.world_tick();
        let q = w.quests.iter().find(|q| q.id == "q-life").unwrap();
        assert_eq!(q.status, QuestStatus::Completed);
        let a = w.entities.iter().find(|e| e.id == "agent-a").unwrap();
        assert_eq!(a.credits, 130); // 110 + 20 reward
        assert!(a.reputation >= 0.5);
        assert_eq!(w.proofs.len(), 2); // both quest proofs
    }

    #[test]
    fn vendors_restock_bounded_and_deterministic() {
        let mut w = WorldState::default();
        w.spawn_npcs();
        assert!(w.listings.iter().all(|l| !l.active));
        for _ in 0..5 {
            w.world_tick();
        }
        let active: Vec<_> = w.listings.iter().filter(|l| l.active).collect();
        // Both vendors listed once (tick 5 fires).
        assert_eq!(active.len(), 2);
        assert!(active.iter().any(|l| l.seller_id == "npc-broker"));
        assert!(active.iter().any(|l| l.seller_id == "npc-auctioneer"));
        // Catalog alternates by tick parity: tick 5 → odd items (5 % 10 >= 5).
        assert!(active.iter().any(|l| l.item.id.starts_with("trade-compass-5")));
        // Cap holds over many ticks: never more than 2 per vendor.
        for _ in 0..60 {
            w.world_tick();
        }
        for seller in ["npc-broker", "npc-auctioneer"] {
            let n = w
                .listings
                .iter()
                .filter(|l| l.active && l.seller_id == seller)
                .count();
            assert!(n <= 2, "{seller} over listed: {n}");
        }
        assert!(!w.listings.is_empty());
    }

    #[test]
    fn requeue_returns_dead_submission_to_pending() {
        let mut w = WorldState::default();
        w.entities.push(WorldEntity {
            id: "agent-1".to_string(),
            name: "Agent".to_string(),
            entity_type: "agent".to_string(),
            zone_id: "z".to_string(),
            location_id: "l".to_string(),
            state: EntityState::Idle,
            capabilities: vec![],
            needs: vec![],
            wallet: "erd1test".to_string(),
            reputation: 0.0,
            credits: 100,
            activity: String::new(),
            last_move_tick: 0,
            inventory: vec![],
        });
        let proof = w
            .settle_on_chain("quest_completion", "c", "agent-1", 20)
            .unwrap();
        let pid = proof.id.clone();
        w.submit_settlement(&pid, "deadhash", "erd1deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef", Some(5)).unwrap();
        assert_eq!(w.proofs[0].status, SettlementStatus::Submitted);
        // Non-submitted proofs refuse requeue.
        assert!(w.requeue_settlement("missing").is_err());
        w.requeue_settlement(&pid).unwrap();
        assert_eq!(w.proofs[0].status, SettlementStatus::Pending);
        assert!(w.proofs[0].tx_hash.is_empty());
        assert_eq!(w.proofs[0].nonce, None);
        // ...but the intent still rebuilds byte-identical for resubmission.
        let (intent, _) = w
            .submit_intent(&pid, "erd1deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef")
            .unwrap();
        assert_eq!(intent.data_field(), proof.tx_data);
        // Confirmed proofs refuse requeue.
        w.submit_settlement(&pid, "livehash", "erd1deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef", Some(6)).unwrap();
        w.confirm_settlement(&pid).unwrap();
        assert!(w.requeue_settlement(&pid).is_err());
    }

    #[test]
    fn dynamic_pricing_heats_and_cools() {
        let mut w = WorldState::default();
        // Seed a service offer + buyer at that location.
        w.locations.push(WorldLocation {
            id: "loc-dyn".to_string(),
            zone_id: "z".to_string(),
            label: "Dyn".to_string(),
            description: String::new(),
            services: [("svc".to_string(), 20u64)].into_iter().collect(),
            marketplace: false,
            capacity: 10,
        });
        w.entities.push(WorldEntity {
            id: "buyer".to_string(),
            name: "B".to_string(),
            entity_type: "agent".to_string(),
            zone_id: "z".to_string(),
            location_id: "loc-dyn".to_string(),
            state: EntityState::Idle,
            capabilities: vec![],
            needs: vec![],
            wallet: "erd1buyer".to_string(),
            reputation: 0.0,
            credits: 10_000,
            activity: String::new(),
            last_move_tick: 0,
            inventory: vec![],
        });

        assert_eq!(w.service_price("loc-dyn", "svc"), Some(20));
        let (_, _, p1) = w.buy_service("buyer", "loc-dyn", "svc").unwrap();
        assert_eq!(p1, 20);
        // Demand 1 → 20 + 20*1/20 = 21.
        assert_eq!(w.service_price("loc-dyn", "svc"), Some(21));
        // Heat to the 2x cap.
        for _ in 0..30 {
            let (_, _, p) = w.buy_service("buyer", "loc-dyn", "svc").unwrap();
            let _ = p;
        }
        assert_eq!(w.service_price("loc-dyn", "svc"), Some(40));
        // Ticks cool it back to base.
        for _ in 0..40 {
            w.world_tick();
        }
        assert_eq!(w.service_price("loc-dyn", "svc"), Some(20));
        // Unknown service → None, never a panic.
        assert_eq!(w.service_price("loc-dyn", "nope"), None);
    }

    #[test]
    fn trade_proof_uses_trade_namespace_and_merit_gate() {
        let mut w = WorldState::default();
        w.entities.push(WorldEntity {
            id: "npc-smith".to_string(),
            name: "Smith".to_string(),
            entity_type: "npc".to_string(),
            zone_id: "forge".to_string(),
            location_id: "forge-workshop".to_string(),
            state: EntityState::Idle,
            capabilities: vec!["coding".to_string()],
            needs: vec![],
            wallet: "npc:npc-smith".to_string(),
            reputation: 1.0,
            credits: 0,
            activity: String::new(),
            last_move_tick: 0,
            inventory: vec![],
        });

        // Dust stays off-chain.
        assert!(!WorldState::merits_settlement(4));
        assert!(w
            .trade_settlement_proof("service_sale", "dust", "npc-smith", 4)
            .is_none());
        // Real sale settles under the trade namespace.
        assert!(WorldState::merits_settlement(10));
        let proof = w
            .trade_settlement_proof("service_sale", "coding sale", "npc-smith", 10)
            .unwrap();
        assert_eq!(proof.entity_id, "npc-smith");
        let job_hex = hex::encode("trade-npc-smith-0".as_bytes());
        assert!(
            proof.tx_data.starts_with(&format!("submit_proof@{job_hex}@")),
            "unexpected tx_data: {}",
            proof.tx_data
        );
        // Unknown earner → None, never a panic.
        assert!(w
            .trade_settlement_proof("service_sale", "x", "ghost", 10)
            .is_none());
    }

    #[test]
    fn submit_intent_rebuilds_builder_intent_deterministically() {
        let mut w = WorldState::default();
        w.entities.push(WorldEntity {
            id: "agent-1".to_string(),
            name: "Agent".to_string(),
            entity_type: "agent".to_string(),
            zone_id: "central-hub".to_string(),
            location_id: "hub-plaza".to_string(),
            state: EntityState::Idle,
            capabilities: vec![],
            needs: vec![],
            wallet: "erd1test".to_string(),
            reputation: 0.0,
            credits: 100,
            activity: String::new(),
            last_move_tick: 0,
            inventory: vec![],
        });

        let proof = w
            .settle_on_chain("quest_completion", "completed", "agent-1", 20)
            .unwrap();
        let sender = "erd17y5h7t00yd7r7qlfjmndnvhu2yu2arpe9sexj73v42qzgysthpjsk2q033";
        let (intent, payload_hex) = w.submit_intent(&proof.id, sender).unwrap();
        // Same data the proof carries — rebuilt, not re-rolled.
        assert_eq!(intent.data_field(), proof.tx_data);
        assert_eq!(intent.sender.as_deref(), Some(sender));
        assert_eq!(intent.chain_id.as_deref(), Some("T"));
        assert_eq!(intent.network, "multiversx-testnet");
        assert!(!payload_hex.is_empty());
        // Deterministic: second call → identical bytes.
        let (again, payload_again) = w.submit_intent(&proof.id, sender).unwrap();
        assert_eq!(intent, again);
        assert_eq!(payload_hex, payload_again);
        // Guards: bad sender, unknown proof, non-pending proof.
        assert!(w.submit_intent(&proof.id, "").is_err());
        assert!(w.submit_intent(&proof.id, "not-an-address").is_err());
        assert!(w.submit_intent("proof-missing", sender).is_err());
        w.submit_settlement(&proof.id, "deadbeef", sender, Some(9)).unwrap();
        assert_eq!(w.proofs[0].nonce, Some(9));
        assert_eq!(w.proofs[0].sender, sender);
        assert!(w.submit_intent(&proof.id, sender).is_err());
    }
}
