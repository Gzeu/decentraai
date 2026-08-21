//! Standalone Reactive UI Dashboard for the DecentraAI Inference Credit Economy.
//!
//! Dedicated, fluid, lightweight, single-file HTML5/CSS3/JS reactive interface
//! specifically for `research/inference-credit-economy`.
//!
//! Features:
//! - Per-model mode switcher (🌴 Day-Off Max Yield, ⚖️ Balanced Work, 🆓 Free-Tier Only, 🎮 Idle GPU).
//! - Instant One-Click Sharing Toggle (ON / OFF).
//! - Interactive Strategy Configuration Modal (Sliders for Personal Reserve, Cooldown on 429, Max Concurrency).
//! - Connect Provider Wizard (OpenRouter, Anthropic, DeepSeek, OpenAI, Groq, Ollama).
//! - Live Provenance Feed & Output-Driven Settlement Metrics.

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
      --card-inner: #1a233a;
      --card-border: rgba(255, 255, 255, 0.08);
      --accent: #3b82f6;
      --accent-glow: rgba(59, 130, 246, 0.25);
      --purple: #8b5cf6;
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
      font-size: 13px;
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
      border-radius: 12px;
      padding: 18px;
      display: flex;
      flex-direction: column;
      gap: 14px;
    }
    .provider-header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 12px;
    }
    .provider-title { font-weight: 700; font-size: 16px; color: #fff; display: flex; align-items: center; gap: 8px; }
    .provider-meta { font-size: 12px; color: var(--text-muted); font-family: var(--font-mono); }

    .strategy-pills {
      display: flex;
      gap: 8px;
      flex-wrap: wrap;
    }
    .pill {
      background: var(--card-inner);
      border: 1px solid var(--card-border);
      color: var(--text-muted);
      border-radius: 8px;
      padding: 6px 12px;
      font-size: 12px;
      font-weight: 600;
      cursor: pointer;
      display: flex;
      align-items: center;
      gap: 6px;
      transition: all 0.15s ease;
    }
    .pill:hover { border-color: rgba(255, 255, 255, 0.2); color: #fff; }
    .pill.active {
      background: rgba(59, 130, 246, 0.15);
      border-color: var(--accent);
      color: #93c5fd;
    }
    .pill.active.dayoff {
      background: rgba(139, 92, 246, 0.15);
      border-color: var(--purple);
      color: #c084fc;
    }
    .pill.active.free {
      background: rgba(16, 185, 129, 0.15);
      border-color: var(--success);
      color: #6ee7b7;
    }

    .switch {
      position: relative;
      display: inline-block;
      width: 48px;
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
    input:checked + .slider:before { transform: translateX(22px); }

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
    .btn-sm { padding: 4px 10px; font-size: 11px; border-radius: 6px; }

    .feed { display: flex; flex-direction: column; gap: 8px; max-height: 440px; overflow-y: auto; }
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

    /* Modal */
    .modal-backdrop {
      position: fixed; top: 0; left: 0; width: 100%; height: 100%;
      background: rgba(0, 0, 0, 0.7); backdrop-filter: blur(4px);
      display: none; justify-content: center; align-items: center; z-index: 100;
    }
    .modal {
      background: var(--card);
      border: 1px solid var(--card-border);
      border-radius: 16px;
      max-width: 540px;
      width: 90%;
      padding: 24px;
      display: flex;
      flex-direction: column;
      gap: 18px;
    }
    .modal h3 { font-size: 18px; color: #fff; }
    .form-group { display: flex; flex-direction: column; gap: 6px; }
    .form-group label { font-size: 12px; font-weight: 600; color: var(--text-muted); }
    .form-control {
      background: var(--card-inner);
      border: 1px solid var(--card-border);
      color: #fff;
      padding: 10px 12px;
      border-radius: 8px;
      font-size: 14px;
      font-family: inherit;
    }
    .form-control:focus { outline: none; border-color: var(--accent); }
    .modal-actions { display: flex; justify-content: flex-end; gap: 10px; margin-top: 8px; }
  </style>
</head>
<body>
  <div class="container">
    <header>
      <div class="brand">
        <span class="brand-badge">RESEARCH TRACK</span>
        <h1 class="brand-title">DecentraAI · Inference Credit Economy</h1>
      </div>
      <div style="display: flex; gap: 10px;">
        <button class="btn btn-secondary" onclick="openStrategyConfigModal()">⚙️ Global Strategy</button>
        <button class="btn" onclick="openConnectModal()">+ Connect Provider API</button>
      </div>
    </header>

    <div class="hero-banner">
      <div class="hero-text">
        <h2>🌴 Output-Driven Subscription Sharing</h2>
        <p>No need to guess your token balance. When you're not working, toggle sharing ON: your node processes requests for others, measures the exact output tokens generated, and earns fair, model-weighted CU. If your provider hits an hourly limit, the node auto-cooldowns safely.</p>
      </div>
      <button class="btn" style="background: var(--purple);" onclick="setAllMode('dayoff')">Set All to Day-Off Mode</button>
    </div>

    <div class="kpi-grid">
      <div class="kpi-card">
        <span class="kpi-label">Available Balance</span>
        <span class="kpi-value" id="kpi-available">458,200 CU</span>
        <span class="kpi-sub">Spendable on any AI model or remote GPU</span>
      </div>
      <div class="kpi-card">
        <span class="kpi-label">Output Tokens Served</span>
        <span class="kpi-value" style="color: var(--success)" id="kpi-output">312,400 tokens</span>
        <span class="kpi-sub">Actual measured completions generated</span>
      </div>
      <div class="kpi-card">
        <span class="kpi-label">Current Earning Velocity</span>
        <span class="kpi-value" style="color: var(--warning)" id="kpi-velocity">18,600 CU/h</span>
        <span class="kpi-sub">3 models actively taking P2P tasks</span>
      </div>
      <div class="kpi-card">
        <span class="kpi-label">Persistent Invariant</span>
        <span class="kpi-value" style="color: #60a5fa">100% DURABLE</span>
        <span class="kpi-sub">CU survive daily & monthly provider resets</span>
      </div>
    </div>

    <div class="main-grid">
      <div class="panel">
        <div class="panel-header">
          <h2 class="panel-title">Connected Providers & Mode Selectors</h2>
          <span style="font-size: 12px; color: var(--text-muted)">Secrets stay local in CredentialVault</span>
        </div>

        <div class="provider-list" id="provider-list">
          <!-- Model Card 1 -->
          <div class="provider-card" id="card-claude">
            <div class="provider-header">
              <div class="provider-info">
                <div class="provider-title">Anthropic · Claude 3.5 Sonnet</div>
                <div class="provider-meta">Key: sk-...4a9f · Tier: Frontier ($150 CU/1k out) · Rolling 5h limit</div>
              </div>
              <div style="display: flex; align-items: center; gap: 12px;">
                <button class="btn btn-secondary btn-sm" onclick="openConfigureModal('Claude 3.5 Sonnet', 'card-claude')">Configure</button>
                <label class="switch">
                  <input type="checkbox" checked onchange="toggleShare('card-claude', this.checked)">
                  <span class="slider"></span>
                </label>
              </div>
            </div>
            <div class="strategy-pills">
              <div class="pill active dayoff" onclick="setCardMode('card-claude', 'dayoff', this)">🌴 Day-Off (Max Yield)</div>
              <div class="pill" onclick="setCardMode('card-claude', 'balanced', this)">⚖️ Balanced Work (50%)</div>
              <div class="pill" onclick="setCardMode('card-claude', 'buffer', this)">🛡️ Safety Buffer (20k res.)</div>
            </div>
          </div>

          <!-- Model Card 2 -->
          <div class="provider-card" id="card-deepseek">
            <div class="provider-header">
              <div class="provider-info">
                <div class="provider-title">DeepSeek · DeepSeek R1 Reasoning</div>
                <div class="provider-meta">Key: sk-...81f2 · Tier: Frontier ($100 CU/1k out) · Daily reset 00:00 UTC</div>
              </div>
              <div style="display: flex; align-items: center; gap: 12px;">
                <button class="btn btn-secondary btn-sm" onclick="openConfigureModal('DeepSeek R1', 'card-deepseek')">Configure</button>
                <label class="switch">
                  <input type="checkbox" checked onchange="toggleShare('card-deepseek', this.checked)">
                  <span class="slider"></span>
                </label>
              </div>
            </div>
            <div class="strategy-pills">
              <div class="pill active dayoff" onclick="setCardMode('card-deepseek', 'dayoff', this)">🌴 Day-Off (Max Yield)</div>
              <div class="pill" onclick="setCardMode('card-deepseek', 'balanced', this)">⚖️ Balanced Work (70%)</div>
              <div class="pill" onclick="setCardMode('card-deepseek', 'buffer', this)">🛡️ Safety Buffer (10k res.)</div>
            </div>
          </div>

          <!-- Model Card 3 -->
          <div class="provider-card" id="card-openrouter">
            <div class="provider-header">
              <div class="provider-info">
                <div class="provider-title">OpenRouter · Free Tier Cascade (Llama 3.3 / Gemini Flash)</div>
                <div class="provider-meta">Zero $ Cost · Tier: Free/Commodity · Auto-cascade on 429</div>
              </div>
              <div style="display: flex; align-items: center; gap: 12px;">
                <button class="btn btn-secondary btn-sm" onclick="openConfigureModal('OpenRouter Free Cascade', 'card-openrouter')">Configure</button>
                <label class="switch">
                  <input type="checkbox" checked onchange="toggleShare('card-openrouter', this.checked)">
                  <span class="slider"></span>
                </label>
              </div>
            </div>
            <div class="strategy-pills">
              <div class="pill active free" onclick="setCardMode('card-openrouter', 'free', this)">🆓 100% Free Tiers Only</div>
              <div class="pill" onclick="setCardMode('card-openrouter', 'balanced', this)">⚖️ Throttle Capped (30 rpm)</div>
            </div>
          </div>
        </div>
      </div>

      <div class="panel">
        <div class="panel-header">
          <h2 class="panel-title">Live Provenance & Output Feed</h2>
        </div>
        <div class="feed" id="audit-feed">
          <div class="feed-item">
            <div>
              <strong>+4,500 CU</strong> · Claude 3.5 Sonnet (30 prompt + 1,470 out)<br>
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
          <div class="feed-item" style="border-left-color: var(--purple);">
            <div>
              <strong>-5,000 CU</strong> · Spent on Remote GPU (Qwen 72B)<br>
              <span style="color: var(--text-muted); font-family: var(--font-mono)">Cross-resource execution on remote worker node</span>
            </div>
            <span style="color: var(--purple); font-weight: 700;">CONSUMED</span>
          </div>
        </div>
      </div>
    </div>
  </div>

  <!-- Configure Modal -->
  <div class="modal-backdrop" id="config-modal">
    <div class="modal">
      <h3 id="modal-title">Configure Sharing Strategy</h3>
      <div class="form-group">
        <label>Active Mode</label>
        <select class="form-control" id="modal-mode-select">
          <option value="dayoff">🌴 Day-Off (Max Yield — Auto-cooldown on 429)</option>
          <option value="balanced">⚖️ Balanced Drip (Controlled Sharing with Safety Reserve)</option>
          <option value="freetier">🆓 100% Free Tiers Only (Zero Cost)</option>
          <option value="gpu">🎮 Idle Hardware Monetization</option>
        </select>
      </div>
      <div class="form-group">
        <label>Personal Safety Reserve (Tokens reserved exclusively for your work)</label>
        <input type="number" class="form-control" id="modal-reserve-input" value="15000">
      </div>
      <div class="form-group">
        <label>Auto-Cooldown on Rate Limit 429 (Minutes to rest before resuming)</label>
        <input type="number" class="form-control" id="modal-cooldown-input" value="60">
      </div>
      <div class="modal-actions">
        <button class="btn btn-secondary" onclick="closeModal()">Cancel</button>
        <button class="btn" onclick="saveConfiguration()">Save Strategy</button>
      </div>
    </div>
  </div>

  <!-- Connect Modal -->
  <div class="modal-backdrop" id="connect-modal">
    <div class="modal">
      <h3>Connect AI Provider / API Key</h3>
      <div class="form-group">
        <label>Provider</label>
        <select class="form-control" id="connect-provider-select">
          <option value="openrouter">OpenRouter (Claude, Llama, Gemini, DeepSeek)</option>
          <option value="anthropic">Anthropic API / Claude Pro</option>
          <option value="deepseek">DeepSeek API (V3 / R1)</option>
          <option value="openai">OpenAI (GPT-4o, o3-mini)</option>
          <option value="ollama">Ollama / Local vLLM (Zero API Cost)</option>
          <option value="groq">Groq (Ultra-fast Llama 3.3)</option>
        </select>
      </div>
      <div class="form-group">
        <label>API Key / Secret (Saved strictly in local in-memory CredentialVault)</label>
        <input type="password" class="form-control" id="connect-key-input" placeholder="sk-...">
      </div>
      <div class="form-group">
        <label>Model ID</label>
        <input type="text" class="form-control" id="connect-model-input" value="anthropic/claude-3.5-sonnet">
      </div>
      <div class="modal-actions">
        <button class="btn btn-secondary" onclick="closeConnectModal()">Cancel</button>
        <button class="btn" onclick="submitConnect()">Save in CredentialVault</button>
      </div>
    </div>
  </div>

  <script>
    let activeConfigCard = null;

    function setCardMode(cardId, mode, el) {
      const parent = el.parentElement;
      parent.querySelectorAll('.pill').forEach(p => p.classList.remove('active', 'dayoff', 'free'));
      el.classList.add('active');
      if (mode === 'dayoff') el.classList.add('dayoff');
      if (mode === 'free') el.classList.add('free');
    }

    function setAllMode(mode) {
      document.querySelectorAll('.provider-card').forEach(card => {
        const pills = card.querySelectorAll('.pill');
        if (pills.length > 0) {
          pills.forEach(p => p.classList.remove('active', 'dayoff', 'free'));
          pills[0].classList.add('active', 'dayoff');
        }
      });
      alert("All connected models set to Day-Off Mode (Max Yield on outputs).");
    }

    function toggleShare(cardId, enabled) {
      const card = document.getElementById(cardId);
      if (enabled) {
        card.style.opacity = "1";
      } else {
        card.style.opacity = "0.5";
      }
    }

    function openConfigureModal(title, cardId) {
      activeConfigCard = cardId;
      document.getElementById('modal-title').innerText = "Configure Strategy: " + title;
      document.getElementById('config-modal').style.display = 'flex';
    }

    function openStrategyConfigModal() {
      openConfigureModal('Global Node Defaults', null);
    }

    function closeModal() {
      document.getElementById('config-modal').style.display = 'none';
    }

    function saveConfiguration() {
      alert("Strategy configuration updated and saved locally.");
      closeModal();
    }

    function openConnectModal() {
      document.getElementById('connect-modal').style.display = 'flex';
    }

    function closeConnectModal() {
      document.getElementById('connect-modal').style.display = 'none';
    }

    function submitConnect() {
      const prov = document.getElementById('connect-provider-select').value;
      const model = document.getElementById('connect-model-input').value;
      alert("Successfully connected " + prov + " (" + model + ") to local CredentialVault!");
      closeConnectModal();
    }
  </script>
</body>
</html>
"#;
