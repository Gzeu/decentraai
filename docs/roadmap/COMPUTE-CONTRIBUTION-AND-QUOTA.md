# Compute Contribution & Quota

## Status

**ROADMAP — PLANNED**

This initiative defines a measured, contribution-backed access model for DecentraAI.

The core idea is intentionally simple:

> **Contribute compute → earn quota → use quota to consume compute.**

A participant that contributes CPU/GPU/RAM or other explicitly supported fabric resources receives a measurable amount of compute quota. The participant can then use that quota through DecentraAI APIs, agents, chat clients, MCP or other authorized consumers.

Quota is a **right to consume measured fabric capacity**, not a cryptocurrency, not an artificial benchmark score and not an unlimited API key.

## Product Model

There are two primary sides of the system.

```text
                    DECENTRAAI FABRIC
                           |
             +-------------+-------------+
             |                           |
        PROVIDERS                    CONSUMERS
             |                           |
     contribute compute            consume compute
             |                           |
             v                           v
        MEASURED USAGE              MEASURED USAGE
             |                           |
             +-----------> QUOTA <-------+
                           |
                           v
                       API KEY
                           |
             +-------------+-------------+
             |             |             |
          OpenClaw      Agent        Application
```

A single account may be both a provider and a consumer.

Example:

```text
Desktop GPU      + compute contribution
Laptop           + compute contribution
Phone            + limited compute contribution
                         |
                         v
                  account quota
                         |
                  API key / keys
                         |
                  inference requests
                         |
                  quota consumed
```

Adding more **verified and measured contribution** can increase the account's available quota according to the configured contribution policy.

## Critical Principle

The API key does not generate compute power.

The API key represents an authorized credential for consuming a defined quota.

```text
resources contributed
        |
        v
measured contribution
        |
        v
contribution credit
        |
        v
account quota
        |
        v
API key with limits
        |
        v
compute consumption
        |
        v
measured usage
        |
        v
quota/accounting update
```

This separation prevents credentials from becoming an unbounded source of compute entitlement.

## No Token Economy in Phase 1

The first implementation must **not** introduce:

- cryptocurrency;
- blockchain;
- speculative token prices;
- transferable financial assets;
- anonymous compute credits;
- artificial benchmark multipliers;
- a second identity system.

The initial system is a centralized, auditable compute accounting layer backed by real execution measurements.

A future marketplace or settlement layer can be considered only after the accounting model is proven.

## What Counts as Contribution

Contribution must be based on real, attributable work or reserved capacity.

Potential signals include:

- measured GPU execution time;
- measured CPU execution time;
- tokens actually processed/generated;
- actual successful execution duration;
- explicitly reserved resource capacity where the existing system can prove it;
- storage or bandwidth only if DecentraAI later defines and measures those resources as billable contribution classes.

The initial implementation should prefer inference work that the fabric can already measure reliably.

Do **not** assign contribution merely because a machine advertises:

- a GPU model;
- a CPU model;
- a theoretical TFLOPS value;
- a nominal amount of RAM;
- a claimed benchmark.

Hardware metadata can help determine eligibility, but it must not become fabricated contribution.

## Contribution Units

The accounting layer should use a versioned internal unit rather than hard-code a future financial value.

Conceptually:

```text
ContributionUnit
├── amount
├── resource_class
├── measurement_basis
├── execution_id
├── worker_id
├── timestamp
├── provenance
└── accounting_version
```

Every credited contribution must be traceable to real fabric evidence.

Examples:

```text
GPU execution
  worker = desktop
  execution = abc123
  measured GPU execution = 42 s
  contribution = X units

CPU execution
  worker = laptop
  execution = def456
  measured CPU execution = 95 s
  contribution = Y units
```

The exact conversion formula must be deterministic, versioned and documented before production accounting is enabled.

## Quota

Quota is the amount of compute a consumer is currently authorized to consume.

A quota record should conceptually contain:

```text
Quota
├── account_id
├── available
├── reserved
├── consumed
├── earned
├── adjustment
├── expires_at (optional)
├── policy_version
└── provenance
```

Do not represent missing values as zero.

Unknown or unavailable accounting data must remain explicitly distinguishable from a real zero balance.

## Reservation vs Consumption

Quota must distinguish between:

