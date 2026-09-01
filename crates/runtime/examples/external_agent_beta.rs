//! External Agent Beta — real external agent demo via consumer `dca_...` + MCP `/mcp`
//! Run: cargo run -p decentraai-runtime --example external_agent_beta -- --nocapture
//! Or: cargo test -p decentraai-runtime external_agent_gateway_three_agent_economy -- --nocapture
//!
//! This example starts a live ApiState (hub + society + personal memory + quota ledger)
//! and simulates 3 external agents (A/B/C) connecting ONLY via consumer keys + MCP.
//! Each agent's decision is advised by Qwen when available (POST /v1/chat/completions),
//! otherwise deterministic fallback with same shape — so the demo is reproducible
//! but shows where Qwen plugs in.

use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Reuse the same test harness that the automated test uses, but as a runnable demo
    // so you can see the flow with `cargo run --example`.
    println!("=== DecentraAI External Agent Beta — 3-agent economy ===\n");
    println!(
        "This demo mirrors `cargo test external_agent_gateway_three_agent_economy -- --nocapture`"
    );
    println!("but as a standalone binary you can point at a live node via DECENTRAAI_ENDPOINT.\n");

    // If DECENTRAAI_ENDPOINT is set, run against a live node (real external agent).
    // Otherwise, spin an ephemeral node (like the test) so the demo is self-contained.
    if let Ok(endpoint) = std::env::var("DECENTRAAI_ENDPOINT") {
        let key = std::env::var("DECENTRAAI_CONSUMER_KEY").expect("set DECENTRAAI_CONSUMER_KEY");
        println!(
            "→ Live mode: endpoint={endpoint}, key={}...",
            &key[..8.min(key.len())]
        );
        live_demo(endpoint, key).await?;
    } else {
        ephemeral_demo().await?;
    }
    Ok(())
}

async fn live_demo(endpoint: String, key: String) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let payload = serde_json::json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"discover_capabilities","arguments":{}}
    });
    let r: serde_json::Value = client
        .post(format!("{endpoint}/mcp"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&payload)
        .send()
        .await?
        .json()
        .await?;
    println!(
        "discover_capabilities → {}",
        serde_json::to_string_pretty(&r)?
    );
    println!("\nNext steps for your agent (copy-paste):");
    println!(
        "  curl -X POST $ENDPOINT/mcp -H \"Authorization: Bearer $KEY\" -d '{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"hub_state\",\"arguments\":{{}}}}}}'"
    );
    println!(
        "  ... then hub_publish_task / hub_place_bid / hub_propose / hub_form_team / hub_execute"
    );
    Ok(())
}

