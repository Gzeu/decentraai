# DecentraAI 345-Step Execution Tracker

This is the execution ledger for `docs/ROADMAP_345_DECENTRALIZED_INTELLIGENCE.md`. Every step is tracked with status, completion percentage, evidence, dependencies, next action and a 100% acceptance gate.

## Completion scale

- `0% NOT_STARTED`: no implementation or verified design.
- `25% DESIGNED`: documented and assigned, no working implementation.
- `50% PARTIAL`: related or partial code exists, but it is not integrated.
- `75% IMPLEMENTED`: implementation and local tests exist, but integration/security/reproducibility evidence is incomplete.
- `100% COMPLETE`: code, tests, integration evidence, security review, operational documentation and reproducible validation exist.

Never mark a step complete because a README, type, stub, mock, generated file or `cargo check` exists.

## Current baseline

The branch contains partial identity, discovery, trust, registry, monitoring, frontend, distributed inference and queue foundations. The real end-to-end inference path, production API, secure worker binding and two-node proof are not yet validated. Therefore all related steps remain below 100% until evidence is attached.

## Phase status

| Phase | Steps | Initial completion | Gate |
|---|---:|---:|---|
| Bootstrap/identity | 1-10 | 50% | persistent identity and restart proof |
| Discovery | 11-20 | 50% | LAN/WAN discovery, expiry and negative tests |
| Pairing/approval | 21-30 | 50% | signed pairing, scopes and revoke |
| Resource profiling | 31-46 | 25% | verified benchmark and signed capabilities |
| Model registry | 47-58 | 50% | signed manifests and readiness |
| Admission/planning | 59-86 | 25% | policy-aware explainable plans |
| Reservation | 87-96 | 25% | TTL and atomic commit/abort |
| Dispatch/execution/streaming | 97-137 | 25% | real backend and ordered stream |
| Recovery | 138-150 | 25% | crash/partition/fallback E2E |
| Privacy/supply chain/tenancy | 151-198 | 25% | isolation and artifact verification |
| Consistency/fault/security | 199-254 | 25% | stale, forged and partition tests |
| Reputation/policy/evidence | 255-301 | 25% | measured reputation and trace evidence |
| Upgrade/optimization | 302-330 | 25% | mixed-version and benchmark gates |
| Agent/governance | 331-345 | 25% | authorized, audited, reversible actions |

## Per-step task matrix

Use this format for every numbered step in the roadmap:

```text
Step: <ID>
Phase: <phase>
Definition: <what must exist>
Status: NOT_STARTED | DESIGNED | PARTIAL | IMPLEMENTED | VERIFIED | COMPLETE
Completion: 0% | 25% | 50% | 75% | 100%
Dependencies: <step IDs>
Implementation: <files and commit>
Tests: <commands and result>
Integration/E2E: <scenario and result>
Security: <negative tests/review>
Operations: <manual reproduction and rollback>
Missing: <what prevents 100%>
Next: <one concrete action>
```

The following step-by-step matrix is authoritative. Initial values are conservative and must be updated only with evidence.

### Steps 1-30: identity, discovery and pairing

