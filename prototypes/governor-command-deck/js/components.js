import { COMMANDS, MODELS, PATH, pathIndex, state } from "./state.js";

export function GovernorStatus() {
  return `<div class="online"><i class="pip"></i><span>Governor online</span></div>`;
}

export function GovernorPresence() {
  const idx = pathIndex(state.phase);
  const label = state.provider === "hybrid"
    ? `${MODELS[state.model]} / Local`
    : `${MODELS[state.model]} / ${state.provider === "command-code" ? "Command Code" : "Local"}`;
  return `
    <header class="bar">
      ${GovernorStatus()}
      <div class="bar-r">
        <button class="model mono" id="modelBtn" type="button">${label}</button>
        <button class="icon-btn mono" id="cmdBtn" type="button">⌘K</button>
      </div>
    </header>
    <section class="presence">
      <div class="who">Governor</div>
      <p class="utter" id="utter">${state.utterance}</p>
      <ol class="path">${PATH.map((n, i) => `<li class="${i === idx ? "on" : i < idx ? "done" : ""}">${n}</li>`).join("")}</ol>
      <div class="flow">↓ Live execution</div>
    </section>`;
}

export function ActivityCapsule(c, i, mi) {
  const policy = c.kind === "PROPOSAL"
    ? `<div class="policy">AI proposes → simulated Rust decision → execution</div>` : "";
  return `<div class="cap ${c.open ? "open" : ""}" data-kind="${c.kind}" data-cap="${mi}:${i}">
    <i class="sig"></i>
    <div>
      <div class="cap-h"><span class="tag">${c.kind}</span><span class="cap-t">${c.title}</span><span class="cap-m mono">${c.meta || ""}</span></div>
      <div class="cap-x">${c.detail || ""}${policy}</div>
    </div></div>`;
}

export function ChatMessage(m, mi) {
  return `<article class="msg ${m.role}">
    <div class="meta-row"><span>${m.role === "user" ? "Operator" : "Governor"}</span><span class="mono">${m.t}</span></div>
    <div class="body">${m.text}</div>
    ${m.caps?.length ? `<div class="caps">${m.caps.map((c, i) => ActivityCapsule(c, i, mi)).join("")}</div>` : ""}
  </article>`;
}

export function ChatSurface() {
  return `<div class="transcript">${state.messages.map(ChatMessage).join("")}</div>`;
}

export function ToolCall(ev) { return ev.tool || "—"; }
export function SkillActivity(ev) { return ev.skill || "—"; }
export function DelegationActivity(ev) { return ev.worker || "—"; }

export function ExecutionEvent(ev) {
  return `<div class="ev ${ev.open ? "open" : ""} ${ev.live ? "live" : ""} ${ev.done ? "done" : ""}" data-ev="${ev.id}">
    <div class="ev-h"><span>${ev.title}</span><span class="cap-m mono">${ev.elapsed}</span></div>
    <div class="ev-x"><div class="kv">
      <b>Operation</b><span>${ev.op || ev.title}</span>
      <b>State</b><span class="mono">${ev.st || state.governor}</span>
      <b>Elapsed</b><span class="mono">${ev.elapsed}</span>
      <b>Model</b><span class="mono">${ev.model}</span>
      <b>Skill</b><span class="mono">${SkillActivity(ev)}</span>
      <b>Tool</b><span class="mono">${ToolCall(ev)}</span>
      <b>Worker</b><span class="mono">${DelegationActivity(ev)}</span>
      <b>Result</b><span>${ev.result || "pending"}</span>
      <b>Verify</b><span>${ev.verify || "—"}</span>
    </div></div></div>`;
}

export function ExecutionTimeline() {
  if (!state.events.length) {
    return `<div class="timeline"><p class="body" style="color:var(--dim)">No live execution. Ask the Governor, or replay from the palette.</p></div>`;
  }
  return `<div class="timeline"><div class="meta-row"><span>Governor</span><span class="mono">${state.events[0].id}</span></div>
    <div class="spine">${state.events.map(ExecutionEvent).join("")}</div></div>`;
}