```text
AVAILABLE
   |
   +---- reserve for request ---->
   |                              RESERVED
   |                                  |
   |                              execution
   |                                  |
   +------------------------------> CONSUMED
```

If a request is cancelled before consumption, unused reserved quota should be released.

If execution fails, the accounting policy must distinguish between:

- no work performed;
- partial work performed;
- completed work;
- infrastructure failure;
- consumer cancellation.

Never charge the same execution twice.

This should reuse existing request/execution/reservation identifiers and idempotency semantics.

## API Keys

DecentraAI should eventually provide first-class consumer API keys.

A key should identify an account/consumer context and reference policy, not hold the quota itself.

Conceptually:

```text
API key
├── key_id
├── secret material
├── account_id
├── status: ACTIVE / REVOKED
├── permissions
├── allowed models/capabilities (optional)
├── rate limits
├── quota limits
├── created_at
└── expires_at (optional)
```

Security requirements:

- show secret material only at creation/rotation;
- store only a secure verifier/hash where practical;
- support revocation;
- support rotation;
- support per-key quotas/rate limits;
- never log full keys;
- never expose provider credentials to consumers;
- never let an API key bypass worker trust/policy;
- never let an API key grant control-plane privileges unless explicitly authorized.

The existing `dsk_` credential model should be reused or extended deliberately rather than creating an unrelated token authority.

## Multiple Keys

One account may have multiple consumer keys.

Example:

```text
Account
|
+-- Personal key       -> quota limit 100
+-- OpenClaw key       -> quota limit  50
+-- Development key    -> quota limit  25
```

Keys can have independent:

- rate limits;
- model/capability restrictions;
- expiration;
- quota ceilings;
- revocation state.

All keys still draw from the same authoritative account-level entitlement unless a future accounting policy explicitly separates balances.

## Contribution → Quota Policy

The conversion from contribution to quota must be:

- deterministic;
- versioned;
- explainable;
- based on measured evidence;
- resistant to duplicate execution reports;
- independent of hardware marketing claims.

A conceptual policy is:

```text
verified contribution
        |
        +-- measurement quality
        +-- successful execution
        +-- resource class
        +-- accounting policy version
        |
        v
contribution units
        |
        v
account quota
```

The policy must not silently change retroactively. Historical records must retain the policy version that produced them.

## Anti-Abuse Rules

Contribution accounting must assume that workers can be unreliable or malicious.

The system must prevent:

- duplicate execution credit;
- self-reported fake tokens;
- inflated resource claims;
- replayed completion events;
- credit for failed/unperformed work;
- credit generated by an untrusted worker;
- quota creation by editing local state;
- using multiple workers to claim the same execution.

The existing trust, signed fabric messages, execution identifiers, measured telemetry and replay/idempotency protections should remain authoritative.

Do not build a second reputation system solely for quota accounting.

## Provider and Consumer Separation

A provider contributes resources.

A consumer spends quota.

The same account may do both.

```text
Provider account
     |
     | contributes
     v
 measured fabric work
     |
     v
 earned quota
     |
     +-------------------+
                         |
Consumer request <-------+
                         |
                         v
                   execution
                         |
                         v
                   consumed quota
```

This creates a closed-loop compute contribution model without requiring an external payment system.

## Quota Growth

Quota growth should be proportional to **verified contribution**, not simply the number of devices connected.

Example concept:

```text
1 GPU worker
   -> measured contribution
   -> quota increases

+ laptop worker
   -> additional measured contribution
   -> quota increases again

+ phone worker
   -> only if it actually performs supported work
   -> additional measured contribution
```

A phone that merely connects but performs no work should not generate meaningful compute quota.

Likewise, a powerful GPU that is idle should not automatically generate unlimited quota.

## Consumer Controls

Consumers should eventually be able to see:

```text
Quota
  Earned       1,240
  Reserved        80
  Consumed       410
  Available      750

Contribution
  Desktop       820
  Laptop        310
  Phone         110

Usage
  OpenClaw      220
  Development   130
  Other          60
```

All figures must come from authoritative accounting records.

## Agent Integration

Agents and applications should be able to consume the fabric using API keys.

Example:

```text
OpenClaw
   |
   | Authorization: Bearer dca_...
   v
DecentraAI API
   |
   +-- authenticate consumer
   +-- check permission
   +-- check quota
   +-- resolve intent/capability
   +-- find fabric fit
   +-- reserve resources/quota
   +-- execute
   +-- measure
   +-- settle usage
   v
result + usage/provenance
```