1. Install node binary — 50% PARTIAL — verify release packaging and startup.
2. Validate OS/runtime prerequisites — 25% DESIGNED — add diagnostics and unsupported-runtime tests.
3. Create data directory — 50% PARTIAL — test permissions and read-only failure.
4. Generate persistent Ed25519 identity — 50% PARTIAL — test stable restart identity.
5. Derive PeerId — 50% PARTIAL — test deterministic identity binding.
6. Encrypt private key at rest — 25% DESIGNED — implement secure storage and recovery.
7. Create NodeManifest — 50% PARTIAL — version schema and required fields.
8. Hash/sign manifest — 50% PARTIAL — add tamper test.
9. Detect node role — 25% DESIGNED — add role policy and CLI/config validation.
10. Start as UNTRUSTED — 50% PARTIAL — prove protected work is rejected.
11. Discover LAN peers — 50% PARTIAL — add reproducible LAN integration test.
12. Discover DHT peers — 25% DESIGNED — bounded bootstrap and expiry.
13. Publish presence gossip — 25% DESIGNED — signed messages and rate limits.
14. Exchange PeerId/protocols — 50% PARTIAL — incompatible protocol test.
15. Exchange feature flags — 25% DESIGNED — version and downgrade behavior.
16. Verify transport identity — 50% PARTIAL — bind announcement to transport PeerId.
17. Measure latency — 25% DESIGNED — rolling probe profile.
18. Measure bandwidth — 25% DESIGNED — bounded non-disruptive probe.
19. Record network scope — 25% DESIGNED — LAN/region/public policy.
20. Create expiring PeerCandidate — 50% PARTIAL — TTL and stale cleanup tests.
21. Generate pairing token/QR — 50% PARTIAL — expiry and replay tests.
22. Receive candidate manifest — 50% PARTIAL — schema and size validation.
23. Verify manifest signature — 50% PARTIAL — forged signature test.
24. Compare observed/announced PeerId — 25% DESIGNED — hard rejection.
25. Check expiry/nonce — 25% DESIGNED — persistent replay window.
26. Require human approval — 50% PARTIAL — approve/reject UI/API.
27. Assign scopes/capabilities — 25% DESIGNED — deny-by-default policy.
28. Create/sign TrustGrant — 50% PARTIAL — version and verification.
29. Persist trust both sides — 50% PARTIAL — restart and revoke tests.
30. Transition to TRUSTED — 50% PARTIAL — no work before approval.

### Steps 31-70: profiling, model registry and admission

31. Detect OS/runtime — 50% PARTIAL — normalized profile.
32. Detect CPU model — 50% PARTIAL — permission failure test.
33. Detect CPU cores — 50% PARTIAL — scheduler capacity input.
34. Detect SIMD/accelerators — 25% DESIGNED — verified capability flags.
35. Detect GPU vendor/model — 25% DESIGNED — graceful no-GPU path.
36. Detect GPU count/VRAM — 25% DESIGNED — runtime verification.
37. Detect RAM — 50% PARTIAL — available memory included.
38. Detect storage throughput — 25% DESIGNED — bounded benchmark.
39. Detect network bandwidth — 25% DESIGNED — expiring profile.
40. Detect engines — 25% DESIGNED — adapter/version discovery.
41. Detect local models/hashes — 50% PARTIAL — registry filesystem integration.
42. Detect formats/quantization — 25% DESIGNED — engine compatibility.
43. Detect max context — 25% DESIGNED — safe bounded probe.
44. Run sandboxed benchmark — 25% DESIGNED — deterministic limits.
45. Measure TTFT/throughput/queue — 25% DESIGNED — signed profile.
46. Sign VerifiedCapabilities — 25% DESIGNED — expiry and stale tests.
47. Publish model hashes — 50% PARTIAL — remove name-only identity.
48. Validate ModelManifest — 25% DESIGNED — schema/version tests.
49. Verify model integrity — 25% DESIGNED — corruption test.
50. Verify publisher signature — 25% DESIGNED — publisher policy.
51. Check license/policy — 25% DESIGNED — distribution rejection.
52. Validate tokenizer/template — 25% DESIGNED — compatibility probe.
53. Validate quantization/engine — 25% DESIGNED — unsupported rejection.
54. Record memory/context limits — 50% PARTIAL — admission integration.
55. Associate models/workers — 50% PARTIAL — expiry-aware association.
56. Replicate registry updates — 25% DESIGNED — signed deltas.
57. Expire stale advertisements — 50% PARTIAL — clock skew test.
58. Model readiness states — 50% PARTIAL — READY probe.
59. Receive task requirements — 50% PARTIAL — typed contract.
60. Resolve immutable model — 50% PARTIAL — reject ambiguity.
61. Resolve context/output — 25% DESIGNED — server limits.
62. Resolve deadline/priority — 50% PARTIAL — propagation tests.
63. Resolve privacy/locality — 25% DESIGNED — policy engine.
64. Resolve CPU/GPU/external policy — 25% DESIGNED — filters.
65. Resolve cache affinity — 25% DESIGNED — safe cache key.
66. Resolve quota/token budget — 25% DESIGNED — accounting.
67. Filter trust/revocation — 50% PARTIAL — negative tests.
68. Filter model/readiness/capability — 50% PARTIAL — explain rejection.
69. Reject impossible requests early — 25% DESIGNED — typed errors.
70. Produce candidate set — 25% DESIGNED — deterministic planner input.