export function WorkerStatus() {
  return `<h2 class="h">Workers</h2>${state.workers.map((w) => `<div class="cap open" data-kind="WORKER" style="margin-bottom:12px">
    <i class="sig"></i><div><div class="cap-h"><span>${w.name}</span><span class="cap-m mono">${w.id}</span></div>
    <div class="cap-x">Latency ${w.lat} · ${w.ok ? "healthy" : "incident"}</div></div></div>`).join("")}`;
}

export function ProviderStatus() {
  return `<h2 class="h">Providers / models</h2>
    <button class="cmd" data-cmd="prov:local">Local</button>
    <button class="cmd" data-cmd="prov:command-code">Command Code</button>
    <button class="cmd" data-cmd="prov:hybrid">Local + Command Code</button>
    <button class="cmd" data-cmd="model:ox-alpha">Ox Alpha</button>
    <button class="cmd" data-cmd="model:laguna">Laguna</button>
    <button class="cmd" data-cmd="model:local">Local</button>`;
}

export function CapabilityGap(g) {
  return `<p class="tag" style="color:var(--research)">Capability gap #42</p><p>${g.title}</p>`;
}

export function Experiment(e) {
  return `<div class="kv" style="margin:16px 0">
    <b>Experiment</b><span>${e.title}</span>
    <b>Result</b><span>${e.result || "pending"}</span></div>`;
}

export function EvolutionPanel() {
  const g = state.gaps[0], e = state.experiments[0];
  return `<h2 class="h">Evolution</h2>${CapabilityGap(g)}${Experiment(e)}
    <div class="kv"><b>Research</b><span>${g.research} sources</span><b>Status</b><span class="mono">${g.status}</span></div>
    <p style="color:var(--dim)">Skill evolution, not a ticket. A proposal waits for deterministic ALLOW.</p>`;
}

export function MemoryInspector() {
  return `<h2 class="h">Memory</h2>${state.memories.map((m) => `<div class="cap open" data-kind="MEMORY" style="margin-bottom:14px">
    <i class="sig"></i><div><div class="cap-h"><span class="tag">${m.type}</span><span class="cap-m mono">${Math.round(m.conf * 100)}%</span></div>
    <div class="cap-x">${m.text}<ul class="rel">${m.rel.map((r) => `<li class="mono">${r}</li>`).join("")}</ul></div></div></div>`).join("")}`;
}

export function CommandPalette() {
  const q = state.paletteQ.toLowerCase();
  const items = COMMANDS.filter((c) => c[0].toLowerCase().includes(q));
  state.paletteI = Math.max(0, Math.min(state.paletteI, Math.max(items.length - 1, 0)));
  return `<div class="box"><input id="palInput" placeholder="Ask Governor..." value="${state.paletteQ.replace(/"/g, "")}" />
    <div class="rule"></div>
    ${items.map((c, i) => `<button class="cmd ${i === state.paletteI ? "on" : ""}" data-cmd="${c[1]}" type="button"><span>${c[0]}</span></button>`).join("")}</div>`;
}

export function GovernorShell() {
  return `${GovernorPresence()}
    <nav class="nav">
      <button type="button" data-surface="chat" class="${state.surface === "chat" ? "on" : ""}">Chat</button>
      <button type="button" data-surface="execution" class="${state.surface === "execution" ? "on" : ""}">Execute</button>
    </nav>
    <main class="stage" id="stage">${state.surface === "chat" ? ChatSurface() : ExecutionTimeline()}</main>
    <div class="composer" id="composerWrap" style="display:${state.surface === "chat" ? "flex" : "none"}">
      <input id="composer" placeholder="Speak to the Governor…" autocomplete="off" />
      <button class="icon-btn" id="send" type="button">Send</button>
    </div>
    <div class="hint mono">1 chat · 2 execute · ⌘K ask · space pause · r replay</div>`;
}
