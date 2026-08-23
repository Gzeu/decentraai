import { addEv, addMsg, COMMANDS, lastEv, setGov, state } from "./state.js";
import { getGovernorState, getWorkers } from "./api.js";
import { CommandPalette, EvolutionPanel, GovernorShell, MemoryInspector, ProviderStatus, WorkerStatus } from "./components.js";

const app = document.querySelector("#app");
const drawer = document.querySelector("#drawer");
const palette = document.querySelector("#palette");

function inspectorHtml() {
  if (state.inspector === "memory") return MemoryInspector();
  if (state.inspector === "evolution") return EvolutionPanel();
  if (state.inspector === "workers") return WorkerStatus();
  if (state.inspector === "providers") return ProviderStatus();
  return "";
}

function render() {
  const focusId = document.activeElement && document.activeElement.id;
  const caret = focusId === "composer" || focusId === "palInput" ? document.activeElement.selectionStart : null;
  app.innerHTML = GovernorShell();
  if (!state.inspector) { drawer.hidden = true; drawer.innerHTML = ""; }
  else {
    drawer.hidden = false;
    drawer.innerHTML = `<aside class="panel"><button class="close" data-close type="button">Close</button>${inspectorHtml()}</aside>`;
  }
  if (!state.palette) { palette.hidden = true; palette.innerHTML = ""; }
  else {
    palette.hidden = false;
    palette.innerHTML = CommandPalette();
    const inp = document.querySelector("#palInput");
    if (inp) { inp.focus(); inp.selectionStart = inp.value.length; }
  }
  const stage = document.querySelector("#stage");
  if (stage && state.surface === "chat") stage.scrollTop = stage.scrollHeight;
  if (focusId === "composer") {
    const el = document.querySelector("#composer");
    if (el) { el.focus(); if (caret != null) el.selectionStart = el.selectionEnd = caret; }
  }
}

function runCapability() {
  state.surface = "chat";
  setGov("USING_SKILL", "Invoking fabric-diagnostics. I propose; Rust decides.");
  addMsg("gov", "Running a bounded capability.", [{ kind: "SKILL", title: "fabric-diagnostics", meta: "skill", detail: "Read-only probes.", open: true }]);
  addEv({ title: "Skill → diagnostics", skill: "fabric-diagnostics", st: "USING_SKILL" });
  setTimeout(() => { lastEv({ live: false, done: true, elapsed: "0.8s", result: "admitted" }); setGov("IDLE", state.utterance); render(); }, 900);
}

function runBenchmark() {
  state.surface = "execution";
  setGov("EXECUTING", "Benchmarking embedding runtime v2 against v1.");
  addMsg("gov", "Opening the v2 vs v1 benchmark. Proposal only after verification.");
  addEv({ title: "Benchmark", skill: "inference-benchmark", st: "EXECUTING" });
  setTimeout(() => {
    lastEv({ live: false, done: true, elapsed: "1.4s", result: "+18% throughput" });
    state.experiments[0].result = "+18% throughput";
    setGov("VERIFYING", "v2 is faster on the stalled path.");
    render();
  }, 1200);
}

function simulateIncident() {
  state.workers[0].ok = false;
  setGov("INCIDENT", "Desktop dropped heartbeats. Holding new delegations.");
  addMsg("gov", "Desktop dropped heartbeats. Holding new delegations until fabric reports healthy.", [
    { kind: "WARNING", title: "wrk_desktop_01 unreachable", meta: "incident", detail: "Restrained hold.", open: false }
  ]);
}

function togglePause() {
  if (!state.running) return;
  state.paused = !state.paused;
  if (state.paused) setGov("WAITING", "Waiting. Execution held.");
}

function runCmd(cmd) {
  state.palette = false;
  if (cmd === "replay") startScenario();
  else if (cmd === "pause") togglePause();
  else if (cmd === "incident") simulateIncident();
  else if (cmd === "capability") runCapability();
  else if (cmd === "benchmark") runBenchmark();
  else if (cmd === "research") {
    state.inspector = "evolution";
    setGov("RESEARCHING", "Researching embedding runtime optimization.");
    addMsg("gov", "Capability gap #42 is the live evolutionary thread.");
  } else if (cmd.startsWith("insp:")) state.inspector = cmd.slice(5);
  else if (cmd.startsWith("prov:")) state.provider = cmd.slice(5);
  else if (cmd.startsWith("model:")) state.model = cmd.slice(6);
  render();
}

