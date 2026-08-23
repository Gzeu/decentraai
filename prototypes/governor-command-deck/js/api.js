/**
 * api.js — Governor Command Deck API contract adapter (M16.x prototype).
 *
 * SINGLE seam between the Command Deck UI and the future real Governor
 * backend. Every UI call goes through these functions; when the real
 * endpoints land, ONLY this file changes — the UI stays frozen.
 *
 * Current mode: MOCK. All data is fabricated to match real DecentraAI
 * concepts (Agent OS, DFCP, Sharing is Caring, Fabric Intelligence, M15
 * pressure, Obsidian memory, credit ledger). No network calls.
 *
 * Future mode: set window.DECENTRAAI_API = { baseUrl, apiKey } and flip
 * USE_MOCK = false once the real Governor API exists on the node.
 *
 * Invariant visible in every response:
 *   AI proposes -> deterministic policy decides -> workers execute.
 */

export const USE_MOCK = true;

/** Base URL for the future real API (unused while USE_MOCK = true). */
const BASE_URL = "/v1/governor";

/**
 * @typedef {"LOCAL"|"OX_ALPHA"|"LAGUNA_S_2_1_FREE"} ProviderId
 */

/**
 * @typedef {Object} WorkerStatus
 * @property {string} worker_id     e.g. "wrk_desktop_01"
 * @property {string} name          e.g. "Desktop"
 * @property {boolean} healthy
 * @property {string} avg_latency   human-readable, e.g. "1.82s"
 * @property {number} load_percent  0..100
 * @property {string[]} capabilities hub taxonomy snake_case names
 * @property {number} contribution_balance  signed credits (earned - consumed)
 */

/**
 * @typedef {Object} ExecutionEvent
 * @property {string} event_id      e.g. "ex_0f3a01"
 * @property {string} title
 * @property {string} stage         OBSERVING|THINKING|USING_SKILL|CALLING_MCP|
 *                                  DELEGATING|EXECUTING|VERIFYING|LEARNING|
 *                                  COMPLETED
 * @property {string} [skill]       skill invoked during this step
 * @property {string} [tool]        MCP tool invoked (e.g. "get_worker_status")
 * @property {string} [worker_id]   worker executing this step
 * @property {string} [result]      outcome when done
 * @property {boolean} done
 */

/**
 * @typedef {Object} MemoryNote
 * @property {"OBSERVATION"|"FACT"|"DECISION"|"LESSON"} type
 * @property {string} text
 * @property {number} confidence    0..1
 * @property {string[]} related     worker ids or concept tags
 */

/**
 * @typedef {Object} CapabilityGap
 * @property {string} id            e.g. "gap-42"
 * @property {string} title
 * @property {number} research_sources
 * @property {"open"|"research"|"proposed"|"accepted"} status
 */

// ---------------------------------------------------------------------------
// MOCK DATA — mirrors real DecentraAI state shapes
// ---------------------------------------------------------------------------

const MOCK_WORKERS = [
  {
    worker_id: "wrk_desktop_01",
    name: "Desktop",
    healthy: true,
    avg_latency: "1.82s",
    load_percent: 52,
    capabilities: ["chat", "text_generation", "reasoning", "embeddings"],
    contribution_balance: +42,
  },
  {
    worker_id: "wrk_vps_02",
    name: "VPS",
    healthy: true,
    avg_latency: "0.41s",
    load_percent: 12,
    capabilities: ["chat", "embeddings", "ocr", "stt"],
    contribution_balance: +17,
  },
  {
    worker_id: "wrk_laptop_03",
    name: "Laptop",
    healthy: true,
    avg_latency: "0.66s",
    load_percent: 8,
    capabilities: ["chat", "coding", "summarization"],
    contribution_balance: +3,
  },
];

const MOCK_PROVIDERS = [
  {
    provider_id: "LOCAL",
    label: "DecentraAI local model",
    available: true,
    cost: "free",
    latency_class: "fast",
    privacy: "on-node",
  },
  {
    provider_id: "OX_ALPHA",
    label: "Command Code / Ox Alpha",
    available: false,
    cost: "free-tier",
    latency_class: "medium",
    privacy: "external",
  },
  {
    provider_id: "LAGUNA_S_2_1_FREE",
    label: "Command Code / Laguna S 2.1 Free",
    available: true,
    cost: "free-tier",
    latency_class: "medium",
    privacy: "external",
  },
];