### Steps 71-150: planning, reservation, dispatch and inference

71. Detect topology — 25% DESIGNED — network/interconnect probe.
72. Estimate queue wait — 25% DESIGNED — rolling predictor.
73. Estimate prefill/decode — 25% DESIGNED — benchmark estimator.
74. Estimate throughput/cost — 25% DESIGNED — configurable model.
75. Estimate failure probability — 25% DESIGNED — reputation input.
76. Check cache affinity — 25% DESIGNED — cache-aware routing.
77. Check interconnect — 25% DESIGNED — reject unsafe TP/PP.
78. Check privacy/network — 25% DESIGNED — hard gate.
79. Generate SingleWorker plans — 50% PARTIAL — explicit traced plan.
80. Generate replica plans — 25% DESIGNED — capacity reservation.
81. Generate TensorParallel plans — 25% DESIGNED — trusted cluster only.
82. Generate PipelineParallel plans — 25% DESIGNED — topology verification.
83. Generate Speculative plans — 25% DESIGNED — model compatibility.
84. Generate disaggregated plans — 25% DESIGNED — opt-in experiment.
85. Score after hard filters — 25% DESIGNED — safety first.
86. Sign/trace ExecutionPlan — 25% DESIGNED — immutable evidence.
87. Send reservation requests — 25% DESIGNED — protocol.
88. Validate capacity/plan — 25% DESIGNED — atomic worker check.
89. Create TTL reservation — 25% DESIGNED — expiry cleanup.
90. Return accept/reject — 25% DESIGNED — typed responses.
91. Prepare multi-worker commit — 25% DESIGNED — all participants.
92. Commit atomically — 25% DESIGNED — integration test.
93. Abort partial reservations — 25% DESIGNED — fault injection.
94. Refresh with heartbeat — 25% DESIGNED — bounded renewal.
95. Release on expiry/cancel/failure — 25% DESIGNED — leak test.
96. Convert to active lease — 25% DESIGNED — lifecycle integration.
97. Open versioned stream — 50% PARTIAL — streaming protocol.
98. Authenticate transport — 50% PARTIAL — identity binding.
99. Verify sender/target — 25% DESIGNED — mismatch rejection.
100. Negotiate features — 25% DESIGNED — compatibility matrix.
101. Verify plan signature — 25% DESIGNED — tamper test.
102. Verify reservation owner — 25% DESIGNED — theft prevention.
103. Verify nonce/replay — 25% DESIGNED — replay suite.
104. Verify deadline/idempotency — 25% DESIGNED — duplicate test.
105. Verify scopes/tenant — 25% DESIGNED — authorization.
106. Send dispatch envelope — 50% PARTIAL — DTO/serialization.
107. Typed worker rejection — 50% PARTIAL — error taxonomy.
108. Atomic queue commit — 50% PARTIAL — shared queue tests.
109. Queue position estimate — 25% DESIGNED — bounded estimate.
110. Confirm readiness — 25% DESIGNED — backend/model probe.
111. Validate worker boundary — 25% DESIGNED — defense in depth.
112. Tokenize — 25% DESIGNED — adapter integration.
113. Apply limits — 25% DESIGNED — server enforcement.
114. Prefix/KV lookup — 25% DESIGNED — privacy-safe cache.
115. Prefill — 25% DESIGNED — real adapter.
116. Continuous batching — 25% DESIGNED — token budget.
117. Deadline before decode — 25% DESIGNED — timeout test.
118. Decode loop — 25% DESIGNED — real backend.
119. First-token event — 25% DESIGNED — TTFT.
120. Heartbeats — 25% DESIGNED — disconnect detection.
121. Backpressure — 25% DESIGNED — bounded stream.
122. Cancellation — 25% DESIGNED — queue and engine.
123. Worker metrics — 50% PARTIAL — monitoring integration.
124. Release compute slots — 25% DESIGNED — leak test.
125. STARTED event — 25% DESIGNED — protocol event.
126. PREFILL_STARTED event — 25% DESIGNED — lifecycle event.
127. FIRST_TOKEN event — 25% DESIGNED — correctness.
128. Ordered chunks — 25% DESIGNED — sequence contract.
129. Sequence numbers — 25% DESIGNED — duplicate/gap handling.
130. Detect duplicates/gaps — 25% DESIGNED — reorder buffer.
131. Bounded reorder buffer — 25% DESIGNED — memory/timeout limits.
132. SSE/WebSocket forwarding — 25% DESIGNED — API integration.
133. Frontend incremental state — 50% PARTIAL — live API connection.
134. UI worker/model status — 50% PARTIAL — replace simulated data.
135. Accept cancellation — 25% DESIGNED — API contract.
136. CANCEL_ACK — 25% DESIGNED — race-safe behavior.
137. Exactly one terminal event — 25% DESIGNED — invariant test.
138. Release batch capacity — 25% DESIGNED — terminal handler.
139. Release KV reservation — 25% DESIGNED — cache lifecycle.
140. Release capacity lease — 25% DESIGNED — coordinator.
141. Close stream — 50% PARTIAL — graceful close.
142. Persist final state — 25% DESIGNED — durable store.
143. Finalize audit — 50% PARTIAL — lifecycle events.
144. Aggregate metrics/trace — 50% PARTIAL — correlation IDs.
145. Update reputation — 25% DESIGNED — outcome pipeline.
146. Update circuit breaker — 25% DESIGNED — health policy.
147. Update performance profiles — 25% DESIGNED — benchmark feedback.
148. Detect uncertain completion — 25% DESIGNED — status protocol.
149. Safe retry — 25% DESIGNED — idempotency/deadline gate.
150. Expose typed final result — 50% PARTIAL — API/frontend error model.

