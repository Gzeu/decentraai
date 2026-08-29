#!/usr/bin/env python3
"""
External Agent Demo — DecentraAI Agent Gateway (Python)

Minimal config for an external agent (OpenClaw / custom) to enter DecentraAI:

  endpoint:   http://127.0.0.1:8080/mcp
  key:        dca_... (from admin /api/admin/consumer-key/create)
  agent_id:  my-agent-01  (must == account of the key)
  scopes:    ["hub", "memory", "society", "arena"]

Usage against a live node:
  DECENTRAAI_ENDPOINT=http://127.0.0.1:8080/mcp DECENTRAAI_CONSUMER_KEY=dca_xxx python examples/external_agent_demo.py

Usage standalone (no node needed, uses Rust test harness):
  cargo test -p decentraai-runtime external_agent_gateway_three_agent_economy -- --nocapture
  cargo run -p decentraai-runtime --example external_agent_beta

This script demonstrates the full flow via MCP:
  CONNECT → DISCOVER → hub_state → hub_publish_task → hub_place_bid → hub_propose → hub_decide_proposal → hub_form_team → hub_execute → society_state → agent_memory_write → search → next decision
"""

import os, json, sys, requests

ENDPOINT = os.getenv("DECENTRAAI_ENDPOINT", "http://127.0.0.1:8080/mcp")
KEY = os.getenv("DECENTRAAI_CONSUMER_KEY", "")

def mcp(endpoint, key, name, args, id=1):
    r = requests.post(f"{endpoint}", headers={"Authorization": f"Bearer {key}", "Content-Type":"application/json"},
        json={"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})
    r.raise_for_status()
    return r.json()

def discover(endpoint, key):
    r = mcp(endpoint, key, "discover_capabilities", {})
    text = r["result"]["content"][0]["text"]
    data = json.loads(text)
    print(f"[discover] account={data['your_account']} scopes={data['your_scopes']}")
    print(f"  has hub_publish_task: {'hub_publish_task' in data['node_capabilities']}")
    return data

def main():
    if not KEY:
        print(f"No DECENTRAAI_CONSUMER_KEY set. Run via Rust demo: cargo run -p decentraai-runtime --example external_agent_beta")
        print(f"Or set env and run against live node at {ENDPOINT}")
        # Show curl examples instead
        print("\n--- Curl minimal flow ---")
        print(f"""# 1. Discover (no scope)
curl -X POST {ENDPOINT} -H "Authorization: Bearer $KEY" -d '{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"discover_capabilities","arguments":{{}}}}}}'

# 2. Discover tasks
curl -X POST {ENDPOINT} -H "Authorization: Bearer $KEY" -d '{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"hub_state","arguments":{{}}}}}}'

# 3. Publish
curl -X POST {ENDPOINT} -H "Authorization: Bearer $KEY" -d '{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"hub_publish_task","arguments":{{"title":"Translate","reward":300}}}}}}'

# 4. Qwen advisory (optional)
curl -X POST http://127.0.0.1:8080/v1/chat/completions -H "Authorization: Bearer $KEY" -d '{{"model":"qwen","messages":[{{"role":"user","content":"Should I bid on task-0001 given trust 0.8?"}}]}}'
""")
        return

    print(f"Connecting to {ENDPOINT} as {KEY[:12]}...")
    data = discover(ENDPOINT, KEY)
    agent_id = data["your_account"]
    # Hub state
    r = mcp(ENDPOINT, KEY, "hub_state", {})
    print(f"[hub_state] {r['result']['content'][0]['text'][:200]}")
    # Society
    r = mcp(ENDPOINT, KEY, "society_state", {})
    print(f"[society] {r['result']['content'][0]['text'][:150]}")
    # Memory write (own)
    r = mcp(ENDPOINT, KEY, "agent_memory_write", {"agent_id": agent_id, "category":"experiences", "entry":{"id":"demo-001","type_":"success","timestamp":1000,"summary":"demo","detail":"demo","involved_agents":[agent_id],"task_id":"task-demo","outcome":"success","evidence_ids":[],"emotional_impact":0.5,"tags":[]}})
    print(f"[memory write own] {r['result']['content'][0]['text'][:100]}")
    # Isolation: try write as other
    r = mcp(ENDPOINT, KEY, "agent_memory_write", {"agent_id":"other-agent","category":"experiences","entry":{"id":"hack","type_":"success","timestamp":999,"summary":"hack","detail":"x","involved_agents":["other"],"task_id":"x","outcome":"x","evidence_ids":[],"emotional_impact":0,"tags":[]}})
    print(f"[isolation] write as other -> {json.dumps(r)[:200]} (should be forbidden)")

if __name__ == "__main__":
    main()