const SCENARIO = [
  { d: 600, run() { setGov("OBSERVING", "I detected elevated latency on Desktop."); addMsg("gov", "I detected elevated latency on Desktop."); addEv({ title: "Investigating latency", op: "observe fabric telemetry", st: "OBSERVING" }); } },
  { d: 1100, run() { lastEv({ live: false, done: true, elapsed: "0.8s" }); setGov("THINKING", "Tracing whether the stall is admission, prefill, or provider selection."); addMsg("gov", "Tracing whether the stall is admission, prefill, or provider selection.", [{ kind: "THINKING", title: "Compare Desktop tail vs Hub/Studio", meta: "0.9s", detail: "Hypothesis: local prefill on wrk_desktop_01.", open: true }]); addEv({ title: "Thinking through stall locus", st: "THINKING" }); } },
  { d: 1200, run() { lastEv({ live: false, done: true, elapsed: "1.1s" }); setGov("USING_SKILL", "Diagnosing with fabric-diagnostics."); addMsg("gov", "Invoking a bounded diagnostic skill. I propose; Rust will decide whether it may run.", [{ kind: "SKILL", title: "fabric-diagnostics", meta: "skill", detail: "Read-only probes.", open: true }]); addEv({ title: "Skill → diagnostics", skill: "fabric-diagnostics", st: "USING_SKILL" }); } },
  { d: 1000, run() { lastEv({ live: false, done: true, elapsed: "0.7s", result: "skill admitted" }); setGov("CALLING_MCP", "Asking MCP for live worker status."); addMsg("gov", "Calling MCP for live worker status.", [{ kind: "MCP", title: "get_worker_status", meta: "mcp", detail: "Target wrk_desktop_01 · mock transport only.", open: true }]); addEv({ title: "MCP → worker status", tool: "get_worker_status", st: "CALLING_MCP" }); } },
  { d: 1100, run() { lastEv({ live: false, done: true, elapsed: "0.6s", result: "Desktop 1.82s tail" }); setGov("DELEGATING", "Delegating a diagnostic task to Desktop."); addMsg("gov", "Desktop is alive. Delegating after deterministic ALLOW.", [{ kind: "DELEGATION", title: "Desktop → diagnostic task", meta: "wrk_desktop_01", detail: "Policy: ALLOW · workers execute; the Governor does not.", open: true }]); addEv({ title: "Delegation → Desktop", worker: "wrk_desktop_01", st: "DELEGATING" }); } },
  { d: 1400, run() { lastEv({ live: false, done: true, elapsed: "0.4s" }); setGov("EXECUTING", "Desktop is executing the diagnostic."); addEv({ title: "Desktop executes diagnostic", worker: "wrk_desktop_01", skill: "fabric-diagnostics", st: "EXECUTING" }); } },
  { d: 1300, run() { lastEv({ live: false, done: true, elapsed: "1.8s", result: "Latency confirmed: 1.8s" }); addMsg("gov", "Latency confirmed on Desktop prefill.", [{ kind: "RESULT", title: "Latency confirmed: 1.8s", meta: "verify pending", detail: "p95 1.82s", open: true }]); addEv({ title: "Result captured", result: "1.8s prefill tail" }); } },
  { d: 900, run() { lastEv({ live: false, done: true, elapsed: "0.3s" }); setGov("VERIFYING", "Verifying the measurement against fabric telemetry."); addMsg("gov", "Verifying the measurement against fabric telemetry.", [{ kind: "VERIFICATION", title: "Independent corroboration", meta: "ok", detail: "Two observers agree.", open: false }]); addEv({ title: "VERIFYING", st: "VERIFYING", verify: "corroborated" }); } },
  { d: 1100, run() { lastEv({ live: false, done: true, elapsed: "0.8s", result: "verified" }); setGov("RESEARCHING", "This is a capability gap, not an incident."); addMsg("gov", "This is a capability gap, not an incident.", [{ kind: "RESEARCH", title: "Embedding runtime optimization", meta: "gap #42", detail: "4 sources.", open: true }]); state.gaps[0].status = "research"; addEv({ title: "Research capability gap #42", st: "RESEARCHING" }); } },
  { d: 1200, run() { lastEv({ live: false, done: true, elapsed: "1.0s" }); addMsg("gov", "Opening a bounded experiment.", [{ kind: "PROPOSAL", title: "Experiment v2 vs v1", meta: "exp-7", detail: "Compare v2 against v1 on Desktop only.", open: true }]); addEv({ title: "Benchmark", skill: "inference-benchmark", st: "EXECUTING" }); setGov("EXECUTING", "Benchmarking v2 against v1."); } },
  { d: 1300, run() { lastEv({ live: false, done: true, elapsed: "1.4s", result: "+18% throughput" }); state.experiments[0].result = "+18% throughput"; setGov("VERIFYING", "v2 is faster on the stalled path."); addMsg("gov", "v2 is faster on the stalled path.", [{ kind: "RESULT", title: "+18% throughput", meta: "v2", detail: "Ready to propose, not to deploy.", open: true }]); addEv({ title: "Benchmark complete", result: "+18% throughput" }); } },
  { d: 1000, run() { setGov("LEARNING", "I propose promoting skill v2. Rust decides."); addMsg("gov", "Proposal only. Deterministic Rust decides promotion.", [{ kind: "PROPOSAL", title: "Promote skill v2", meta: "awaiting Rust", detail: "I do not ship skills.", open: true }, { kind: "MEMORY", title: "Lesson written", meta: "0.88", detail: "Worker latency is not a provider failure.", open: false }]); state.gaps[0].status = "proposed"; addEv({ title: "Proposal → promote skill v2", st: "LEARNING", verify: "policy ALLOW (sim)" }); } },
  { d: 900, run() { lastEv({ live: false, done: true, elapsed: "0.5s", result: "ALLOW recorded" }); setGov("COMPLETED", "Policy ALLOW recorded. Skill v2 is proposed, not merged."); state.running = false; addMsg("gov", "Policy ALLOW recorded in simulation. Skill v2 is proposed, not merged."); } }
];