### Steps 151-198: privacy, supply chain and tenancy

151. Assign PrivacyPolicy — 25% DESIGNED — typed request policy.
152. Implement locality modes — 25% DESIGNED — policy tests.
153. Classify input — 25% DESIGNED — defaults and override.
154. Enforce locality — 25% DESIGNED — hard routing gate.
155. Strip metadata — 25% DESIGNED — privacy test.
156. Encrypt payload end-to-end — 50% PARTIAL — transport/application review.
157. Separate metadata/content — 25% DESIGNED — protocol split.
158. Minimize worker data — 25% DESIGNED — payload review.
159. Apply retention TTL — 25% DESIGNED — cleanup job.
160. Isolate tenant KV — 0% NOT_STARTED — namespace implementation.
161. Prevent cross-tenant cache — 0% NOT_STARTED — negative test.
162. Hash audit data — 25% DESIGNED — redaction.
163. User deletion — 0% NOT_STARTED — API and cleanup.
164. Redact logs/errors — 25% DESIGNED — secret/prompt tests.
165. Privacy negative E2E — 0% NOT_STARTED — matrix.
166. Signed ModelManifest — 25% DESIGNED — schema/signature.
167. Verify source/publisher — 25% DESIGNED — trust policy.
168. Verify artifact hashes — 25% DESIGNED — corruption test.
169. Verify license — 0% NOT_STARTED — metadata policy.
170. Validate format/tokenizer/template — 25% DESIGNED — probe.
171. Validate quantization/engine — 25% DESIGNED — matrix.
172. Scan artifacts — 0% NOT_STARTED — scanner.
173. Content-addressed download — 0% NOT_STARTED — chunk protocol.
174. Verify download chunks — 0% NOT_STARTED — retry/corruption tests.
175. Assemble/hash artifact — 0% NOT_STARTED — artifact store.
176. Sandbox smoke test — 0% NOT_STARTED — isolated runner.
177. Quality/format test — 0% NOT_STARTED — bounded evaluation.
178. Approval before distribution — 25% DESIGNED — verifier workflow.
179. Keep rollback version — 0% NOT_STARTED — lifecycle.
180. Mark incompatible workers — 25% DESIGNED — registry status.
181. Model deprecation — 0% NOT_STARTED — policy.
182. Emergency revocation — 0% NOT_STARTED — signed revoke.
183. Version registry updates — 25% DESIGNED — signed deltas.
184. Mixed-version tests — 0% NOT_STARTED — matrix.
185. Namespaces — 0% NOT_STARTED — tenant model.
186. Tenant/user assignment — 0% NOT_STARTED — identity mapping.
187. Per-tenant scopes — 25% DESIGNED — policy engine.
188. Per-tenant quotas — 0% NOT_STARTED — quota store.
189. Queue isolation — 0% NOT_STARTED — scheduler.
190. Cache isolation — 0% NOT_STARTED — namespaced keys.
191. Metrics isolation — 0% NOT_STARTED — authorization filters.
192. Cross-namespace rejection — 0% NOT_STARTED — negative tests.
193. Fair scheduling — 0% NOT_STARTED — weighted queues.
194. Active sequence limits — 25% DESIGNED — worker admission.
195. Tokens/minute limit — 25% DESIGNED — rate policy.
196. VRAM monopolization prevention — 0% NOT_STARTED — reservation quotas.
197. Worker allowlists — 25% DESIGNED — tenant policy.
198. Boundary audit — 25% DESIGNED — policy events.

