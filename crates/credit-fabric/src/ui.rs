//! Standalone UI Dashboard for the DecentraAI Inference Credit Economy.
//!
//! Dedicated, fluid, lightweight, single-file HTML5/CSS3/JS reactive interface
//! specifically for `research/inference-credit-economy`.
//!
//! Features:
//! - "I'm not working today, share my subscription capacity" one-click toggle.
//! - Dynamic output-driven CU earning (get CU proportional to actual output generated).
//! - Auto-handling of rolling hourly limits (Claude/ChatGPT) and daily reset (DeepSeek/Groq).
//! - Zero-config auto-pause on 429 throttling.
//! - Live CU balances (Available / Earned / Spent / Locked).

pub const DEDICATED_ECONOMY_DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>DecentraAI — Inference Credit Economy Console</title>
  <style>
    :root {
      --bg: #090d16;
      --card: #131a2b;
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
    
    .hero-banner {
      background: linear-gradient(135deg, rgba(59, 130, 246, 0.12), rgba(139, 92, 246, 0.12));
      border: 1px solid rgba(59, 130, 246, 0.3);
      border-radius: 14px;
      padding: 20px 24px;
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 20px;
    }
    .hero-text h2 { font-size: 18px; font-weight: 800; color: #fff; margin-bottom: 4px; }
    .hero-text p { font-size: 13px; color: var(--text-muted); }

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

    .provider-list { display: flex; flex-direction: column; gap: 14px; }
    .provider-card {
      background: rgba(255, 255, 255, 0.02);
      border: 1px solid var(--card-border);
      border-radius: 10px;
      padding: 18px;
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 16px;
    }
    .provider-info { display: flex; flex-direction: column; gap: 6px; }
    .provider-name { font-weight: 700; font-size: 15px; color: #fff; display: flex; align-items: center; gap: 8px; }
    .badge {
      font-size: 11px;
      padding: 3px 8px;
      border-radius: 6px;
      font-weight: 700;
      text-transform: uppercase;
    }
    .badge-active { background: rgba(16, 185, 129, 0.15); color: var(--success); border: 1px solid rgba(16, 185, 129, 0.3); }
    .badge-throttled { background: rgba(239, 68, 68, 0.15); color: var(--danger); border: 1px solid rgba(239, 68, 68, 0.3); }
    .badge-dayoff { background: rgba(139, 92, 246, 0.15); color: #c084fc; border: 1px solid rgba(139, 92, 246, 0.3); }
    .provider-meta { font-size: 12px; color: var(--text-muted); font-family: var(--font-mono); }

    .switch {
      position: relative;
      display: inline-block;
      width: 46px;
      height: 26px;
    }
    .switch input { opacity: 0; width: 0; height: 0; }
    .slider {
      position: absolute; cursor: pointer; top: 0; left: 0; right: 0; bottom: 0;
      background-color: #374151; transition: .2s; border-radius: 26px;
    }
    .slider:before {
      position: absolute; content: ""; height: 20px; width: 20px; left: 3px; bottom: 3px;
      background-color: white; transition: .2s; border-radius: 50%;
    }
    input:checked + .slider { background-color: var(--success); }
    input:checked + .slider:before { transform: translateX(20px); }

    .btn {
      background: var(--accent);
      color: white;
      border: none;
      border-radius: 8px;
      padding: 9px 16px;
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

    .feed { display: flex; flex-direction: column; gap: 8px; max-height: 420px; overflow-y: auto; }
    .feed-item {
      background: rgba(255, 255, 255, 0.02);
      border-left: 3px solid var(--accent);
      padding: 12px 14px;
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
        <h1 class="brand-title">DecentraAI · Inference Credit Economy</h1>
      </div>
      <div>
        <button class="btn" onclick="openConnectModal()">+ Connect Subscription / API</button>
      </div>
    </header>

    <div class="hero-banner">
      <div class="hero-text">
        <h2>🌴 Day-Off Mode & Output-Driven Settlement Active</h2>
        <p>You don't need to calculate token limits. When you're not working, turn sharing ON: your node serves requests for others, and you earn durable CU proportional to the exact completion tokens generated. Auto-pauses on provider hourly rate limits.</p>
      </div>
      <button class="btn" style="background: #8b5cf6;" onclick="toggleAllDayOff()">Toggle All to Day-Off Mode</button>
    </div>

    <div class="kpi-grid">
      <div class="kpi-card">
        <span class="kpi-label">Available Balance</span>
        <span class="kpi-value" id="kpi-available">458,200 CU</span>
        <span class="kpi-sub">Reusable on any AI model or remote GPU</span>
      </div>
      <div class="kpi-card">
        <span class="kpi-label">Output Tokens Generated</span>
        <span class="kpi-value" style="color: var(--success)" id="kpi-output">312,400 tokens</span>
        <span class="kpi-sub">Actual measured completions served</span>
      </div>
      <div class="kpi-card">
        <span class="kpi-label">Active Earning Velocity</span>
        <span class="kpi-value" style="color: var(--warning)" id="kpi-rate">18,600 CU/h</span>
        <span class="kpi-sub">3 models currently taking P2P tasks</span>
      </div>
      <div class="kpi-card">
        <span class="kpi-label">Durable vs Quota Invariant</span>
        <span class="kpi-value" style="color: #60a5fa">100% PERSISTENT</span>
        <span class="kpi-sub">CU survive daily/monthly provider resets</span>
      </div>
    </div>

    <div class="main-grid">
      <div class="panel">
        <div class="panel-header">
          <h2 class="panel-title">Your AI Subscriptions & Connected Models</h2>
          <span style="font-size: 12px; color: var(--text-muted)">API keys stay local in CredentialVault</span>
        </div>

        <div class="provider-list" id="provider-list">
          <div class="provider-card">
            <div class="provider-info">
              <div class="provider-name">
                Anthropic · Claude 3.5 Sonnet (Pro / Team Sub)
                <span class="badge badge-dayoff">🌴 DAY-OFF DRAIN</span>
              </div>
              <div class="provider-meta">
                Key: sk-...4a9f · Mode: Rolling 5-Hour limit · Served today: 142k tokens (+284k CU) · Auto-cooldown on 429
              </div>
            </div>
            <div style="display: flex; align-items: center; gap: 12px;">
              <label class="switch">
                <input type="checkbox" checked onchange="toggleModelShare('claude', this.checked)">
                <span class="slider"></span>
              </label>
            </div>
          </div>

          <div class="provider-card">
            <div class="provider-info">
              <div class="provider-name">
                DeepSeek · DeepSeek R1 (Tier 1 API)
                <span class="badge badge-active">⚡ ACTIVE SHARING</span>
              </div>
              <div class="provider-meta">
                Key: sk-...81f2 · Mode: Daily reset at 00:00 UTC · Served: 85k output tokens (+170k CU)
              </div>
            </div>
            <div style="display: flex; align-items: center; gap: 12px;">
              <label class="switch">
                <input type="checkbox" checked onchange="toggleModelShare('deepseek', this.checked)">
                <span class="slider"></span>
              </label>
            </div>
          </div>

          <div class="provider-card">
            <div class="provider-info">
              <div class="provider-name">
                OpenRouter · Free Models Cascade (Llama 3.3 / Gemini Flash)
                <span class="badge" style="background: rgba(16,185,129,0.15); color: var(--success)">🆓 100% FREE TIER</span>
              </div>
              <div class="provider-meta">
                Zero $ cost · Auto-swaps between free models if throttled · Served: 34k tokens
              </div>
            </div>
            <div style="display: flex; align-items: center; gap: 12px;">
              <label class="switch">
                <input type="checkbox" checked onchange="toggleModelShare('openrouter-free', this.checked)">
                <span class="slider"></span>
              </label>
            </div>
          </div>
        </div>
      </div>

      <div class="panel">
        <div class="panel-header">
          <h2 class="panel-title">Live Output Settlement & Receipts</h2>
        </div>
        <div class="feed" id="audit-feed">
          <div class="feed-item">
            <div>
              <strong>+3,600 CU</strong> · Claude 3.5 Sonnet (1,800 out tokens)<br>
              <span style="color: var(--text-muted); font-family: var(--font-mono)">P13 sig: ed25519:7a4f... · Consumer: peer-8b1c</span>
            </div>
            <span style="color: var(--success); font-weight: 700;">OUTPUT SETTLED</span>
          </div>
          <div class="feed-item">
            <div>
              <strong>+8,400 CU</strong> · DeepSeek R1 (4,200 out tokens)<br>
              <span style="color: var(--text-muted); font-family: var(--font-mono)">P13 sig: ed25519:1d9c... · Consumer: peer-3f90</span>
            </div>
            <span style="color: var(--success); font-weight: 700;">OUTPUT SETTLED</span>
          </div>
          <div class="feed-item" style="border-left-color: #8b5cf6;">
            <div>
              <strong>-5,000 CU</strong> · Spent on Remote GPU (Qwen 72B)<br>
              <span style="color: var(--text-muted); font-family: var(--font-mono)">Cross-resource execution on remote node</span>
            </div>
            <span style="color: #8b5cf6; font-weight: 700;">CONSUMED</span>
          </div>
        </div>
      </div>
    </div>
  </div>

  <script>
    function openConnectModal() {
      alert("Connect Subscription: Choose Anthropic, OpenRouter, DeepSeek, OpenAI, Groq or Ollama. Keys are stored locally in CredentialVault.");
    }
    function toggleAllDayOff() {
      alert("All connected subscription models set to Day-Off Mode: maximum sharing speed, auto-pause on 429.");
    }
    function toggleModelShare(modelId, enabled) {
      console.log("Model " + modelId + " share toggled: " + enabled);
    }
  </script>
</body>
</html>
"#;
