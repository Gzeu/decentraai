# MX-8004 WRITE PATH — source-backed verification

Research phase. **No transaction code written.** Classification per
capability: VERIFIED / PARTIALLY VERIFIED / UNVERIFIED / NOT AVAILABLE.

## Verified sources

| ID | Source | Fetched |
|---|---|---|
| S1 | `multiversx/mx-agent-standard` @ master → `mx8004_technical_specs.md` ("Technical Specification MX-8004 v2.1", Rust contract-level) | 2026-08-23 |
| S2 | same repo → `starter_kit_technical_specs.md` (tx-data format + lifecycle) | 2026-08-23 |
| S3 | `agents.multiversx.com/skill.md` (HTTP-wrapper API reference) | 2026-08-23 |

## Architecture truth (from S1)

Three registries — **Identity**, **Validation**, **Reputation**.
There is NO separate escrow contract in the v2.1 spec: payment escrowing
happens INSIDE the Validation job flow
(`init_job_with_payment` forwards payment to the agent owner).

## Capability table

| Capability | Contract | Function | Args | Caller | Devnet HTTP wrapper | Source | Status |
|---|---|---|---|---|---|---|---|
| Issue NFT collection | Identity | `issue_token(name, ticker)` | display, ticker | Owner only (+EGLD cost) | n/a | S1 §1.3 | VERIFIED |
| **Agent registration** | Identity | `register_agent(name, uri, public_key, metadata?)` | buffers + optional kv | Public once token issued | `POST /agents` (S3) | S1+S2+S3 | VERIFIED |
| Tx-data format | Identity | `register_agent@nameHex@uriHex@pubKeyHex[@kHex@vHex…]` | hex-encoded | signer=owner wallet | — | S2 §3.1 | VERIFIED |
| Update identity (uri/key/metadata) | Identity | `update_agent(new_uri,new_public_key,metadata?)` — sends Agent NFT to contract (Transfer-Execute) | buffers | NFT owner only | not in S3 | S1 §1.3 | VERIFIED |
| Metadata upsert / read | Identity | `set_metadata(nonce,entries)` / `get_metadata(nonce,key)` view | | NFT owner / public | not in S3 | S1 | VERIFIED |
| Read agent / nonce-by-address | Identity | `get_agent(nonce)` · `get_agent_id(address)` views | | public | `GET /agents/:nonce`, list | S1+S3 | VERIFIED |
| Job init WITH payment | Validation | `init_job_with_payment(job_id, agent_nonce, service_id)` | buffers+nonce | Employer (public) | not in S3 | S1 §2.2 | VERIFIED |
| Proof submission | Validation | `submit_proof(job_id, proof)` | hash/data-URI | Agent | not in S3 | S1 | VERIFIED |
| Validation request | Validation | `validation_request(job_id, validator_address, request_uri, request_hash)` | | Agent OWNER (cross-contract check) | not in S3 | S1 | VERIFIED |
| Validation response | Validation | `validation_response(request_hash, response, response_uri, response_hash, tag)` | score 0–100 inside response | Nominated validator ONLY | not in S3 | S1 | VERIFIED |
| Job cleanup | Validation | `clean_old_jobs(job_ids)` | jobs older than 3 days | public | not in S3 | S1 | VERIFIED |
| Feedback (rating) | Reputation | `submit_feedback(job_id, agent_nonce, rating)` | employer-checked cross-contract; dup-prevention | Job employer | not in S3 | S1 §3 | VERIFIED |
| Response append | Reputation | `append_response(job_id, response_uri)` | permissionless (ERC-8004) | public | not in S3 | S1 | VERIFIED |
| Reputation query | Reputation | storage read (`reputationScore`,`totalJobs`) | | public read | `GET /reputations/agents/:nonce` | S1+S3 | VERIFIED |
| **Devnet contract addresses** | all three | — | — | — | NOT published in S1/S2/S3 | — | **UNVERIFIED** |
| Live API reachability | wrapper | DNS for `devnet-mx8004-api.multiversx.com` did NOT resolve from our environment (other *.multiversx.com hosts did) | | — | — | live probe | PARTIALLY VERIFIED (documented but unreachable here) |
| Escrow as standalone contract | — | none in v2.1 spec | — | — | — | S1 | **NOT AVAILABLE** (job-payment flow instead) |
| Anchoring endpoint | — | none documented | — | — | — | — | **UNVERIFIED** |
| Mainnet | — | "Coming soon" | | | | S3 | NOT AVAILABLE |

## DecentraAI mapping (no duplication)

| DecentraAI | → MX-8004 | Mechanism |
|---|---|---|
| SignedComputeReceipt | `submit_proof(job_id, proof)` | proof = BLAKE3 anchor of receipt/evidence bytes |
| EvidenceChain | `validation_request` / `validation_response` | our evidence refs become request/response hashes; validator = nominated peer |
| CompensationLedger | job payment flow (`init_job_with_payment`) | ledger stays authoritative internally; MX records the job/payment event |
| Agent OS Identity | Identity Registry | Ed25519 pubkey byte-equality link (already implemented: `verify_link`) |
| EconomicEvidence | settlement prep | evidence_hash carried in `anchoring_payload` (preparation shape) |

## Key separation (preserved, never collapsed)

node Ed25519 key (signs receipts + agent auth) ≠ wallet key (funds txs,
held by OPERATOR in a secret manager) ≠ validator role (a nominated peer).
MX-8004 itself enforces the split: agent auth key vs NFT-owner wallet.

## Minimal implementation proposal (AFTER this research — not implemented)

- Phase 1 (operator-controlled registration): build tx-data string
  `register_agent@…` from validated types + operator signs/sends with their
  wallet tooling. Our crate emits the payload; never signs.
- Phase 2 (proof anchoring): after a job exists, emit `submit_proof` tx-data
  carrying our `EconomicEvidence.evidence_hash`.
- Phase 3 (validation): nominate an operator-chosen validator; consume
  `validation_response` score into Collective Memory as a VERIFIED
  observation.
- Phase 4 (optional): leverage job-payment flow instead of building escrow.

All four require the devnet contract addresses (UNVERIFIED) and an
operator-funded wallet — both outside this repository's scope.