### Steps 199-254: consistency, faults and security

199. Local mesh view — 25% DESIGNED — state model.
200. Timestamp/expiry — 50% PARTIAL — apply to announcements.
201. Gossip presence — 25% DESIGNED — signed events.
202. Async registry replication — 25% DESIGNED — delta protocol.
203. Conflict resolution — 25% DESIGNED — deterministic rules.
204. Reservation owner/TTL — 25% DESIGNED — lease schema.
205. Authorized cleanup — 25% DESIGNED — permission tests.
206. Multi-worker participants — 25% DESIGNED — coordinator.
207. Prepare/commit/abort — 25% DESIGNED — fault injection.
208. Idempotent control — 25% DESIGNED — duplicate tests.
209. Stale message rejection — 25% DESIGNED — version/clock policy.
210. Duplicate nonce rejection — 25% DESIGNED — replay store.
211. Reconciliation queue — 0% NOT_STARTED — reconnect workflow.
212. Stale registry policy — 0% NOT_STARTED — degraded mode.
213. Delta publication — 0% NOT_STARTED — registry protocol.
214. Divergence detection — 0% NOT_STARTED — consistency checks.
215. Re-verification — 25% DESIGNED — stale node workflow.
216. Heartbeat monitoring — 50% PARTIAL — worker health integration.
217. Suspected disconnect — 25% DESIGNED — state transition.
218. Grace period — 25% DESIGNED — configurable timer.
219. Last-state query — 25% DESIGNED — status protocol.
220. Uncertain request — 25% DESIGNED — API semantics.
221. Expired reservation release — 25% DESIGNED — cleanup worker.
222. Circuit breaker — 25% DESIGNED — scheduler.
223. Safe fallback — 50% PARTIAL — retry policy tests.
224. Preserve request ID — 50% PARTIAL — lifecycle contract.
225. Prevent duplicate completion — 25% DESIGNED — completion store.
226. Preserve partial output — 25% DESIGNED — result model.
227. Unsafe retry notification — 25% DESIGNED — frontend state.
228. Durable queue recovery — 0% NOT_STARTED — persistence.
229. UNKNOWN running requests — 25% DESIGNED — restart semantics.
230. Identity re-probe — 25% DESIGNED — startup health.
231. Backend/model re-probe — 25% DESIGNED — readiness.
232. Registry restore — 25% DESIGNED — durable state.
233. Network partition tests — 0% NOT_STARTED — fault harness.
234. Crash/restart/disk tests — 0% NOT_STARTED — recovery suite.
235. Peer rate limit — 25% DESIGNED — limiter.
236. Identity/tenant rate limit — 25% DESIGNED — policy.
237. Pairing attempt limit — 25% DESIGNED — abuse test.
238. Announcement spam detection — 0% NOT_STARTED — security signal.
239. Manifest limits — 25% DESIGNED — parser guard.
240. Pre-deserialization validation — 50% PARTIAL — audit protocol paths.
241. Nesting/decompression limits — 25% DESIGNED — fuzz tests.
242. No remote code execution — 25% DESIGNED — sandbox policy.
243. No arbitrary model loading — 25% DESIGNED — allowlist.
244. Least privilege engine — 25% DESIGNED — deployment hardening.
245. Backend sandbox — 0% NOT_STARTED — isolation.
246. Filesystem restriction — 25% DESIGNED — container/native policy.
247. GPU restriction — 25% DESIGNED — device policy.
248. Control/data stream separation — 25% DESIGNED — protocol split.
249. Downgrade protection — 25% DESIGNED — negotiation tests.
250. Pairing token rotation — 25% DESIGNED — credential lifecycle.
251. Credential revocation — 25% DESIGNED — propagation.
252. Security event isolation — 25% DESIGNED — logging policy.
253. Dependency/secret scanning — 50% PARTIAL — CI review.
254. Malicious worker tests — 0% NOT_STARTED — adversarial suite.

