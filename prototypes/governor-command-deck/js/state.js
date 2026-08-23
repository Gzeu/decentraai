export const PATH = ["OBSERVING", "DIAGNOSING", "DELEGATING", "VERIFYING"];
export const MODELS = { "ox-alpha": "Ox Alpha", laguna: "Laguna", local: "Local" };
export const COMMANDS = [
  ["Run capability", "capability"],
  ["Inspect worker", "insp:workers"],
  ["Inspect memory", "insp:memory"],
  ["Run benchmark", "benchmark"],
  ["Research capability", "research"],
  ["Replay execution", "replay"],
  ["Pause execution", "pause"],
  ["Open incident", "incident"],
  ["Switch provider", "insp:providers"],
  ["Switch model", "insp:providers"]
];

export const state = {
  governor: "IDLE",
  phase: "idle",
  surface: "chat",
  provider: "hybrid",
  model: "ox-alpha",
  inspector: null,
  palette: false,
  paletteQ: "",
  paletteI: 0,
  utterance: "Quiet. Watching the fabric.",
  paused: false,
  running: false,
  step: -1,
  selectedEvent: null,
  workers: [
    { id: "wrk_desktop_01", name: "Desktop", ok: true, lat: "1.82s" },
    { id: "wrk_hub_02", name: "Hub", ok: true, lat: "0.41s" },
    { id: "wrk_studio_03", name: "Studio", ok: true, lat: "0.66s" }
  ],
  messages: [],
  events: [],
  memories: [
    { type: "OBSERVATION", text: "Desktop prefill tail exceeds 1.6s under mixed batch.", conf: 0.86, rel: ["wrk_desktop_01"] },
    { type: "FACT", text: "Fabric health remains 3/3. Pressure is scheduling, not capacity.", conf: 0.93, rel: ["fabric"] },
    { type: "DECISION", text: "Diagnostic tasks on Desktop require deterministic ALLOW.", conf: 0.99, rel: ["policy"] },
    { type: "LESSON", text: "Do not treat worker latency as provider failure.", conf: 0.88, rel: ["m1"] }
  ],
  gaps: [{ id: "gap-42", title: "Embedding runtime optimization", research: 4, status: "open" }],
  experiments: [{ id: "exp-7", title: "v2 vs v1", result: null, status: "idle" }]
};

export function now() {
  return new Date().toLocaleTimeString([], { hour12: false, hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export function phaseOf(s) {
  if (["INCIDENT", "WARNING"].includes(s)) return "hold";
  if (["IDLE", "WAITING"].includes(s)) return "idle";
  if (s === "OBSERVING") return "observing";
  if (["THINKING", "RESEARCHING", "USING_SKILL", "CALLING_MCP"].includes(s)) return "diagnosing";
  if (["DELEGATING", "EXECUTING"].includes(s)) return "delegating";
  if (["VERIFYING", "COMPLETED"].includes(s)) return "verifying";
  if (s === "LEARNING") return "learning";
  return "idle";
}

export function pathIndex(phase) {
  return { observing: 0, diagnosing: 1, delegating: 2, verifying: 3, learning: 3, hold: -1, idle: -1 }[phase];
}

export function setGov(s, utter) {
  state.governor = s;
  state.phase = phaseOf(s);
  if (utter) state.utterance = utter;
  const app = document.querySelector("#app");
  app.dataset.phase = state.phase;
  app.dataset.live = state.events.some((e) => e.live) || ["DELEGATING", "EXECUTING"].includes(s) ? "true" : "false";
}

export function addMsg(role, text, caps = []) {
  state.messages.push({ role, text, caps, t: now() });
}

export function addEv(partial) {
  const ev = {
    id: "ex_0f3a" + String(state.events.length + 1).padStart(2, "0"),
    open: false, live: true, done: false, elapsed: "0.0s",
    model: MODELS[state.model], ...partial
  };
  state.events.push(ev);
  state.selectedEvent = ev.id;
  return ev;
}

export function lastEv(patch) {
  Object.assign(state.events[state.events.length - 1] || {}, patch);
}
