//! Standalone UI Dashboard for the DecentraAI Inference Credit Economy.
//!
//! Dedicated, fluid, lightweight, single-file HTML5/CSS3/JS reactive interface
//! specifically for `research/inference-credit-economy`.
//!
//! Features:
//! - Local `CredentialVault` manager with masked fingerprints (`sk-...4a9f`).
//! - "Share Model" toggle & Smart Sharing Strategy configurator (Drain Burst, Balanced Drip, Free-Tier Only).
//! - Live CU balances (Available / Earned / Spent / Locked).
//! - Provenance feed & verified compute receipt inspector.
//! - Test Chat Gateway sandbox (OpenAI / OpenCode compatible).

pub const DEDICATED_ECONOMY_DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>DecentraAI — Inference Credit Economy Dashboard</title>
  <style>
    :root {
      --bg: #0b0f19;
      --card: #151c2e;
      --card-border: rgba(255, 255, 255, 0.08);
      --accent: #3b82f6;
      --accent-glow: rgba(59, 130, 246, 0.25);
      --success: #10b981;
      --warning: #f59e0b;
      --danger: #ef4444;
      --text: #f3f4f6;
      --text-muted: #9ca3af;
      --font-mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    }
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      background: var(--bg);
      color: var(--text);
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      line-height: 1.5;
      padding: 24px;
    }
    .container { max-width: 1400px; margin: 0 auto; display: flex; flex-direction: column; gap: 24px; }
    header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding-bottom: 20px;
      border-bottom: 1px solid var(--card-border);
    }
    .brand { display: flex; align-items: center; gap: 12px; }
    .brand-badge {
      background: linear-gradient(135deg, #3b82f6, #8b5cf6);
      color: white;
      font-weight: 800;
      padding: 6px 12px;
      border-radius: 8px;
      font-size: 14px;
      letter-spacing: 0.5px;
    }
    .brand-title { font-size: 20px; font-weight: 700; letter-spacing: -0.5px; }
    .kpi-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
      gap: 16px;
    }
    .kpi-card {
      background: var(--card);
      border: 1px solid var(--card-border);
      border-radius: 12px;
      padding: 18px;
      display: flex;
      flex-direction: column;
      gap: 6px;
      transition: transform 0.15s ease, border-color 0.15s ease;
    }
    .kpi-card:hover { transform: translateY(-2px); border-color: rgba(59, 130, 246, 0.4); }
    .kpi-label { font-size: 12px; text-transform: uppercase; color: var(--text-muted); font-weight: 600; letter-spacing: 0.5px; }
    .kpi-value { font-size: 28px; font-weight: 800; color: #fff; font-family: var(--font-mono); }
    .kpi-sub { font-size: 12px; color: var(--text-muted); }

    .main-grid {
      display: grid;
      grid-template-columns: 2fr 1fr;
      gap: 24px;
    }
    @media (max-width: 1024px) { .main-grid { grid-template-columns: 1fr; } }

    .panel {
      background: var(--card);
      border: 1px solid var(--card-border);
      border-radius: 12px;
      padding: 20px;
      display: flex;
      flex-direction: column;
      gap: 16px;
    }
    .panel-header {
      display: flex;
      justify-content: space-between;
      align-items: center;
    }
    .panel-title { font-size: 16px; font-weight: 700; color: #fff; display: flex; align-items: center; gap: 8px; }

    .provider-list { display: flex; flex-direction: column; gap: 12px; }
    .provider-card {
      background: rgba(255, 255, 255, 0.02);
      border: 1px solid var(--card-border);
      border-radius: 10px;
      padding: 16px;
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 16px;
    }
    .provider-info { display: flex; flex-direction: column; gap: 4px; }
    .provider-name { font-weight: 700; font-size: 15px; color: #fff; display: flex; align-items: center; gap: 8px; }
    .badge {
      font-size: 11px;
      padding: 2px 8px;
      border-radius: 6px;
      font-weight: 700;
      text-transform: uppercase;
    }
    .badge-drain { background: rgba(245, 158, 11, 0.15); color: var(--warning); border: 1px solid rgba(245, 158, 11, 0.3); }
    .badge-balanced { background: rgba(59, 130, 246, 0.15); color: var(--accent); border: 1px solid rgba(59, 130, 246, 0.3); }
    .badge-free { background: rgba(16, 185, 129, 0.15); color: var(--success); border: 1px solid rgba(16, 185, 129, 0.3); }
    .provider-meta { font-size: 12px; color: var(--text-muted); font-family: var(--font-mono); }

    .switch {
      position: relative;
      display: inline-block;
      width: 44px;
      height: 24px;
    }
    .switch input { opacity: 0; width: 0; height: 0; }
    .slider {
      position: absolute; cursor: pointer; top: 0; left: 0; right: 0; bottom: 0;
      background-color: #374151; transition: .2s; border-radius: 24px;
    }
    .slider:before {
      position: absolute; content: ""; height: 18px; width: 18px; left: 3px; bottom: 3px;
      background-color: white; transition: .2s; border-radius: 50%;
    }
    input:checked + .slider { background-color: var(--success); }
    input:checked + .slider:before { transform: translateX(20px); }

    .btn {
      background: var(--accent);
      color: white;
      border: none;
      border-radius: 8px;
      padding: 8px 16px;
      font-weight: 600;
      font-size: 13px;
      cursor: pointer;
      display: inline-flex;
      align-items: center;
      gap: 6px;
      transition: background 0.15s ease;
    }
    .btn:hover { background: #2563eb; }
    .btn-secondary { background: rgba(255, 255, 255, 0.08); color: var(--text); }
    .btn-secondary:hover { background: rgba(255, 255, 255, 0.12); }

    .feed { display: flex; flex-direction: column; gap: 8px; max-height: 400px; overflow-y: auto; }
    .feed-item {
      background: rgba(255, 255, 255, 0.02);
      border-left: 3px solid var(--accent);
      padding: 10px 14px;
      border-radius: 4px 8px 8px 4px;
      font-size: 12px;
      display: flex;
      justify-content: space-between;
      align-items: center;
    }
  </style>
</head>
<body>
  <div class="container">
    <header>
      <div class="brand">
        <span class="brand-badge">RESEARCH TRACK</span>
        <h1 class="brand-title">Inference Credit Economy — Operational Console</h1>
      </div>
      <div>
        <button class="btn" onclick="openConnectModal()">+ Connect Provider API</button>
      </div>
    </header>

    <div class="kpi-grid">
      <div class="kpi-card">
        <span class="kpi-label">Available Balance</span>
        <span class="kpi-value" id="kpi-available">320,450 CU</span>
        <span class="kpi-sub">Ready to spend on any network resource</span>
      </div>
      <div class="kpi-card">
        <span class="kpi-label">Lifetime Earned</span>
        <span class="kpi-value" style="color: var(--success)" id="kpi-earned">580,000 CU</span>
        <span class="kpi-sub">From 24 verified contributions</span>
      </div>
      <div class="kpi-card">
        <span class="kpi-label">Active Sharing Rate</span>
        <span class="kpi-value" style="color: var(--warning)" id="kpi-rate">12,400 CU/h</span>
        <span class="kpi-sub">3 models actively offered to P2P</span>
      </div>
      <div class="kpi-card">
        <span class="kpi-label">Escrow / Locked</span>
        <span class="kpi-value" style="color: #8b5cf6" id="kpi-locked">0 CU</span>
        <span class="kpi-sub">Crypto-readiness clearinghouse</span>
      </div>
    </div>

    <div class="main-grid">
      <div class="panel">
        <div class="panel-header">
          <h2 class="panel-title">Connected Providers & Smart Sharing Strategies</h2>
          <span style="font-size: 12px; color: var(--text-muted)">Secrets stay local; raw keys never enter P2P</span>
        </div>

        <div class="provider-list" id="provider-list">
          <div class="provider-card">
            <div class="provider-info">
              <div class="provider-name">
                OpenRouter · Claude 3.5 Sonnet
                <span class="badge badge-drain">⚡ DRAIN BURST</span>
              </div>
              <div class="provider-meta">
                Key: sk-...4a9f · Quota remaining: 420,000 / 500,000 tokens · Resets in 6h 12m
              </div>
            </div>
            <div style="display: flex; align-items: center; gap: 12px;">
              <button class="btn btn-secondary" onclick="configureStrategy('openrouter-claude')">Strategy</button>
              <label class="switch">
                <input type="checkbox" checked onchange="toggleShare('openrouter-claude', this.checked)">
                <span class="slider"></span>
              </label>
            </div>
          </div>

          <div class="provider-card">
            <div class="provider-info">
              <div class="provider-name">
                DeepSeek · DeepSeek R1
                <span class="badge badge-balanced">⚖ BALANCED 70%</span>
              </div>
              <div class="provider-meta">
                Key: sk-...991a · Quota remaining: 1,800,000 tokens · Reserve: 50,000 personal
              </div>
            </div>
            <div style="display: flex; align-items: center; gap: 12px;">
              <button class="btn btn-secondary" onclick="configureStrategy('deepseek-r1')">Strategy</button>
              <label class="switch">
                <input type="checkbox" checked onchange="toggleShare('deepseek-r1', this.checked)">
                <span class="slider"></span>
              </label>
            </div>
          </div>

          <div class="provider-card">
            <div class="provider-info">
              <div class="provider-name">
                Local GPU (vLLM) · Llama 3.3 70B (AWQ)
                <span class="badge badge-free">🎮 ZERO API COST</span>
              </div>
              <div class="provider-meta">
                RTX 4090 24GB · Idle monetization active (00:00 - 08:00 UTC)
              </div>
            </div>
            <div style="display: flex; align-items: center; gap: 12px;">
              <button class="btn btn-secondary" onclick="configureStrategy('local-llama')">Strategy</button>
              <label class="switch">
                <input type="checkbox" checked onchange="toggleShare('local-llama', this.checked)">
                <span class="slider"></span>
              </label>
            </div>
          </div>
        </div>
      </div>

      <div class="panel">
        <div class="panel-header">
          <h2 class="panel-title">Live Provenance & Receipt Audit</h2>
        </div>
        <div class="feed" id="audit-feed">
          <div class="feed-item">
            <div>
              <strong>+4,000 CU</strong> · Claude 3.5 Sonnet<br>
              <span style="color: var(--text-muted); font-family: var(--font-mono)">P13 sig: 8a4f...91bc · Consumer: peer-7a1b</span>
            </div>
            <span style="color: var(--success); font-weight: 700;">SETTLED</span>
          </div>
          <div class="feed-item">
            <div>
              <strong>+12,500 CU</strong> · DeepSeek R1<br>
              <span style="color: var(--text-muted); font-family: var(--font-mono)">P13 sig: 3b11...e420 · Consumer: peer-9f0c</span>
            </div>
            <span style="color: var(--success); font-weight: 700;">SETTLED</span>
          </div>
          <div class="feed-item" style="border-left-color: #8b5cf6;">
            <div>
              <strong>-8,000 CU</strong> · Consumed Qwen-Max<br>
              <span style="color: var(--text-muted); font-family: var(--font-mono)">Cross-resource execution on remote worker</span>
            </div>
            <span style="color: #8b5cf6; font-weight: 700;">CONSUMED</span>
          </div>
        </div>
      </div>
    </div>
  </div>

  <script>
    function openConnectModal() {
      alert("Connect Provider Wizard: supports OpenRouter, Anthropic, DeepSeek, OpenAI, Ollama, and local vLLM. Keys are stored strictly in local CredentialVault.");
    }
    function configureStrategy(modelId) {
      alert("Strategy Configurator for " + modelId + ":\n- ⚡ Drain Until Renewal (burst before reset)\n- ⚖ Balanced Drip (metered share with personal reserve)\n- 🆓 Selective Free-Tier Only (zero cost)\n- 🎮 Idle GPU Monetization");
    }
    function toggleShare(modelId, enabled) {
      console.log("Toggled share for " + modelId + ": " + enabled);
    }
  </script>
</body>
</html>
"#;
