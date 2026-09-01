use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::{RwLock, broadcast};
use uuid::Uuid;

type AgentId = String;

#[derive(Debug, Error)]
pub enum EventBusError {
    #[error("Subscription not found: {0}")]
    SubscriptionNotFound(String),
    #[error("Channel closed")]
    ChannelClosed,
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct EventId(pub String);

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl EventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Topic(pub String);

impl Topic {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn agent(agent_id: &str) -> Self {
        Self(format!("agent/{}", agent_id))
    }

    pub fn hub() -> Self {
        Self("hub".to_string())
    }

    pub fn society() -> Self {
        Self("society".to_string())
    }

    pub fn arena() -> Self {
        Self("arena".to_string())
    }

    pub fn memory() -> Self {
        Self("memory".to_string())
    }

    pub fn system() -> Self {
        Self("system".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    pub topic: Topic,
    pub source: AgentId,
    pub timestamp: u64,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub metadata: EventMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EventMetadata {
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub tags: Vec<String>,
    pub priority: EventPriority,
    pub ttl_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum EventPriority {
    Low = 0,
    #[default]
    Normal = 1,
    High = 2,
    Critical = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EventFilter {
    pub topics: Option<Vec<Topic>>,
    pub sources: Option<Vec<AgentId>>,
    pub event_types: Option<Vec<String>>,
    pub since_timestamp: Option<u64>,
    pub until_timestamp: Option<u64>,
    pub min_priority: Option<EventPriority>,
    pub tags: Option<Vec<String>>,
}

impl EventFilter {
    pub fn primary_topic(filter: &EventFilter) -> Option<Topic> {
        filter.topics.as_ref().and_then(|t| t.first().cloned())
    }

    pub fn for_agent(agent_id: &AgentId) -> Self {
        Self {
            sources: Some(vec![agent_id.clone()]),
            ..Default::default()
        }
    }

    pub fn for_topic(topic: Topic) -> Self {
        Self {
            topics: Some(vec![topic]),
            ..Default::default()
        }
    }

    pub fn since(timestamp: u64) -> Self {
        Self {
            since_timestamp: Some(timestamp),
            ..Default::default()
        }
    }

    pub fn with_types(mut self, types: Vec<String>) -> Self {
        self.event_types = Some(types);
        self
    }

    pub fn with_priority(mut self, priority: EventPriority) -> Self {
        self.min_priority = Some(priority);
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = Some(tags);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub subscriber: AgentId,
    pub filter: EventFilter,
    pub created_at: u64,
    pub last_delivered: Option<u64>,
}

#[async_trait::async_trait]
pub trait EventHandler: Send + Sync {
    async fn handle(&self, event: &Event) -> Result<(), EventBusError>;
    fn subscribed_topics(&self) -> Vec<Topic>;
    fn can_handle(&self, event: &Event) -> bool {
        self.subscribed_topics().contains(&event.topic)
    }
}

#[async_trait::async_trait]
pub trait EventStore: Send + Sync {
    async fn append(&self, event: &Event) -> Result<(), EventBusError>;
    async fn query(&self, filter: &EventFilter, limit: usize) -> Result<Vec<Event>, EventBusError>;
    async fn get_since(
        &self,
        since_timestamp: u64,
        limit: usize,
    ) -> Result<Vec<Event>, EventBusError>;
    async fn prune_before(&self, timestamp: u64) -> Result<u64, EventBusError>;
    async fn get_latest_timestamp(&self) -> Result<u64, EventBusError>;
}

pub struct EventBus {
    subscriptions: Arc<DashMap<String, Subscription>>,
    topic_subscribers: Arc<DashMap<Topic, Vec<String>>>,
    broadcast_tx: broadcast::Sender<Event>,
    event_store: Arc<dyn EventStore>,
    #[allow(dead_code)]
    max_history: usize,
}

impl EventBus {
    pub fn new(event_store: Arc<dyn EventStore>) -> Self {
        let (tx, _) = broadcast::channel(10000);
        Self {
            subscriptions: Arc::new(DashMap::new()),
            topic_subscribers: Arc::new(DashMap::new()),
            broadcast_tx: tx,
            event_store,
            max_history: 10000,
        }
    }

    pub async fn publish(&self, event: Event) -> Result<(), EventBusError> {
        let stored_event = event.clone();
        self.event_store.append(&stored_event).await?;

        #[allow(clippy::let_underscore_future)]
        let _ = self.broadcast_tx.send(event.clone());

        if let Some(subscriber_ids) = self.topic_subscribers.get(&event.topic) {
            for sub_id in subscriber_ids.value() {
                if let Some(sub) = self.subscriptions.get(sub_id) {
                    let filter = sub.value().filter.clone();
                    if Self::matches_filter(&event, &filter) {}
                }
            }
        }

        Ok(())
    }

    fn matches_filter(event: &Event, filter: &EventFilter) -> bool {
        if let Some(topics) = &filter.topics
            && !topics.contains(&event.topic)
        {
            return false;
        }
        if let Some(sources) = &filter.sources
            && !sources.contains(&event.source)
        {
            return false;
        }
        if let Some(event_types) = &filter.event_types
            && !event_types.contains(&event.event_type)
        {
            return false;
        }
        if let Some(since) = filter.since_timestamp
            && event.timestamp < since
        {
            return false;
        }
        if let Some(until) = filter.until_timestamp
            && event.timestamp > until
        {
            return false;
        }
        if let Some(min_priority) = filter.min_priority
            && event.metadata.priority < min_priority
        {
            return false;
        }
        if let Some(tags) = &filter.tags
            && !tags.iter().any(|t| event.metadata.tags.contains(t))
        {
            return false;
        }
        true
    }

    pub async fn subscribe(
        &self,
        subscriber: AgentId,
        filter: EventFilter,
    ) -> Result<String, EventBusError> {
        let sub_id = Uuid::new_v4().to_string();
        let subscription = Subscription {
            id: sub_id.clone(),
            subscriber: subscriber.clone(),
            filter,
            created_at: current_timestamp(),
            last_delivered: None,
        };

        self.subscriptions
            .insert(sub_id.clone(), subscription.clone());

        let topic = EventFilter::primary_topic(&subscription.filter).unwrap_or(Topic::system());
        self.topic_subscribers
            .entry(topic)
            .or_default()
            .push(sub_id.clone());

        Ok(sub_id)
    }

    pub async fn unsubscribe(&self, subscription_id: &str) -> Result<(), EventBusError> {
        if let Some((_, sub)) = self.subscriptions.remove(subscription_id)
            && let Some(mut topics) = self
                .topic_subscribers
                .get_mut(&EventFilter::primary_topic(&sub.filter).unwrap_or(Topic::system()))
        {
            topics.retain(|id| id != subscription_id);
        }
        Ok(())
    }

    pub async fn get_events(
        &self,
        filter: EventFilter,
        limit: usize,
    ) -> Result<Vec<Event>, EventBusError> {
        self.event_store.query(&filter, limit).await
    }

    pub async fn get_since(&self, since: u64, limit: usize) -> Result<Vec<Event>, EventBusError> {
        self.event_store.get_since(since, limit).await
    }

    pub async fn get_subscription(&self, sub_id: &str) -> Option<Subscription> {
        self.subscriptions.get(sub_id).map(|s| s.value().clone())
    }

    pub async fn list_subscriptions(&self, agent_id: &AgentId) -> Vec<Subscription> {
        self.subscriptions
            .iter()
            .filter(|s| s.value().subscriber == *agent_id)
            .map(|s| s.value().clone())
            .collect()
    }

    pub fn subscribe_broadcast(&self) -> broadcast::Receiver<Event> {
        self.broadcast_tx.subscribe()
    }

    /// Non-blocking publish. Used by callers that must not await
    /// (e.g. the agent runtime emitting lifecycle events). The
    /// event is pushed onto the broadcast channel immediately. The
    /// store append is *not* performed here (callers that need
    /// durable persistence should use the async `publish`).
    pub fn try_publish(&self, event: Event) -> Result<(), EventBusError> {
        // Broadcast is synchronous and bounded. If the channel is
        // full or has no receivers, send returns Err — we treat that
        // as a dropped event (the bus does not block callers).
        #[allow(clippy::let_underscore_future)]
        let _ = self.broadcast_tx.send(event);
        Ok(())
    }
}

pub struct InMemoryEventStore {
    events: Arc<RwLock<VecDeque<Event>>>,
    max_size: usize,
}

impl InMemoryEventStore {
    pub fn new(max_size: usize) -> Self {
        Self {
            events: Arc::new(RwLock::new(VecDeque::with_capacity(max_size))),
            max_size,
        }
    }
}

#[async_trait::async_trait]
impl EventStore for InMemoryEventStore {
    async fn append(&self, event: &Event) -> Result<(), EventBusError> {
        let mut events = self.events.write().await;
        if events.len() >= self.max_size {
            events.pop_front();
        }
        events.push_back(event.clone());
        Ok(())
    }

    async fn query(&self, filter: &EventFilter, limit: usize) -> Result<Vec<Event>, EventBusError> {
        let events = self.events.read().await;
        let mut results = Vec::new();
        for event in events.iter().rev() {
            if EventBus::matches_filter(event, filter) {
                results.push(event.clone());
                if results.len() >= limit {
                    break;
                }
            }
        }
        results.reverse();
        Ok(results)
    }

    async fn get_since(
        &self,
        since_timestamp: u64,
        limit: usize,
    ) -> Result<Vec<Event>, EventBusError> {
        let events = self.events.read().await;
        let mut results = Vec::new();
        for event in events.iter().rev() {
            if event.timestamp >= since_timestamp {
                results.push(event.clone());
                if results.len() >= limit {
                    break;
                }
            }
        }
        results.reverse();
        Ok(results)
    }

    async fn prune_before(&self, timestamp: u64) -> Result<u64, EventBusError> {
        let mut events = self.events.write().await;
        let initial_len = events.len();
        while let Some(front) = events.front() {
            if front.timestamp < timestamp {
                events.pop_front();
            } else {
                break;
            }
        }
        let removed = initial_len - events.len();
        Ok(removed as u64)
    }

    async fn get_latest_timestamp(&self) -> Result<u64, EventBusError> {
        let events = self.events.read().await;
        Ok(events.back().map(|e| e.timestamp).unwrap_or(0))
    }
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_bus_publish_subscribe() {
        let store = Arc::new(InMemoryEventStore::new(1000));
        let bus = EventBus::new(store.clone());

        let _sub_id = bus
            .subscribe(
                AgentId::from("agent-1"),
                EventFilter::for_topic(Topic::hub()),
            )
            .await
            .unwrap();

        let event = Event {
            id: EventId::new(),
            topic: Topic::hub(),
            source: AgentId::from("agent-1"),
            timestamp: current_timestamp(),
            event_type: "task_published".to_string(),
            payload: serde_json::json!({"task_id": "task-1"}),
            metadata: EventMetadata::default(),
        };

        bus.publish(event).await.unwrap();

        let events = store
            .query(&EventFilter::for_topic(Topic::hub()), 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_event_filter_matching() {
        let event = Event {
            id: EventId::new(),
            topic: Topic::hub(),
            source: AgentId::from("agent-1"),
            timestamp: 1000,
            event_type: "task_published".to_string(),
            payload: serde_json::json!({}),
            metadata: EventMetadata {
                priority: EventPriority::High,
                tags: vec!["important".to_string()],
                ..Default::default()
            },
        };

        let filter = EventFilter {
            topics: Some(vec![Topic::hub()]),
            min_priority: Some(EventPriority::Normal),
            tags: Some(vec!["important".to_string()]),
            ..Default::default()
        };

        assert!(EventBus::matches_filter(&event, &filter));

        let filter_no_match = EventFilter {
            topics: Some(vec![Topic::arena()]),
            ..Default::default()
        };
        assert!(!EventBus::matches_filter(&event, &filter_no_match));
    }

    // Persistence: events stored in InMemoryEventStore survive the
    // EventBus instance being torn down and rebuilt against the same
    // store. This is the foundation for the Sprint 0.1 restart
    // contract: when the daemon restarts, the audit_bridge can be
    // re-attached and the new EventBus will see the same events.
    #[tokio::test]
    async fn events_persist_across_eventbus_recreation() {
        let store = Arc::new(InMemoryEventStore::new(1024));
        let bus1 = EventBus::new(store.clone());
        let event = Event {
            id: EventId::new(),
            topic: Topic::hub(),
            source: AgentId::from("agent-1"),
            timestamp: 12345,
            event_type: "task_published".to_string(),
            payload: serde_json::json!({"task_id": "task-99"}),
            metadata: EventMetadata::default(),
        };
        bus1.publish(event).await.unwrap();
        drop(bus1);

        // Same store, new bus. The event must still be queryable.
        let bus2 = EventBus::new(store.clone());
        let filter = EventFilter::for_topic(Topic::hub());
        let events = bus2.get_events(filter, 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "task_published");
        assert_eq!(events[0].timestamp, 12345);
    }

    // Lifecycle: prune_before removes old events but keeps new ones.
    #[tokio::test]
    async fn prune_before_keeps_new_events() {
        let store = Arc::new(InMemoryEventStore::new(1024));
        let bus = EventBus::new(store.clone());
        for ts in [100, 200, 300, 400, 500].iter() {
            let event = Event {
                id: EventId::new(),
                topic: Topic::system(),
                source: AgentId::from("test"),
                timestamp: *ts,
                event_type: "tick".to_string(),
                payload: serde_json::json!({"t": ts}),
                metadata: EventMetadata::default(),
            };
            bus.publish(event).await.unwrap();
        }
        let removed = store.prune_before(300).await.unwrap();
        assert_eq!(removed, 2);
        let filter = EventFilter::for_topic(Topic::system());
        let events = bus.get_events(filter, 10).await.unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].timestamp, 300);
        assert_eq!(events[1].timestamp, 400);
        assert_eq!(events[2].timestamp, 500);
    }
}

// Sprint 0.1: dual-write bridge to the audit log.
pub mod audit_bridge;
