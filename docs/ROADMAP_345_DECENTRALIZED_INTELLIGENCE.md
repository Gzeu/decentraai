# DecentraAI Roadmap: 345 Steps

## Purpose

This is the complete product and engineering roadmap for turning DecentraAI into a decentralized intelligence fabric: nodes discover one another, establish trust, verify resources and models, plan execution, process inference, stream results, recover from failures, learn from performance and remain governable.

The roadmap is intentionally numbered so an implementation agent can reference exact steps. Steps are grouped by lifecycle phase; phases may run concurrently after their dependencies are satisfied.

## System invariant

No request may be executed unless identity, policy, model, capacity, deadline and execution plan are valid. No worker may become trusted solely because it is reachable on the network. No optimization may merge without a benchmark against the M10 baseline.

## A. Bootstrap and identity: 1-10

1. Install the node binary.
2. Validate OS and runtime prerequisites.
3. Create the node data directory.
4. Generate a persistent Ed25519 identity.
5. Derive the libp2p PeerId from the public key.
6. Encrypt private key material at rest.
7. Create the initial NodeManifest.
8. Hash and sign the manifest.
9. Detect node role: client, coordinator, worker, relay or observer.
10. Start the node in `UNTRUSTED` state.

## B. Discovery: 11-20

11. Discover local peers through LAN mechanisms.
12. Discover extended peers through libp2p DHT.
13. Publish presence through the mesh gossip protocol.
14. Exchange PeerId and supported protocols.
15. Exchange protocol versions and feature flags.
16. Verify the observed transport identity.
17. Measure peer latency.
18. Measure available bandwidth.
19. Record network scope: loopback, LAN, region or public mesh.
20. Create an expiring PeerCandidate record.

## C. Pairing and approval: 21-30

21. Generate a one-time pairing token or QR code.
22. Receive the candidate NodeManifest.
23. Verify the manifest signature.
24. Compare announced and observed PeerId.
25. Check token expiry and nonce uniqueness.
26. Require human approval for a new privileged worker.
27. Assign peer scopes and capabilities.
28. Create and sign a TrustGrant.
29. Persist the trust relationship on both nodes.
30. Transition the peer from `UNTRUSTED` to `TRUSTED` only after all checks pass.

## D. Resource profiling: 31-46

31. Detect operating system and runtime.
32. Detect CPU model and architecture.
33. Detect CPU cores and threads.
34. Detect SIMD and accelerator features.
35. Detect GPU vendor and model.
36. Detect GPU count and VRAM.
37. Detect system RAM and available memory.
38. Detect storage capacity and throughput.
39. Detect network interface and bandwidth.
40. Detect installed inference engines.
41. Detect installed models and hashes.
42. Detect supported formats and quantizations.
43. Detect maximum context capacity.
44. Run a bounded, sandboxed benchmark.
45. Measure TTFT, throughput and queue capacity.
46. Sign and publish VerifiedCapabilities with an expiry.

## E. Distributed model registry: 47-58

47. Publish model hashes instead of names alone.
48. Validate ModelManifest schema.
49. Verify model file integrity.
50. Verify publisher signature where available.
51. Check model license and usage policy.
52. Validate tokenizer and chat template compatibility.
53. Validate quantization and engine compatibility.
54. Record required memory and context limits.
55. Associate the model with verified workers.
56. Replicate registry updates through the mesh.
57. Expire stale worker and model advertisements.
58. Mark models as `LOADING`, `READY`, `DEGRADED`, `UNAVAILABLE` or `REVOKED`.

## F. Resource admission: 59-70

59. Receive the requested model and task requirements.
60. Resolve model version and immutable hash.
61. Resolve context and output token requirements.
62. Resolve deadline and priority.
63. Resolve privacy and locality policy.
64. Resolve whether CPU, GPU or external peers are allowed.
65. Resolve cache affinity requirements.
66. Resolve tenant quota and token budget.
67. Filter workers by trust and revocation status.
68. Filter workers by model, readiness and capabilities.
69. Reject impossible requests before network dispatch.
70. Produce a feasible-plan candidate set.

## G. Execution planning: 71-86