async fn ephemeral_demo() -> anyhow::Result<()> {
    use decentraai_compute::ContributionPolicy;
    use decentraai_compute::QuotaLedger;
    use decentraai_runtime::api::{ApiState, DashboardInfo, serve_api};
    use decentraai_runtime::queue::InferenceQueue;

    // Minimal helpers mirroring api::tests
    async fn start_backend() -> String {
        // tiny fake engine is enough for ApiState to start; Qwen is optional advisory
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new()
            .route("/v1/models", axum::routing::get(|| async { "{\"object\":\"list\",\"data\":[]}" }))
            .route("/v1/chat/completions", axum::routing::post(|| async {
                "{\"choices\":[{\"message\":{\"content\":\"{\\\"action\\\":\\\"hub_place_bid\\\", \\\"price\\\":240}\"}}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":10}}"
            }));
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    let dir = TempDir::new().unwrap();
    let master = "master-demo".to_string();
    let backend = start_backend().await;
    let manager = {
        // reuse test_manager logic inline (fake llama-server)
        use decentraai_runtime::{LlamaServer, RuntimeConfig, ServeManager};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let tmp = dir.path().join("model.gguf");
        std::fs::write(&tmp, b"fake").unwrap();
        let binary = {
            let p = dir.path().join("fake-server");
            std::fs::write(&p, b"#!/bin/sh\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            p
        };
        let mut cfg = RuntimeConfig::new(tmp);
        cfg.port = Some(addr.port());
        let srv = LlamaServer::start(&binary, &cfg).expect("fake server");
        let app = axum::Router::new()
            .route(
                "/v1/models",
                axum::routing::get(|| async { "{\"object\":\"list\",\"data\":[]}" }),
            )
            .route(
                "/v1/chat/completions",
                axum::routing::post(|| async { "{\"choices\":[]}" }),
            );
        let listener2 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _ = listener2;
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Arc::new(Mutex::new(ServeManager::new(
            srv,
            Duration::from_secs(3600),
        )))
    };
    let ledger = Arc::new(std::sync::Mutex::new(QuotaLedger::new(
        ContributionPolicy::default(),
    )));
    {
        let mut l = ledger.lock().unwrap();
        for acc in ["agent-a", "agent-b", "agent-c"] {
            l.credit(&acc.to_string(), "seed", Some(5000), None);
        }
    }
    let mut state = ApiState::new(
        backend.clone(),
        Some(master.clone()),
        manager.clone(),
        DashboardInfo {
            repo_root: dir.path().to_path_buf(),
            reputation_path: None,
            max_invalid_chunks: 3,
            ban_duration: Duration::from_secs(3600),
            api_port: 0,
            model_name: "qwen-demo".into(),
            model_size_bytes: 1024,
            generation: decentraai_config::GenerationSection {
                temperature: 0.7,
                top_p: 0.9,
                top_k: Some(40),
                repeat_penalty: 1.1,
                system_prompt: "Test system line.".to_string(),
            },
            resources: decentraai_config::ResourceSection {
                cpu_max_percent: 65,
                reserve_cpu_cores: 2,
                memory_max_percent: 60,
                reserve_ram_mb: 4096,
                gpu_enabled: decentraai_config::GpuPolicy::Auto,
                gpu_max_vram_percent: 75,
                reserve_vram_mb: 1536,
                stop_gpu_temperature_celsius: 83,
                max_upload_mbps: 20,
                max_download_mbps: 80,
            },
            dht_enabled: false,
            relay_enabled: false,
            lan_discovery: true,
            bootstrap_peer_count: 0,
        },
        None,
        None,
        InferenceQueue::new(5, Duration::from_secs(5)),
        None,
        None,
    );
    state.attach_consumer(
        Some(dir.path().join("db/consumer_keys.json")),
        Some(ledger.clone()),
    );
    let pm = Arc::new(decentraai_agent_personal_memory::PersonalMemoryStore::new(
        dir.path().join("memory"),
    ));
    state.attach_personal_memory(pm);
    let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
    let client = reqwest::Client::new();

    async fn create_key(
        client: &reqwest::Client,
        api: std::net::SocketAddr,
        master: &str,
        account: &str,
    ) -> String {
        let r: serde_json::Value = client.post(format!("http://{api}/api/admin/consumer-key/create"))
            .header("Authorization", format!("Bearer {master}"))
            .json(&serde_json::json!({"account":account,"quota_ceiling":1000,"rate_limit_per_minute":100,"scopes":["hub","memory","society","arena"]}))
            .send().await.unwrap().json().await.unwrap();
        r["token"].as_str().unwrap().to_string()
    }
    async fn mcp(
        client: &reqwest::Client,
        api: std::net::SocketAddr,
        key: &str,
        name: &str,
        args: serde_json::Value,
    ) -> serde_json::Value {
        client.post(format!("http://{api}/mcp"))
            .header("Authorization", format!("Bearer {key}"))
            .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}))
            .send().await.unwrap().json().await.unwrap()
    }

    let ka = create_key(&client, api, &master, "agent-a").await;
    let kb = create_key(&client, api, &master, "agent-b").await;
    let kc = create_key(&client, api, &master, "agent-c").await;
    println!(
        "Created consumer keys: A={}..., B={}..., C={}...",
        &ka[..12],
        &kb[..12],
        &kc[..12]
    );

    // Qwen advisory helper (tries real inference, fallback deterministic)
    async fn advise(client: &reqwest::Client, backend: &str, prompt: &str) -> String {
        if let Ok(r) = client.post(format!("{backend}/v1/chat/completions"))
            .json(&serde_json::json!({"model":"qwen","messages":[{"role":"user","content":prompt}],"max_tokens":64}))
            .send().await {
                if let Ok(v) = r.json::<serde_json::Value>().await {
                    if let Some(c) = v["choices"][0]["message"]["content"].as_str() { return c.chars().take(200).collect(); }
                }
        }
        format!(
            "advisory fallback for: {}",
            prompt.chars().take(80).collect::<String>()
        )
    }

    // 1. Discover
    for (id, key) in [("agent-a", &ka), ("agent-b", &kb), ("agent-c", &kc)] {
        let r = mcp(
            &client,
            api,
            key,
            "discover_capabilities",
            serde_json::json!({}),
        )
        .await;
        println!(
            "\n[{}] discover_capabilities → has hub={}, memory={}, society={}",
            id,
            r.to_string().contains("hub_publish_task"),
            r.to_string().contains("agent_memory"),
            r.to_string().contains("society_state")
        );
        let _ = advise(
            &client,
            &backend,
            &format!("Agent {id} discovered capabilities"),
        )
        .await;
    }
    // 2. Publish, 3. Discover, 4. Bid, 5. Negotiate, 6. Team, 7. Execute
    let task: serde_json::Value = serde_json::from_str(
        mcp(
            &client,
            api,
            &ka,
            "hub_publish_task",
            serde_json::json!({"title":"Translate","reward":300}),
        )
        .await["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let tid = task["id"].as_str().unwrap().to_string();
    println!("\n[A] published {tid}");
    let _ = mcp(&client, api, &kb, "hub_state", serde_json::json!({})).await;
    let _ = mcp(&client, api, &kc, "hub_state", serde_json::json!({})).await;
    println!(
        "[B] bid {}",
        mcp(
            &client,
            api,
            &kb,
            "hub_place_bid",
            serde_json::json!({"task_id":tid,"price":250})
        )
        .await["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
    );
    println!(
        "[C] bid {}",
        mcp(
            &client,
            api,
            &kc,
            "hub_place_bid",
            serde_json::json!({"task_id":tid,"price":200})
        )
        .await["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
    );
    let prop: serde_json::Value = serde_json::from_str(
        mcp(
            &client,
            api,
            &ka,
            "hub_propose",
            serde_json::json!({"to":"agent-b","task_id":tid,"offer_price":150,"workshare":60}),
        )
        .await["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let pid = prop["id"].as_str().unwrap();
    println!(
        "[A→B] propose {pid} → {}",
        mcp(
            &client,
            api,
            &kb,
            "hub_decide_proposal",
            serde_json::json!({"proposal_id":pid,"accept":true})
        )
        .await["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
    );
    println!(
        "[A] form_team {}",
        mcp(
            &client,
            api,
            &ka,
            "hub_form_team",
            serde_json::json!({"task_id":tid,"members":[["agent-a",40],["agent-b",60]]})
        )
        .await["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
    );
    let exec: serde_json::Value = serde_json::from_str(
        mcp(
            &client,
            api,
            &ka,
            "hub_execute",
            serde_json::json!({"task_id":tid}),
        )
        .await["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    println!(
        "[A] execute → evidence {} settlement team {:?}",
        exec["evidence_id"].as_str().unwrap(),
        exec["team"]
    );

    // 8. Memory + isolation + next decision uses history
    for (key, acc) in [(&ka, "agent-a"), (&kb, "agent-b")] {
        let r = mcp(&client, api, key, "agent_memory_write", serde_json::json!({"agent_id":acc,"category":"experiences","entry":{"id":format!("exp-{acc}-{tid}"),"type_":"success","timestamp":1000,"summary":format!("Task {tid} with team"),"detail":"ok","involved_agents":[acc],"task_id":tid,"outcome":"success","evidence_ids":[],"emotional_impact":0.8,"tags":["collab"]}})).await;
        println!(
            "[{acc}] memory write → {}",
            r["result"]["content"][0]["text"].as_str().unwrap()
        );
    }
    // isolation proof
    let forbid = mcp(&client, api, &ka, "agent_memory_write", serde_json::json!({"agent_id":"agent-b","category":"experiences","entry":{"id":"hack","type_":"success","timestamp":999,"summary":"hack","detail":"x","involved_agents":["agent-b"],"task_id":"x","outcome":"x","evidence_ids":[],"emotional_impact":0.0,"tags":[]}})).await;
    println!(
        "\n[Isolation] A writes to B → {}",
        serde_json::to_string(&forbid)
            .unwrap()
            .chars()
            .take(120)
            .collect::<String>()
    );

    let search = mcp(
        &client,
        api,
        &kb,
        "agent_memory_search",
        serde_json::json!({"agent_id":"agent-b","query":tid}),
    )
    .await;
    println!(
        "[B] search memory for {tid} → {}",
        search["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .chars()
            .take(150)
            .collect::<String>()
    );

    // Second task where history matters — Qwen advisory includes memory
    let task2: serde_json::Value = serde_json::from_str(
        mcp(
            &client,
            api,
            &kb,
            "hub_publish_task",
            serde_json::json!({"title":"Second","reward":500}),
        )
        .await["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let tid2 = task2["id"].as_str().unwrap();
    let hints = mcp(&client, api, &ka, "society_decision_hints", serde_json::json!({"agent_id":"agent-a","hub_state":{},"resources":{"quota_available":5120,"quota_ceiling":10000,"capacity_used":0.2,"max_concurrent_tasks":5,"current_tasks":1}})).await;
    println!(
        "\n[A] decision_hints for next task {} → {}",
        tid2,
        hints["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .chars()
            .take(150)
            .collect::<String>()
    );
    let qwen_advice = advise(
        &client,
        &backend,
        &format!("Should agent-a bid on {tid2} given history with agent-b?"),
    )
    .await;
    println!("[Qwen advisory] {}", qwen_advice);

    println!(
        "\n=== Beta ready: give external agent endpoint http://{api}/mcp + dca_ key with scopes hub,memory,society,arena ==="
    );
    Ok(())
}
