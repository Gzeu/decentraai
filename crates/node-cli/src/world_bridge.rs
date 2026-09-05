//! World ↔ Primordial Mind bridge (operator side, node-cli only).
//!
//! Connects the autonomous research cycle to the REAL World surface:
//! - `GET  /v1/world`          → typed observation (no hand-made files);
//! - `POST /v1/world/mission`  → research becomes a real World activity
//!   (hub task + reward + world record), not a parallel simulation.
//!
//! The bridge NEVER writes world files itself and never bypasses the
//! node's auth: it is a client of the same API any operator/agent uses.
//! Canonical research state stays in proposal (curiosity/journal/store);
//! World sees projections (mission entity + event) of that state.

use anyhow::{Context, Result};
use serde_json::Value;

/// Build a typed observation from the real world snapshot.
/// Deterministic shaping over whatever the node actually exposes:
/// `tick`, entity/event counts, mission presence — no invented facts.
pub fn observation_from_world(world: &Value) -> Result<Value> {
    let tick = world
        .get("tick")
        .and_then(Value::as_u64)
        .context("world snapshot lacks tick")?;
    let entities = world
        .get("entities")
        .or_else(|| world.get("agents"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let events = world
        .get("events")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let mission = world.get("mission_task_id").is_some_and(|v| !v.is_null());
    // Treasury counters live in the raw projection file; when the API
    // exposes them (nested or flat), include — otherwise omit honestly.
    let t = world.get("economy").and_then(|e| e.get("treasury"));
    let minted = world
        .get("treasury_minted")
        .or_else(|| t.and_then(|x| x.get("minted")))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let burned = world
        .get("treasury_burned")
        .or_else(|| t.and_then(|x| x.get("burned")))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Ok(serde_json::json!({
        "id": format!("obs:world:{tick}"),
        "text": format!(
            "world tick {tick} entities {entities} events {events} mission {} minted {minted} burned {burned}",
            if mission { 1 } else { 0 }
        ),
        "source": "world",
    }))
}

/// Build the mission POST body for a selected research decision.
/// Reward is bounded small (world credits, NOT DCAI, NOT xEGLD); it
/// marks the activity in the economy-of-work without touching treasury.
#[must_use]
pub fn mission_body(title: &str, description: &str, reward: u64) -> Value {
    serde_json::json!({
        "title": title.chars().take(128).collect::<String>(),
        "description": description.chars().take(512).collect::<String>(),
        "reward": reward.clamp(1, 100_000),
    })
}

/// GET /v1/world → observation JSON (typed).
pub async fn fetch_world_observation(base: &str, token: &str) -> Result<Value> {
    let url = format!("{}/v1/world", base.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .with_context(|| format!("GET {url} failed"))?;
    if !resp.status().is_success() {
        anyhow::bail!("GET /v1/world → HTTP {}", resp.status());
    }
    let world: Value = resp.json().await.context("world snapshot invalid JSON")?;
    observation_from_world(&world)
}

/// POST /v1/world/mission — the research cycle becomes a World activity.
/// 409 (mission already open) is NOT an error: the agent reports the
/// existing mission id and continues with its own cycle.
pub async fn post_research_mission(
    base: &str,
    token: &str,
    body: &Value,
) -> Result<Option<String>> {
    let url = format!("{}/v1/world/mission", base.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .json(body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .with_context(|| format!("POST {url} failed"))?;
    match resp.status().as_u16() {
        200 | 201 => {
            let v: Value = resp.json().await.unwrap_or_default();
            Ok(v.get("task_id").and_then(Value::as_str).map(str::to_string))
        }
        409 => {
            eprintln!("world: mission already open (409) — agent continues its cycle");
            Ok(None)
        }
        s => anyhow::bail!("POST /v1/world/mission → HTTP {s}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> Value {
        serde_json::json!({
            "tick": 344,
            "entities": [{"id": "a"}, {"id": "b"}],
            "events": [{"kind": "x"}],
            "agents": [{"id": "a"}, {"id": "b"}],
            "economy": {"treasury": {"minted": 6090, "burned": 255}}
        })
    }

    #[test]
    fn observation_is_typed_and_deterministic() {
        let a = observation_from_world(&world()).unwrap();
        let b = observation_from_world(&world()).unwrap();
        assert_eq!(a, b);
        assert_eq!(a["id"], "obs:world:344");
        assert_eq!(a["source"], "world");
        assert!(a["text"].as_str().unwrap().contains("tick 344"));
        assert!(a["text"].as_str().unwrap().contains("entities 2"));
        assert!(a["text"].as_str().unwrap().contains("minted 6090"));
    }

    #[test]
    fn missing_tick_fails_closed() {
        let bad = serde_json::json!({"treasury_minted": 1});
        assert!(observation_from_world(&bad).is_err());
    }

    #[test]
    fn mission_body_bounds_reward_and_title() {
        let b = mission_body("research X", "why", 10_000_000);
        assert_eq!(b["reward"], 100_000);
        assert!(b["title"].as_str().unwrap().len() <= 128);
        let long = "x".repeat(600);
        let b2 = mission_body(&long, &long, 5);
        assert_eq!(b2["title"].as_str().unwrap().len(), 128);
        assert_eq!(b2["description"].as_str().unwrap().len(), 512);
    }
}