MCP should expose the same consumer authorization semantics where applicable.

An agent must never receive direct worker credentials.

## Resource Contribution vs Consumer Quota

Do not assume a one-to-one relationship between a particular worker and a particular consumer request.

The fabric is a shared pool:

```text
Worker A ─┐
Worker B ─┼──> Fabric capacity ──> Consumer 1
Worker C ─┤                       Consumer 2
Worker D ─┘                       Consumer 3
```

The planner remains responsible for selecting eligible workers.

Quota answers **"may this consumer use compute?"**

Fabric fit answers **"where can this request actually run?"**

Policy/trust answers **"is this worker allowed to execute it?"**

These must remain separate decisions.

## Accounting and Existing DecentraAI Systems

Reuse existing authoritative data wherever possible:

- `ExecutionDecision`;
- execution/request identifiers;
- reservations;
- `TokenUsage`;
- execution statistics;
- measured worker performance;
- worker identity;
- trust/policy;
- resource provenance;
- recovery/idempotency.

Do not create a second execution ledger if an existing authoritative event/history mechanism can be extended safely.

## Roadmap

### Q1 — Accounting Foundation

- [ ] Define authoritative contribution evidence.
- [ ] Define versioned contribution unit and conversion policy.
- [ ] Define account-level quota representation.
- [ ] Reuse existing execution/reservation identifiers.
- [ ] Define idempotent accounting events.
- [ ] Define provenance for every credit/debit.

### Q2 — Consumer API Keys

- [ ] Add account-scoped API keys using the existing authentication architecture.
- [ ] Key creation, rotation and revocation.
- [ ] Per-key permissions and rate limits.
- [ ] Quota-aware request authorization.
- [ ] Never expose worker credentials to consumers.

### Q3 — Contribution-backed Quota

- [ ] Credit verified provider contribution.
- [ ] Debit measured consumer usage.
- [ ] Reserve quota before execution.
- [ ] Release unused reservations.
- [ ] Handle partial/failure/cancellation accounting.
- [ ] Prevent duplicate credit/debit.

### Q4 — Operator / Agent Visibility

- [ ] Dashboard quota card.
- [ ] Contribution by worker.
- [ ] Usage by API key/application.
- [ ] Quota reservation/consumption timeline.
- [ ] MCP read-only quota/accounting views.
- [ ] Explain why quota was granted or denied.

### Q5 — Delegation and Policies

- [ ] Optional delegated quota for another API key/account.
- [ ] Spending limits.
- [ ] model/capability restrictions.
- [ ] expiration policies.
- [ ] organization/project quotas.

### Q6 — Marketplace / Settlement (Future Research)

- [ ] Only after contribution accounting is proven.
- [ ] Research external settlement/payment models.
- [ ] Evaluate whether transferable credits are actually necessary.
- [ ] Do not introduce blockchain merely because quota exists.

## Acceptance Criteria

The initiative is successful when:

1. a trusted worker performs real eligible work;
2. the work is attributable to a real execution;
3. contribution is measured from authoritative data;
4. the contributor receives deterministic quota according to a versioned policy;
5. a consumer API key can authenticate against that account;
6. a request checks quota before execution;
7. quota is reserved and then settled from measured usage;
8. failed/cancelled executions cannot create duplicate credit;
9. the dashboard can explain earned, reserved, consumed and available quota;
10. an agent can consume compute without receiving worker credentials;
11. adding verified resources can increase the contributor's quota only through measured contribution;
12. no cryptocurrency or speculative token system is required for the core functionality.

## Non-Goals

This initiative does not initially attempt to:

- create a public cryptocurrency;
- create a permissionless global compute marketplace;
- price compute using arbitrary market rates;
- guarantee financial returns to providers;
- expose private device data;
- make every device equally valuable;
- turn nominal hardware specifications into guaranteed credits.

## Current Status

**Planned.** DecentraAI already has many of the required authoritative building blocks: worker identity/trust, execution IDs, reservations, token usage, resource provenance, measured performance, history, API/MCP control surfaces and lightweight workers. The missing layer is an explicit, versioned contribution-to-quota accounting model.