const MOCK_MEMORY = [
  { type: "OBSERVATION", text: "Desktop prefill tail exceeds 1.6s under mixed batch.", confidence: 0.86, related: ["wrk_desktop_01"] },
  { type: "FACT", text: "Fabric health remains 3/3. Pressure is scheduling, not capacity.", confidence: 0.93, related: ["fabric"] },
  { type: "DECISION", text: "Diagnostic tasks on Desktop require deterministic ALLOW.", confidence: 0.99, related: ["policy"] },
  { type: "LESSON", text: "Do not treat worker latency as provider failure.", confidence: 0.88, related: ["m1"] },
];

const MOCK_GAPS = [
  { id: "gap-42", title: "Embedding runtime optimization", research_sources: 4, status: "open" },
  { id: "gap-43", title: "VPS capability worker (OCR/STT)", research_sources: 2, status: "research" },
];

const MOCK_MODELS = [
  { model_id: "qwen2.5-3b-instruct-q4_k_m.gguf", node: "wrk_laptop_03", ctx: 32768, size_mb: 2007, quantization: "Q4_K_M" },
  { model_id: "llama-3.2-1b-instruct-q4_k_m.gguf", node: "wrk_desktop_01", ctx: 32768, size_mb: 771, quantization: "Q4_K_M" },
  { model_id: "nomic-embed-text-v1.5.Q4_K_M.gguf", node: "wrk_laptop_03", ctx: 2048, size_mb: 81, quantization: "Q4_K_M" },
];

// Simulated latency to make mock async feel real.
function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Generic fetch wrapper for future real mode. */
async function apiFetch(path, options) {
  const cfg = window.DECENTRAAI_API || {};
  const res = await fetch((cfg.baseUrl || BASE_URL) + path, {
    headers: { Authorization: "Bearer " + (cfg.apiKey || ""), "Content-Type": "application/json" },
    ...options,
  });
  if (!res.ok) throw new Error("Governor API error: " + res.status);
  return res.json();
}

// ---------------------------------------------------------------------------
// PUBLIC API CONTRACT
// Each function returns a Promise resolving to the documented shape.
// Mock implementations use the data above; real ones will call apiFetch.
// ---------------------------------------------------------------------------

/**
 * Full Governor state: identity, pressure, sharing, provider, workers.
 * @returns {Promise<{governor_id: string, status: string, pressure_score: number,
 *   sharing_active: boolean, provider: ProviderId, workers: WorkerStatus[],
 *   invariant: string}>}
 */
export async function getGovernorState() {
  if (USE_MOCK) {
    await delay(80);
    return {
      governor_id: "governor",
      status: "OBSERVING",
      pressure_score: 0.15,
      sharing_active: true,
      provider: "LOCAL",
      workers: MOCK_WORKERS,
      invariant: "AI proposes -> deterministic policy decides -> workers execute",
    };
  }
  return apiFetch("/state");
}

/**
 * Send a chat message to the Governor; returns the response + execution trace.
 * The trace shows: observe -> think -> skill -> MCP -> delegate -> execute
 * -> verify -> learn. Every step names its executor.
 * @param {string} message
 * @returns {Promise<{reply: string, events: ExecutionEvent[], proposal?: Object}>}
 */
export async function sendChat(message) {
  if (USE_MOCK) {
    await delay(400);
    const lower = (message || "").toLowerCase();
    if (/latenc|slow|desktop|worker/.test(lower)) {
      return {
        reply: "I detected elevated latency on Desktop. Investigating.",
        events: [
          { event_id: "ex_mock01", title: "Investigating latency", stage: "OBSERVING", done: true, result: "Desktop 1.82s tail" },
          { event_id: "ex_mock02", title: "Skill -> diagnostics", skill: "fabric-diagnostics", stage: "USING_SKILL", done: true },
          { event_id: "ex_mock03", title: "MCP -> worker status", tool: "get_worker_status", stage: "CALLING_MCP", done: true },
          { event_id: "ex_mock04", title: "Delegation -> Desktop", worker_id: "wrk_desktop_01", stage: "DELEGATING", done: true },
          { event_id: "ex_mock05", title: "Result captured", stage: "VERIFYING", done: true, result: "verified" },
        ],
        proposal: null,
      };
    }
    return {
      reply: "Noted. I will not mutate fabric without a deterministic decision.",
      events: [
        { event_id: "ex_mock10", title: "Stay inside the contract", stage: "THINKING", done: true,
          detail: "AI proposes -> deterministic Rust decides -> workers execute." },
      ],
      proposal: null,
    };
  }
  return apiFetch("/chat", { method: "POST", body: JSON.stringify({ message }) });
}