function startScenario() {
  state.running = true; state.paused = false; state.step = -1; state.events = [];
  state.messages = state.messages.filter((m) => m.keep);
  state.workers[0].ok = true; state.gaps[0].status = "open"; state.experiments[0].result = null;
  addMsg("user", "Desktop inference feels slow. Check the fabric.");
  setGov("IDLE", "Quiet. Watching the fabric.");
  render(); advance();
}

function advance() {
  if (!state.running || state.paused) return;
  state.step += 1;
  const step = SCENARIO[state.step];
  if (!step) {
    state.running = false;
    setTimeout(() => { if (state.governor === "COMPLETED") setGov("IDLE", "Quiet. Watching the fabric."); render(); }, 1800);
    return;
  }
  step.run(); render();
  setTimeout(advance, step.d);
}

function sendText(raw) {
  const input = document.querySelector("#composer");
  const t = (raw ?? (input ? input.value : "")).trim();
  if (!t) return;
  if (!raw && input) input.value = "";
  addMsg("user", t);
  const l = t.toLowerCase();
  if (/latenc|slow|desktop|worker/.test(l)) startScenario();
  else if (/memory|remember/.test(l)) { setGov("THINKING", "Opening typed memory."); addMsg("gov", "These are typed residues, not a graph."); state.inspector = "memory"; }
  else if (/evolv|skill|gap/.test(l)) { state.inspector = "evolution"; setGov("RESEARCHING", "Capability gap #42 is live."); addMsg("gov", "Capability gap #42 is the live evolutionary thread."); }
  else if (/incident|down/.test(l)) simulateIncident();
  else {
    setGov("THINKING", "I can observe, propose, and delegate.");
    addMsg("gov", "Noted. I will not mutate fabric without a deterministic decision.", [{ kind: "THINKING", title: "Stay inside the contract", detail: "AI proposes → deterministic Rust decides → workers execute.", open: true }]);
  }
  render();
}

