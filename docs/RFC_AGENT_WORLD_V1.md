# RFC: Agent World v1 — primul vertical slice viu

**Branch:** `feature/agent-world-v1` (nou, off `feature/saes-0.5-gateway@0b4c810`)  
**Regulă:** **zero modificări în SAES 0.5** (PR #76 înghețat) și **zero infrastructură paralelă**.  
**Scop:** o singură `WORLD` persistentă, o singură `MISSION` reală, 2 agenți generici reali care chiar folosesc framework-ul.

```
WORLD "Build X"
 ├── Research Lab — Agent A (research)
 └── Coding Lab   — Agent B (coding)
      └── MISSION (HubTask real) → RESULT (evidence + progress + reputation)
```

Fiecare pixel = stare reală, nu animație decorativă.

---

## 1. WorldState și relația cu existentul

```rust
WorldState { world_id, mission, rooms, agents, tick, events: VecDeque<Event> }
```

* **Nu este ledger/protocol nou.** Este **proiecție** (view) peste 3 surse de adevăr deja persistente:
  * `HubState` (`db/hub.json`) — `tasks/bids/teams/evidence`
  * `SocietyState` (`db/society.json`) — `outcomes/contributions/reputation`
  * `EventBus` (`InMemoryEventStore` + `agent.gateway.*`/`agent.placement.*`/`hub_events`) — corelarea `gw-*`/`pressure-*`

World nu duplică, doar **citește** (RwLock) și **proiectează** în `rooms`.

---

## 2. Schema minimă (free-form, generic agent)

```rust
World { id: "world-build-x", mission: Mission, rooms: Vec<Room>, agents: Vec<WorldAgent> }
Mission { task_id: String, title: String, reward: u64, required_capability: Option<String>,
          status: TaskStatus, team: Vec<(agent_id, share)>, evidence_id: Option<String> }
Room { id: "research-lab" | "coding-lab", capability_filter: String, agent_ids: Vec<String> }
WorldAgent { agent_id, key_id, account, declared_capabilities: Vec<String>,
             room_id, status: "idle"|"bidding"|"placed"|"executing"|"settled",
             reputation: f32, correlation_id: String }
```

* `capability_filter` = hub taxonomy snake_case (`research`, `coding`), nu enum închis.
* Agent generic: `agent_id = "agent-generic-N"`, nu Cline/Pylon/Claude/model hardcodat.

---

## 3. 4 rute — fiecare reutilizează infrastructură existentă

| Rută | Reutilizează | Ce face |
|------|--------------|---------|
| `POST /world/join` | `POST /v1/agents/onboard` + `ConsumerKeyStore::create_with_expiry` + `saes::gateway::validate_*` | Onboarding `dca_` scoped (master-gated) → `GatewaySession(gw-*)` → alocă agent în `Room` după `declared_capabilities`. Nu creează nou key-store. |
| `GET /world` | `HubState` + `SocietyState` + `EventBus::query` | Snapshot determinist: `mission = hub.tasks[world.mission.task_id]`, `rooms` derivate din `WorldAgent.room_id`, `agents` cu `reputation` din `SocietyState`. Read-only. |
| `GET /world/stream` (SSE) | `EventBus::subscribe_broadcast` + `hub/stream` | Stream live, fără polling. Push doar la `EventBus` nou. |
| `POST /world/mission` | `hub_publish_handler` + `QuotaLedger` | Master sau `dca_` cu scope `hub` creează `HubTask` (singura misiune a lui v1). World doar setează `mission.task_id`. |

Nu există `POST /v1/gateway/onboard` separat — ar dubla `agents/onboard`.

---

## 4. Evenimente EventBus care schimbă vizualul (toate cu `correlation_id gw-*`)

`agent.gateway.onboarded` → agent apare în cameră (idle)  
`agent.gateway.reserved` / `quota_denied` → agent → `bidding` / `denied`  
`agent.gateway.placed` / `no_candidate` → agent → `placed` (verzul cu `selected_peer`)  
`agent.gateway.settled` / `released` → agent → `settled`  
`hub_events` (`task_published`, `bid_placed`, `team_formed`, `task_settled` cu `evidence_id`) → mission status `Open→Bidding→Assigned→Settled` + `team` + `evidence`  
`agent.pressure.*` / `agent.placement.*` rămân interne, nu desenează separat în v1.

Orice tranziție vizuală = un event real; niciun `setInterval` fake.

---

## 5. Mapare BID → PLACED → EXECUTED → SETTLED în World

```
BID:     Agent A/B → POST /v1/hub/bid {task_id, price} → HubState.bids + hub_events "bid_placed"
PLACED:  SAES gateway → gateway_reserve_and_place(session, task_id, capability, available, offers)
          → PlacementDecision (hard gates → ±0.15 bias → peer_id tie-break) → agent.gateway.placed
EXECUTED: POST /v1/hub/team {members} → POST /v1/hub/execute {task_id}
          → HubState.mark_executing → evidence_id + Society ContributionRecord + ReputationEvent
SETTLED:  HubState.settle(task_id, evidence_id) + gateway_settle(consumed) → QuotaLedger.credit(team*share)
          → Society outcome Settled + PersonalMemory experience → World.mission.evidence_id + agents.reputation
```

World avansează `tick` doar când Hub/Society avansează — nu tick propriu.

---

## 6. Persistență minimă și ce NU duplicăm

**Persistăm:** `db/world.json` = `{world_id, mission:{task_id}, rooms:[{id, capability_filter}], agents:[{agent_id,key_id,account,room_id}]}` — atomic `tmp+rename`, load la boot cu `crate::world::load_world_state`.

**Nu persistăm/duplicăm:** `QuotaLedger` (rămâne în `runtime/api`), `HubState`/`SocietyState` (sursă), `ConsumerKeyStore` (hash `dca_`), `Placement` (rămâne `saes::placement`), `DFCP`/`Compute` (neatinse), nou token/role system, nou scheduler/economy.

Dacă `db/world.json` lipsește/corupt → World gol (mission `None`, rooms default) — nu pică nodul.

---

## 7. Criteriu exact „vertical slice complet” (2 agenți generici reali)

Slice-ul e verde când, **fără cod SAES nou**, un extern poate rula acest scenariu live pe un nod real:

1. `POST /v1/agents/onboard` cu master → primește 2× `dca_*` (`agent-generic-1` research, `agent-generic-2` coding) — demonstrare BYOA.
2. `POST /world/join` cu fiecare `dca_` + `declared_capabilities` → ambii vizibili în `GET /world` în camerele corecte (Research/Coding Lab) — filtrare `capability_filter` reală.
3. `POST /world/mission` → `HubTask` `Open` vizibil în `GET /world` + `GET /world/stream` push `task_published`.
4. Fiecare agent → `POST /v1/hub/bid` → vizibil `Bidding`.
5. Gateway `reserve_and_place` → `agent.gateway.placed` cu `selected_peer` determinist (contribution bias real).
6. `POST /v1/hub/team` + `POST /v1/hub/execute` → `HubState Settled` + `evidence_id` + `QuotaLedger` credit pe team + `Society` reputation delta.
7. `GET /world` final arată `MISSION Settled`, `EVIDENCE`, `REPUTATION` crescute, `GET /world/stream` a emis secvența `onboarded→reserved→placed→settled` cu același `gw-*`.

**Toate** cu agenți generici (`agent-generic-N`), fără hardcodare model, fără bypass de `classify`/`reserve_consumer_quota`/`select_placement`, fără infrastructură nouă.

**Non-goals v1:** Dream Rooms, Arena/competitions, economie, 10+ agenți, marketplace, grafică.

---

**Următorul pas după aprobare:** implementare pe `feature/agent-world-v1` exact conform acestui RFC, cu teste `cargo test --workspace` + `clippy -D warnings` verzi.