/**
 * Latest execution trace with per-step detail.
 * @returns {Promise<{events: ExecutionEvent[], active: boolean}>}
 */
export async function getExecution() {
  if (USE_MOCK) {
    await delay(60);
    return {
      events: [
        { event_id: "ex_mock20", title: "DFCP lease reserved", stage: "DELEGATING",
          worker_id: "wrk_desktop_01", done: true, result: "lease c74d83a4" },
        { event_id: "ex_mock21", title: "Worker executes assist", stage: "EXECUTING",
          worker_id: "wrk_desktop_01", done: true, result: "Llama-1B 730ms" },
        { event_id: "ex_mock22", title: "RESULT received", stage: "VERIFYING",
          done: true, result: "quota settled" },
      ],
      active: false,
    };
  }
  return apiFetch("/execution");
}

/**
 * All fabric workers with live health + contribution balance.
 * @returns {Promise<WorkerStatus[]>}
 */
export async function getWorkers() {
  if (USE_MOCK) {
    await delay(50);
    return MOCK_WORKERS;
  }
  return apiFetch("/workers");
}

/**
 * Typed Obsidian memory notes (scoped to this agent).
 * @returns {Promise<MemoryNote[]>}
 */
export async function getMemory() {
  if (USE_MOCK) {
    await delay(40);
    return MOCK_MEMORY;
  }
  return apiFetch("/memory");
}

/**
 * Available skills from the Skill Registry.
 * @returns {Promise<{skills: {id: string, name: string, version: string}[]}>}
 */
export async function getSkills() {
  if (USE_MOCK) {
    await delay(30);
    return {
      skills: [
        { id: "fabric-diagnostics", name: "Fabric diagnostics", version: "v1" },
        { id: "inference-benchmark", name: "Inference benchmark", version: "v2" },
      ],
    };
  }
  return apiFetch("/skills");
}

/**
 * Provider availability and routing info.
 * @returns {Promise<{providers: {provider_id: ProviderId, label: string,
 *   available: boolean, cost: string, latency_class: string, privacy: string}[]}>}
 */
export async function getProviders() {
  if (USE_MOCK) {
    await delay(50);
    return { providers: MOCK_PROVIDERS };
  }
  return apiFetch("/providers");
}

/**
 * Models on fabric nodes.
 * @returns {Promise<{models: {model_id: string, node: string, ctx: number, size_mb: number, quantization: string}[]}>}
 */
export async function getModels() {
  if (USE_MOCK) {
    await delay(40);
    return { models: MOCK_MODELS };
  }
  return apiFetch("/models");
}

/**
 * Capability gaps detected by the Governor (evolution backlog).
 * @returns {Promise<CapabilityGap[]>}
 */
export async function getCapabilityGaps() {
  if (USE_MOCK) {
    await delay(35);
    return MOCK_GAPS;
  }
  return apiFetch("/capability-gaps");
}

/**
 * Cancel an in-flight execution.
 * @param {string} event_id
 * @returns {Promise<{cancelled: boolean}>}
 */
export async function cancelExecution(event_id) {
  if (USE_MOCK) {
    await delay(20);
    return { cancelled: true, event_id };
  }
  return apiFetch("/execution/" + encodeURIComponent(event_id) + "/cancel", { method: "POST" });
}

console.info(
  "[api.js] mode:",
  USE_MOCK ? "MOCK (contract adapter)" : "REAL",
  "| invariant: AI proposes -> deterministic policy decides -> workers execute"
);