document.addEventListener("click", (e) => {
  const t = e.target.closest("[data-surface],[data-cap],[data-ev],[data-cmd],[data-close],#cmdBtn,#send,#modelBtn");
  if (!t) return;
  if (t.id === "cmdBtn") { state.palette = true; state.paletteQ = ""; state.paletteI = 0; render(); return; }
  if (t.id === "modelBtn") { state.inspector = "providers"; render(); return; }
  if (t.id === "send") { sendText(); return; }
  if (t.dataset.surface) { state.surface = t.dataset.surface; render(); return; }
  if (t.dataset.cap) {
    const [mi, i] = t.dataset.cap.split(":").map(Number);
    state.messages[mi].caps[i].open = !state.messages[mi].caps[i].open; render(); return;
  }
  if (t.dataset.ev) {
    const ev = state.events.find((x) => x.id === t.dataset.ev);
    ev.open = !ev.open; state.selectedEvent = ev.id; render(); return;
  }
  if (t.dataset.cmd) { runCmd(t.dataset.cmd); return; }
  if (t.hasAttribute("data-close")) { state.inspector = null; render(); }
});

drawer.addEventListener("click", (e) => { if (e.target === drawer) { state.inspector = null; render(); } });
palette.addEventListener("click", (e) => { if (e.target === palette) { state.palette = false; render(); } });
palette.addEventListener("input", (e) => { if (e.target.id === "palInput") { state.paletteQ = e.target.value; render(); } });

document.addEventListener("keydown", (e) => {
  const typing = e.target.matches("input");
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") { e.preventDefault(); state.palette = !state.palette; state.paletteQ = ""; render(); return; }
  if (e.key === "Escape") { state.palette = false; state.inspector = null; render(); return; }
  if (e.target.id === "composer" && e.key === "Enter") { sendText(); return; }
  if (state.palette) {
    const items = COMMANDS.filter((c) => c[0].toLowerCase().includes(state.paletteQ.toLowerCase()));
    if (e.key === "ArrowDown") { e.preventDefault(); state.paletteI += 1; render(); }
    if (e.key === "ArrowUp") { e.preventDefault(); state.paletteI -= 1; render(); }
    if (e.key === "Enter") {
      e.preventDefault();
      const q = state.paletteQ.trim();
      if (items[state.paletteI] && (q === "" || items[state.paletteI][0].toLowerCase().startsWith(q.toLowerCase()))) {
        runCmd(items[state.paletteI][1]); return;
      }
      if (q) { state.palette = false; sendText(q); }
    }
    if (e.target.id === "palInput") return;
  }
  if (typing) return;
  if (e.key === "1") { state.surface = "chat"; render(); }
  if (e.key === "2") { state.surface = "execution"; render(); }
  if (e.key === " ") { e.preventDefault(); togglePause(); render(); }
  if (e.key.toLowerCase() === "r") startScenario();
  if (e.key.toLowerCase() === "m") { state.inspector = "memory"; render(); }
  if (e.key.toLowerCase() === "e") { state.inspector = "evolution"; render(); }
});

setInterval(() => {
  if (!state.running || state.paused) return;
  const ev = state.events.find((x) => x.live);
  if (ev) { ev.elapsed = (parseFloat(ev.elapsed) + 0.1).toFixed(1) + "s"; if (state.surface === "execution") render(); }
}, 100);

addMsg("gov", "Governor online. I observe, propose, and wait for deterministic Rust before anyone executes.");
state.messages[0].keep = true;
render();

// Fetch real fabric state from Governor daemon and update UI.
async function loadRealState() {
  try {
    const gs = await getGovernorState();
    if (gs.workers && gs.workers.length) {
      state.workers = gs.workers.map(w => ({
        id: w.worker_id,
        name: w.name,
        ok: w.healthy,
        lat: w.avg_latency || "-"
      }));
    }
    if (gs.pressure_score > 0) {
      state.utterance = "Pressure " + gs.pressure_score + " — watching.";
      setGov("OBSERVING", state.utterance);
    }
    render();
  } catch (e) {
    console.warn("governor state fetch failed:", e);
  }
}
loadRealState();
setInterval(loadRealState, 30000);

setTimeout(startScenario, 800);
