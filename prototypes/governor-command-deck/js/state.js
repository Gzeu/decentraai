export const PATH = ["OBSERVING", "DIAGNOSING", "DELEGATING", "VERIFYING"];
export const COMMANDS = [
  ["Run capability", "capability"],
  ["Inspect worker", "insp:workers"],
  ["Inspect memory", "insp:memory"],
  ["Run benchmark", "benchmark"],
  ["Research capability", "research"],
  ["Replay execution", "replay"],
  ["Pause execution", "pause"],
  ["Open incident", "incident"]
];

export const state = {
  governor: "IDLE",
  phase: "idle",
  surface: "chat",
  inspector: null,
  palette: false,
  paletteQ: "",
  paletteI: 0,
  utterance: "Pressure 0.2 — watching.",
  pressure: 0.2,
  paused: false,
  running: false,
  step: -1,
  selectedEvent: null,
  workers: [
    { id: "wrk_desktop_01", name: "Desktop", ok: true, lat: "1.82s", note: "local GPU · prefill tail" },
    { id: "wrk_vps_02", name: "VPS", ok: true, lat: "0.64s", note: "remote capacity · GPU 98% in last sample" },
    { id: "wrk_laptop_03", name: "Laptop", ok: true, lat: "0.91s", note: "admitted · energy-sensitive" }
  ],
  messages: [],
  events: [],
  memories: [
    { type: "OBSERVATION", text: "Desktop prefill tail exceeds 1.6s under mixed batch.", conf: 0.86, rel: ["wrk_desktop_01"] },
    { type: "FACT", text: "Fabric is 3/3: Desktop, VPS, Laptop. Pressure is 0.2 at idle.", conf: 0.94, rel: ["fabric"] },
    { type: "DECISION", text: "Diagnostic tasks require deterministic ALLOW before a worker executes.", conf: 0.99, rel: ["policy"] },
    { type: "LESSON", text: "Worker latency is not a provider failure.", conf: 0.88, rel: ["wrk_desktop_01"] }
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
  if (s === "EXECUTING" || s === "DELEGATING") state.pressure = 0.61;
  else if (s === "INCIDENT") state.pressure = 0.84;
  else if (s === "IDLE") state.pressure = 0.2;
  else state.pressure = 0.34;
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
    model: "governor", ...partial
  };
  state.events.push(ev);
  state.selectedEvent = ev.id;
  return ev;
}

export function lastEv(patch) {
  Object.assign(state.events[state.events.length - 1] || {}, patch);
}