71. Detect topology: local, same host, LAN, trusted cluster or public mesh.
72. Estimate queue wait for each candidate.
73. Estimate prefill and decode latency.
74. Estimate throughput and resource cost.
75. Estimate failure probability from reputation.
76. Check prefix/KV-cache affinity.
77. Check interconnect requirements.
78. Check privacy constraints against network scope.
79. Generate SingleWorker plans.
80. Generate DataParallelReplica plans.
81. Generate TensorParallel plans only for eligible clusters.
82. Generate PipelineParallel plans only for eligible clusters.
83. Generate Speculative plans only for compatible model pairs.
84. Generate PrefillDecodeDisaggregated plans only when justified.
85. Score feasible plans without overriding hard safety constraints.
86. Create, sign and trace an immutable ExecutionPlan.

## H. Atomic reservation: 87-96

87. Send a reservation request to each execution participant.
88. Validate worker capacity and plan compatibility.
89. Create a temporary reservation with TTL.
90. Return `RESERVE_ACCEPTED` or `RESERVE_REJECTED`.
91. Move a multi-worker plan to `READY_TO_COMMIT` only when all participants accept.
92. Commit all reservations atomically.
93. Abort every partial reservation if any participant rejects.
94. Refresh reservations with bounded heartbeats.
95. Release reservations on expiry, cancellation or failure.
96. Convert the reservation into an active lease after worker acceptance.

## I. Authenticated dispatch: 97-109

97. Open the versioned libp2p inference stream.
98. Authenticate the transport handshake.
99. Verify sender and target PeerId.
100. Negotiate protocol and feature versions.
101. Verify plan signature and plan ID.
102. Verify reservation ID and lease ownership.
103. Verify nonce and replay window.
104. Verify request deadline and idempotency key.
105. Verify sender scopes and tenant policy.
106. Send the dispatch envelope.
107. Worker returns `ACCEPTED` or a typed rejection.
108. Worker atomically commits the queue slot after acceptance.
109. Worker reports queue position and estimated wait.

## J. Worker execution: 110-124

110. Confirm backend and model readiness.
111. Validate the request again at the worker boundary.
112. Tokenize the prompt.
113. Apply prompt and token limits.
114. Look up a policy-safe prefix/KV cache.
115. Perform prefill.
116. Apply local admission and continuous batching.
117. Check deadline before decode.
118. Execute the engine decode loop.
119. Emit the first-token event.
120. Emit periodic heartbeats.
121. Apply stream backpressure.
122. Detect and process cancellation.
123. Update worker performance metrics.
124. Finalize engine execution and release compute slots.

## K. Streaming: 125-137

125. Emit `STARTED` with request and plan IDs.
126. Emit `PREFILL_STARTED`.
127. Emit `FIRST_TOKEN` with TTFT.
128. Emit ordered token chunks.
129. Include sequence numbers on every chunk.
130. Detect duplicate or missing chunks.
131. Buffer within a bounded window for reordering.
132. Forward validated chunks through SSE or WebSocket.
133. Update frontend response state incrementally.
134. Expose worker, model and performance status to the UI.
135. Accept cancellation from the user.
136. Emit `CANCEL_ACK` when cancellation is accepted.
137. Emit exactly one terminal event.

## L. Termination and recovery: 138-150

138. Release engine batch capacity.
139. Release KV-cache reservation.
140. Release the worker capacity lease.
141. Close the inference stream cleanly.
142. Persist the final request state.
143. Finalize audit events.
144. Aggregate metrics and trace spans.
145. Update worker reputation.
146. Update circuit-breaker state.
147. Update model and worker performance profiles.
148. Detect uncertain completion after disconnect.
149. Retry only when idempotency and deadline permit it.
150. Expose final output, partial output or typed failure to the user.

## M. Data and privacy lifecycle: 151-165

151. Assign a privacy policy to every request.
152. Support `LOCAL_ONLY`, `LAN_ONLY`, `TRUSTED_MESH` and `PUBLIC_MESH` modes.
153. Classify input as public, internal, sensitive or secret.
154. Prevent sensitive data from leaving the permitted scope.
155. Strip unnecessary metadata before dispatch.
156. Encrypt payloads end-to-end.
157. Separate routing metadata from content.
158. Restrict worker visibility to the minimum required data.
159. Apply prompt, output and trace retention TTLs.
160. Isolate tenant-specific KV caches.
161. Prevent cross-tenant prefix-cache reuse.
162. Store hashes instead of raw prompts in audit by default.
163. Support user deletion requests.
164. Redact secrets from logs and error messages.
165. Test privacy policy violations as negative E2E scenarios.