### Steps 255-301: reputation, policy and evidence

255. Uptime score — 25% DESIGNED — profile schema.
256. Success rate — 50% PARTIAL — metrics input.
257. Timeout rate — 50% PARTIAL — metrics input.
258. p95 latency — 50% PARTIAL — monitoring input.
259. Protocol compliance — 25% DESIGNED — scorer.
260. Output validity — 25% DESIGNED — verifier.
261. Malformed output detection — 25% DESIGNED — negative tests.
262. Response schema validation — 25% DESIGNED — protocol layer.
263. Verifier nodes — 0% NOT_STARTED — critical task policy.
264. Duplicate execution policy — 0% NOT_STARTED — verifier flow.
265. Output comparison — 0% NOT_STARTED — verifier engine.
266. Suspicious worker state — 25% DESIGNED — quarantine trigger.
267. Routing weight reduction — 25% DESIGNED — scheduler.
268. Re-benchmarking — 25% DESIGNED — scheduled probe.
269. Quarantine — 50% PARTIAL — trust integration.
270. Re-approval — 25% DESIGNED — UI/API.
271. Local free mode — 50% PARTIAL — settlement independent.
272. Internal credits — 25% DESIGNED — optional ledger.
273. Worker cost estimate — 25% DESIGNED — capability profile.
274. User maximum cost — 25% DESIGNED — admission policy.
275. Cost/latency comparison — 25% DESIGNED — planner score.
276. Budget reservation — 25% DESIGNED — lease integration.
277. Token/GPU accounting — 25% DESIGNED — usage events.
278. Actual cost — 25% DESIGNED — terminal accounting.
279. Quota enforcement — 25% DESIGNED — policy engine.
280. Settlement outside hot path — 25% DESIGNED — architecture rule.
281. Dispute isolation — 25% DESIGNED — separate service.
282. Availability windows — 25% DESIGNED — worker policy.
283. User preferences — 25% DESIGNED — planner input.
284. Policy/billing audit — 25% DESIGNED — evidence events.
285. Request trace — 50% PARTIAL — correlation across crates.
286. API/planning/reservation/P2P spans — 25% DESIGNED — tracing convention.
287. State transition metrics — 25% DESIGNED — lifecycle instrumentation.
288. Admission latency — 25% DESIGNED — metric.
289. Dispatch latency — 25% DESIGNED — metric.
290. Queue wait — 50% PARTIAL — queue metrics.
291. P2P throughput — 25% DESIGNED — transport metrics.
292. Tokenization/prefill/decode — 25% DESIGNED — adapter metrics.
293. Cache hit rate — 25% DESIGNED — cache metrics.
294. Worker health — 50% PARTIAL — monitoring foundation.
295. Memory pressure — 50% PARTIAL — probe foundation.
296. GPU thermal throttling — 0% NOT_STARTED — hardware probe.
297. Network congestion — 25% DESIGNED — transport monitor.
298. Selected plan UI — 25% DESIGNED — frontend integration.
299. Rejection explanation UI — 25% DESIGNED — typed errors.
300. Safe trace export — 25% DESIGNED — redacted API.
301. Incident report — 25% DESIGNED — report format.