## N. Model supply chain: 166-184

166. Register a model through a signed ModelManifest.
167. Verify source and publisher metadata.
168. Verify all model file hashes.
169. Verify license and distribution policy.
170. Validate format, tokenizer and template.
171. Validate quantization and engine compatibility.
172. Scan model artifacts for prohibited content or malformed files.
173. Download by content-addressed chunks where supported.
174. Verify every downloaded chunk.
175. Assemble and hash the complete artifact.
176. Run a sandboxed smoke test.
177. Run a bounded quality/format test.
178. Require approval before public worker distribution.
179. Keep the previous version for rollback.
180. Mark incompatible workers as unavailable.
181. Support model deprecation.
182. Support emergency model revocation.
183. Propagate registry changes with versioning.
184. Test mixed model versions during rolling upgrades.

## O. Multi-tenancy: 185-198

185. Create isolated namespaces.
186. Assign tenants and user identities to namespaces.
187. Apply per-tenant authentication scopes.
188. Apply per-tenant quotas.
189. Isolate queues logically.
190. Isolate cache keys and cache storage.
191. Isolate metrics views.
192. Prevent cross-namespace dispatch.
193. Apply fair scheduling.
194. Enforce maximum active sequences.
195. Enforce maximum tokens per minute.
196. Prevent a tenant from monopolizing VRAM.
197. Support tenant-specific worker allowlists.
198. Audit all cross-boundary policy decisions.

## P. Distributed consistency: 199-215

199. Maintain a local mesh view on every node.
200. Timestamp and expire peer state.
201. Propagate presence through gossip.
202. Replicate registry updates asynchronously.
203. Resolve conflicts using version and signature.
204. Assign an owner and TTL to every reservation.
205. Allow authorized cleanup of expired reservations.
206. Require all participants for multi-worker commit.
207. Use prepare/commit/abort for distributed plans.
208. Make control messages idempotent.
209. Reject stale messages.
210. Reject duplicate nonces.
211. Maintain a reconciliation queue after reconnect.
212. Operate with a stale local registry under explicit policy.
213. Publish deltas instead of full registry snapshots.
214. Detect divergent state.
215. Mark stale or conflicting nodes for re-verification.

## Q. Fault tolerance: 216-234

216. Monitor peer heartbeats.
217. Enter `SUSPECTED_DISCONNECT` after missed heartbeats.
218. Apply a bounded grace period.
219. Query the last known request state.
220. Mark uncertain requests explicitly.
221. Release expired reservations.
222. Open a circuit breaker for unhealthy workers.
223. Select a safe fallback worker.
224. Preserve request ID during fallback.
225. Prevent duplicate visible completion.
226. Preserve partial output separately from final output.
227. Notify the user when retry is unsafe.
228. Recover queued requests after restart where durable state exists.
229. Mark running requests `UNKNOWN` when completion cannot be proven.
230. Re-probe identity after restart.
231. Re-probe backend and model readiness.
232. Restore registry state.
233. Test network partitions.
234. Test crash, restart, timeout and disk-corruption recovery.

## R. Security and anti-abuse: 235-254

235. Rate-limit peers.
236. Rate-limit identities and tenants.
237. Limit pairing attempts.
238. Detect announcement spam.
239. Limit manifest size and complexity.
240. Validate before deserialization.
241. Limit message nesting and decompression ratios.
242. Reject arbitrary remote code execution.
243. Reject arbitrary model loading.
244. Run engines with least privilege.
245. Sandbox inference backends.
246. Restrict filesystem access.
247. Restrict GPU device access.
248. Separate control and inference streams.
249. Prevent protocol downgrade.
250. Rotate pairing tokens.
251. Revoke compromised credentials.
252. Isolate security events from application logs.
253. Run dependency, secret and artifact scanning.
254. Test malicious-worker behavior.

## S. Reputation and verification: 255-270

255. Track uptime score.
256. Track success rate.
257. Track timeout rate.
258. Track p95 latency.
259. Track protocol compliance.
260. Track output validity.
261. Detect malformed output.
262. Validate response schema.
263. Support verifier nodes for critical tasks.
264. Support duplicate execution only when policy permits.
265. Compare outputs when verification is required.
266. Mark suspicious workers.
267. Reduce routing weight during investigation.
268. Trigger re-benchmarking.
269. Quarantine workers.
270. Require re-approval before returning to service.

## T. Economics and policy: 271-284

271. Support local free mode.
272. Support internal credits.
273. Allow workers to publish cost estimates.
274. Allow users to set a maximum cost.
275. Compare cost against latency and quality.
276. Reserve budget before execution.
277. Account for tokens and GPU time.
278. Calculate actual cost after completion.
279. Enforce quotas.
280. Separate settlement from the inference hot path.
281. Keep disputes outside active execution.
282. Support worker availability windows.
283. Support user preferences: cheapest, fastest, local-only or trusted-only.
284. Audit every policy and billing decision.

## U. Observability: 285-301

285. Create a trace for every request.
286. Create spans for API, planning, reservation and P2P.
287. Measure every lifecycle transition.
288. Measure admission latency.
289. Measure dispatch latency.
290. Measure queue wait.
291. Measure P2P throughput.
292. Measure tokenization, prefill and decode.
293. Measure cache hit rate.
294. Measure worker health.
295. Detect memory pressure.
296. Detect GPU thermal throttling.
297. Detect network congestion.
298. Display the selected plan.
299. Explain worker rejection decisions.
300. Allow trace export without raw prompts.
301. Generate incident reports.

## V. Upgrades and compatibility: 302-315

302. Negotiate protocol versions.
303. Advertise supported features.
304. Version the API.
305. Maintain wire compatibility.
306. Version model manifests.
307. Stage node upgrades.
308. Upgrade a worker subset first.
309. Run health probes after upgrade.
310. Compare pre- and post-upgrade metrics.
311. Roll back on regression.
312. Revoke vulnerable versions.
313. Migrate registry schemas safely.
314. Maintain a compatibility matrix.
315. Test mixed-version meshes.

## W. Self-optimization: 316-330

316. Compare estimated and actual performance.
317. Correct latency estimates.
318. Correct throughput estimates.
319. Correct failure probabilities.
320. Learn cache affinity.
321. Learn batch-size efficiency.
322. Detect engine regressions.
323. Disable speculative decoding on regression.
324. Recalculate worker weights.
325. Recalculate plan scores.
326. Propose reconfiguration.
327. Require approval for high-risk automation.
328. Keep decisions explainable.
329. Persist decision inputs and outputs.
330. Replay decisions from recorded metrics.

## X. Agent intelligence and governance: 331-345

331. Expose safe capabilities through MCP.
332. Allow an agent to discover workers.
333. Allow an agent to inspect capabilities.
334. Allow an agent to request benchmarks.
335. Allow an agent to propose worker approval.
336. Require policy approval for privileged worker admission.
337. Allow an agent to explain an execution plan.
338. Allow an agent to estimate request cost and latency.
339. Allow an agent to submit inference.
340. Allow an agent to request cancellation.
341. Allow an agent to inspect traces without secret access.
342. Prevent agents from changing policy without authorization.
343. Require human approval for destructive actions.
344. Audit every agent action and tool call.
345. Support governance policies for upgrades, revocations, resource admission and emergency shutdown.

## Release gates

### M10: production inference

Steps 1-150 pass on a two-node LAN deployment with real backend inference, streaming, cancellation, metrics, audit and recovery tests.

### M11: adaptive compute

Steps 31-46, 71-86 and 285-330 produce measured improvement over the M10 baseline.

### M12: secure mesh

Steps 11-30, 47-70, 97-109 and 199-254 pass with stale-peer, forged-worker and partition tests.

### M13: trustworthy resources

Steps 166-184 and 255-270 pass with signed models, verifier nodes and quarantine behavior.

### M14: privacy and tenancy

Steps 151-165 and 185-198 pass with namespace isolation and data locality tests.

### M15: autonomous operations

Steps 216-234, 302-330 and 331-345 pass with explainable, reversible automation.

### M16: optional economy

Steps 271-284 are enabled only after reliability, privacy and audit gates pass.

## Completion rule

The system is not complete because all numbered steps exist in documentation. A release is complete only when its gate has executable tests, observable metrics, reproducible deployment instructions and a manual recovery procedure.