### Steps 302-345: upgrades, optimization and governance

302. Protocol negotiation — 25% DESIGNED — version handshake.
303. Feature advertisement — 25% DESIGNED — capabilities schema.
304. API versioning — 25% DESIGNED — route policy.
305. Wire compatibility — 25% DESIGNED — fixtures/tests.
306. Manifest schema version — 25% DESIGNED — migration rules.
307. Staged node upgrades — 0% NOT_STARTED — rollout controller.
308. Worker subset upgrade — 0% NOT_STARTED — canary policy.
309. Post-upgrade health probe — 25% DESIGNED — deployment check.
310. Metric comparison — 25% DESIGNED — release gate.
311. Automatic rollback — 0% NOT_STARTED — reversible deployment.
312. Vulnerable version revocation — 25% DESIGNED — signed list.
313. Registry migration — 0% NOT_STARTED — migration framework.
314. Compatibility matrix — 25% DESIGNED — tested matrix.
315. Mixed-version mesh — 0% NOT_STARTED — E2E matrix.
316. Compare estimates/results — 25% DESIGNED — plan outcome.
317. Correct latency estimates — 25% DESIGNED — rolling estimator.
318. Correct throughput estimates — 25% DESIGNED — rolling estimator.
319. Correct failure probability — 25% DESIGNED — reputation input.
320. Learn cache affinity — 25% DESIGNED — cache profile.
321. Learn batch efficiency — 25% DESIGNED — benchmark profile.
322. Detect engine regressions — 25% DESIGNED — comparison gate.
323. Disable regressing speculation — 25% DESIGNED — policy controller.
324. Recalculate worker weights — 25% DESIGNED — scheduler update.
325. Recalculate plan scores — 25% DESIGNED — configurable weights.
326. Propose reconfiguration — 25% DESIGNED — explainable proposal.
327. Approve high-risk automation — 25% DESIGNED — human gate.
328. Explain decisions — 25% DESIGNED — decision record.
329. Persist decision inputs/outputs — 25% DESIGNED — evidence store.
330. Replay decisions — 25% DESIGNED — deterministic tool.
331. Expose safe MCP capabilities — 25% DESIGNED — tool schema/scopes.
332. Agent discovers workers — 25% DESIGNED — read-only tool.
333. Agent inspects capabilities — 25% DESIGNED — redacted response.
334. Agent requests benchmarks — 25% DESIGNED — approval-aware tool.
335. Agent proposes approval — 25% DESIGNED — proposal only.
336. Policy approval for privileged admission — 25% DESIGNED — human/policy gate.
337. Agent explains plan — 25% DESIGNED — decision trace.
338. Agent estimates cost/latency — 25% DESIGNED — planner query.
339. Agent submits inference — 25% DESIGNED — scoped action.
340. Agent requests cancellation — 25% DESIGNED — scoped action.
341. Agent inspects redacted traces — 25% DESIGNED — privacy filter.
342. Prevent unauthorized policy changes — 25% DESIGNED — authorization test.
343. Human approval for destructive actions — 25% DESIGNED — confirmation gate.
344. Audit every agent action — 25% DESIGNED — immutable tool event.
345. Governance for upgrades/revocation/shutdown — 25% DESIGNED — policy engine and emergency runbook.

## Update rule

After every PR, update affected lines with:

```text
status, completion, commit, implementation files, tests, E2E evidence,
security review, operations procedure, missing work and next action.
```

A phase percentage is the weighted average of its steps. A release gate is open only when every required step has 100% evidence.
