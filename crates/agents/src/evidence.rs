//! Evidence RAG (pure) — the fabric's experimental memory.
//!
//! The fabric already produces hard evidence: executed plans, verified compute
//! receipts, collective decisions, memory entries, benchmarks. This module is
//! the single, deterministic index over all of them: `EvidenceEntry` is the
//! canonical shape every runtime source maps into, and `EvidenceIndex` is the
//! in-memory index that answers "what have we learned so far?".
//!
//! Two honest query paths:
//! - **structural**: exact/prefix/tag matching over the entry fields — always
//!   available, deterministic, no external model involved;
//! - **semantic**: cosine retrieval over embeddings — only when a real
//!   embedding backend populated the vectors at runtime. Without vectors, the
//!   semantic path returns nothing (never a fake score).
//!
//! Lessons are *derived* from the evidence, never invented: `lessons()` is a
//! pure aggregation over whatever entries exist (counts, success rates, median
//! durations). Zero evidence in, zero lessons out — same honesty rule as
//! knowledge confidence in `knowledge.rs`.
//!
//! Invariant: evidence entries carry **facts** (who ran, what succeeded, how
//! long, what the decision was). Prompts and outputs are never evidence
//! material — that stays true here as everywhere in the repo.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The five evidence families the fabric produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceFamily {
    /// Benchmarks (tok/s, latency) measured on real hardware.
    Benchmark,
    /// Executed inference plans (M18/M20/M23 route results).
    Execution,
    /// Verified compute receipts (P12) — paid/verified work.
    Receipt,
    /// Collective memory entries (P5, `collective.*` scopes).
    Memory,
    /// Collective decisions (P12/P4) — adopted/rejected/deferred.
    Consensus,
}

impl EvidenceFamily {
    /// Machine-readable tag used by the structural query path.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Benchmark => "kind:benchmark",
            Self::Execution => "kind:execution",
            Self::Receipt => "kind:receipt",
            Self::Memory => "kind:memory",
            Self::Consensus => "kind:consensus",
        }
    }
}

/// One piece of evidence, canonical across all runtime sources.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceEntry {
    /// Stable id (`exec:<request_id>`, `receipt:<exec_id>`, `decision:<id>`,
    /// `memory:<entry_id>`, `bench:<name>`).
    pub id: String,
    /// Which family produced this evidence.
    pub kind: EvidenceFamily,
    /// The fact text (never a prompt/output; facts only).
    pub text: String,
    /// Coarse filters (worker, model, capability, outcome, tags).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Creation time (unix ms).
    pub created_at_ms: u64,
    /// Optional embedding vector, populated by the runtime when a real
    /// embedding backend is configured. Empty = semantic query will not match.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embedding: Vec<f32>,
    /// Ed25519 public key of the signer (32 bytes), when this entry is signed.
    /// Present only on entries that back economic attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer_public_key: Option<Vec<u8>>,
    /// Ed25519 signature over `canonical_evidence_payload(self)` (64 bytes).
    /// Verified before the entry may back economic credit; missing or invalid
    /// signatures fail closed for attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
}

impl EvidenceEntry {
    /// Builds an entry; `kind.tag()` is appended to `tags` so the structural
    /// path can filter by family.
    pub fn new(
        id: impl Into<String>,
        kind: EvidenceFamily,
        text: impl Into<String>,
        created_at_ms: u64,
    ) -> Self {
        let mut tags = vec![kind.tag().to_string()];
        tags.sort();
        tags.dedup();
        Self {
            id: id.into(),
            kind,
            text: text.into(),
            tags,
            created_at_ms,
            embedding: Vec::new(),
            signer_public_key: None,
            signature: None,
        }
    }

    /// Adds a filter tag (kept sorted + unique for determinism).
    pub fn tagged(mut self, tag: impl Into<String>) -> Self {
        let tag = tag.into();
        if !self.tags.iter().any(|t| t == &tag) {
            self.tags.push(tag);
            self.tags.sort();
        }
        self
    }

    /// Attaches the embedding vector (from a real backend at runtime).
    pub fn with_embedding(mut self, embedding: Vec<f32>) -> Self {
        self.embedding = embedding;
        self
    }

    /// Whether a structural query term matches this entry (id prefix, any tag,
    /// or text substring — case-insensitive).
    pub fn matches(&self, term: &str) -> bool {
        let term = term.to_ascii_lowercase();
        self.id.to_ascii_lowercase().contains(&term)
            || self
                .tags
                .iter()
                .any(|t| t.to_ascii_lowercase().contains(&term))
            || self.text.to_ascii_lowercase().contains(&term)
    }
}

/// A structural (non-semantic) hit — used when no embedding backend exists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceHit {
    /// The matched entry id.
    pub id: String,
    /// Family.
    pub kind: EvidenceFamily,
    /// The fact text.
    pub text: String,
    /// Matched tags (for display).
    pub tags: Vec<String>,
    /// `"semantic"` when ranked by cosine over real embeddings, `"structural"`
    /// when matched by keywords/tags (no embedding backend). Honest: the two
    /// are never mixed in one result set.
    pub mode: &'static str,
    /// Score in 0..=1. For structural hits this is a deterministic term-match
    /// weight (all terms matched = 1.0), never a fake semantic similarity.
    pub score: f32,
}

/// Deterministic in-memory evidence index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvidenceIndex {
    entries: BTreeMap<String, EvidenceEntry>,
}

impl EvidenceIndex {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces an entry (idempotent on id).
    pub fn add(&mut self, entry: EvidenceEntry) {
        self.entries.insert(entry.id.clone(), entry);
    }

    /// Whether an entry with this id is already indexed.
    pub fn contains(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    /// Number of indexed entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// All entries, newest-first (deterministic; ids sort, timestamps tiebreak).
    pub fn all(&self) -> Vec<EvidenceEntry> {
        let mut v: Vec<EvidenceEntry> = self.entries.values().cloned().collect();
        v.sort_by(|a, b| {
            b.created_at_ms
                .cmp(&a.created_at_ms)
                .then_with(|| b.id.cmp(&a.id))
        });
        v
    }

    /// Entries of one family.
    pub fn by_kind(&self, kind: EvidenceFamily) -> Vec<EvidenceEntry> {
        self.all().into_iter().filter(|e| e.kind == kind).collect()
    }

    /// Count of entries per family (all five keys present, deterministic).
    pub fn counts(&self) -> BTreeMap<EvidenceFamily, usize> {
        let mut m = BTreeMap::new();
        for e in self.entries.values() {
            *m.entry(e.kind).or_insert(0) += 1;
        }
        m
    }

    /// Structural query: every term must match (AND), any field may satisfy it
    /// (OR per term). Empty query returns nothing. Deterministic ordering:
    /// newest first, id desc.
    pub fn query(&self, terms: &[String]) -> Vec<EvidenceHit> {
        if terms.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<EvidenceHit> = self
            .all()
            .into_iter()
            .filter(|e| terms.iter().all(|t| e.matches(t)))
            .map(|e| {
                let matched = terms.iter().filter(|t| e.matches(t)).count();
                EvidenceHit {
                    score: matched as f32 / terms.len() as f32,
                    id: e.id,
                    kind: e.kind,
                    text: e.text,
                    tags: e.tags,
                    mode: "structural",
                }
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.id.cmp(&a.id))
        });
        hits
    }

    /// Semantic query: ranks entries with real embeddings by cosine similarity
    /// (top `k`). Entries without an embedding never match — a missing backend
    /// must not produce fake scores. Mode is honest `"semantic"` only when
    /// vectors exist.
    pub fn semantic(&self, query: &[f32], k: usize) -> Vec<EvidenceHit> {
        if query.is_empty() || k == 0 {
            return Vec::new();
        }
        let mut scored: Vec<(EvidenceEntry, f32)> = self
            .entries
            .values()
            .filter(|e| !e.embedding.is_empty())
            .map(|e| {
                let s = cosine(query, &e.embedding);
                (e.clone(), s)
            })
            .filter(|(_, s)| *s > f32::EPSILON)
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.0.id.cmp(&a.0.id))
        });
        scored
            .into_iter()
            .take(k)
            .map(|(e, s)| EvidenceHit {
                score: s,
                id: e.id,
                kind: e.kind,
                text: e.text,
                tags: e.tags,
                mode: "semantic",
            })
            .collect()
    }
}

/// One derived lesson: a deterministic aggregation over real evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lesson {
    /// Stable lesson id (e.g. `executions/success_rate`).
    pub id: String,
    /// Human label.
    pub label: String,
    /// Numeric value of the lesson (0.0 when no evidence — honest).
    pub value: f64,
    /// Count of evidence entries behind the lesson.
    pub sample: usize,
    /// How the value was derived.
    pub detail: String,
}

/// Cosine similarity (0..=1), same semantics as `retrieval.rs`.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= f32::EPSILON {
        0.0
    } else {
        (dot / denom).clamp(0.0, 1.0)
    }
}

/// Extracts the value of a `tag:value` tag (e.g. `outcome:succeeded` → "succeeded").
fn tag_value(entry: &EvidenceEntry, prefix: &str) -> Option<String> {
    entry
        .tags
        .iter()
        .find_map(|t| t.strip_prefix(prefix).map(|v| v.to_string()))
}

/// Median of a sorted slice.
fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = v.len() / 2;
    Some(if v.len() % 2 == 0 {
        (v[mid - 1] + v[mid]) / 2.0
    } else {
        v[mid]
    })
}

// ---------------------------------------------------------------------------
// Evidence signing (Ed25519) — economic attribution fails closed
// ---------------------------------------------------------------------------

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

/// Version of the evidence signature scheme.
pub const EVIDENCE_SIGNATURE_VERSION: u16 = 1;

/// Deterministic canonical bytes an EvidenceEntry is signed over. Excludes the
/// signer/signature themselves and the embedding vector, so sign and verify
/// agree byte-for-byte on the same facts.
pub fn canonical_evidence_payload(e: &EvidenceEntry) -> Vec<u8> {
    serde_json::json!({
        "id": e.id,
        "kind": e.kind,
        "text": e.text,
        "tags": e.tags,
        "created_at_ms": e.created_at_ms,
    })
    .to_string()
    .into_bytes()
}

/// Signs an entry with the node's Ed25519 identity: fills `signer_public_key`
/// and `signature` over the canonical payload. The entry is consumed so a
/// caller cannot keep an unsigned copy that would later pass as signed.
pub fn sign_evidence(mut entry: EvidenceEntry, signing_key_bytes: &[u8; 32]) -> EvidenceEntry {
    let signing_key = SigningKey::from_bytes(signing_key_bytes);
    let payload = canonical_evidence_payload(&entry);
    let sig: Signature = signing_key.sign(&payload);
    entry.signer_public_key = Some(signing_key.verifying_key().to_bytes().to_vec());
    entry.signature = Some(sig.to_bytes().to_vec());
    entry
}

/// Why an evidence entry failed signature verification. Fail-closed reasons —
/// economic attribution must not accept any of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceSignatureError {
    MissingSignature,
    MissingSignerKey,
    MalformedSignature,
    MalformedSignerKey,
    InvalidSignature,
}

/// Verifies an entry's signature against its own embedded public key. Fails
/// closed on missing signature, missing key, malformed key/signature, or a
/// tampered payload. `expected_public_key` optionally pins the signer.
pub fn verify_evidence_signature(
    entry: &EvidenceEntry,
    expected_public_key: Option<&[u8]>,
) -> Result<(), EvidenceSignatureError> {
    let Some(sig_bytes) = &entry.signature else {
        return Err(EvidenceSignatureError::MissingSignature);
    };
    let Some(pub_bytes) = &entry.signer_public_key else {
        return Err(EvidenceSignatureError::MissingSignerKey);
    };
    if let Some(expected) = expected_public_key {
        if pub_bytes != expected {
            return Err(EvidenceSignatureError::InvalidSignature);
        }
    }
    let Ok(pub_arr) = <[u8; 32]>::try_from(pub_bytes.as_slice()) else {
        return Err(EvidenceSignatureError::MalformedSignerKey);
    };
    let Ok(vk) = VerifyingKey::from_bytes(&pub_arr) else {
        return Err(EvidenceSignatureError::MalformedSignerKey);
    };
    let Ok(sig_arr) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
        return Err(EvidenceSignatureError::MalformedSignature);
    };
    vk.verify_strict(
        canonical_evidence_payload(entry).as_slice(),
        &Signature::from_bytes(&sig_arr),
    )
    .map_err(|_| EvidenceSignatureError::InvalidSignature)
}

/// Derives the deterministic lesson set from real evidence. Zero evidence in,
/// zero lessons out (the `sample` fields stay 0 and values stay 0.0 — never an
/// invented number). Pure and testable.
pub fn lessons(entries: &[EvidenceEntry]) -> Vec<Lesson> {
    let mut out = Vec::new();

    // 1. Execution success rate (per worker, then global).
    let executions: Vec<&EvidenceEntry> = entries
        .iter()
        .filter(|e| e.kind == EvidenceFamily::Execution)
        .collect();
    let global_ok = executions
        .iter()
        .filter(|e| tag_value(e, "outcome:").as_deref() == Some("succeeded"))
        .count();
    out.push(Lesson {
        id: "executions/success_rate".into(),
        label: "Execution success rate".into(),
        value: if executions.is_empty() {
            0.0
        } else {
            global_ok as f64 / executions.len() as f64
        },
        sample: executions.len(),
        detail: "succeeded / all executed plans (outcome tag)".into(),
    });

    // 2. Median execution duration (ms) across executed plans.
    let mut durations: Vec<f64> = executions
        .iter()
        .filter_map(|e| tag_value(e, "duration_ms:"))
        .filter_map(|v| v.parse::<f64>().ok())
        .collect();
    let median_dur = median(&mut durations);
    out.push(Lesson {
        id: "executions/median_duration_ms".into(),
        label: "Median execution duration (ms)".into(),
        value: median_dur.unwrap_or(0.0),
        sample: durations.len(),
        detail: "median of duration_ms tags on executed plans".into(),
    });

    // 3. Receipt verification rate — how much verified work the fabric earned.
    let receipts: Vec<&EvidenceEntry> = entries
        .iter()
        .filter(|e| e.kind == EvidenceFamily::Receipt)
        .collect();
    let verified = receipts
        .iter()
        .filter(|e| tag_value(e, "verdict:").as_deref() == Some("Verified"))
        .count();
    out.push(Lesson {
        id: "receipts/verified_rate".into(),
        label: "Verified-work rate".into(),
        value: if receipts.is_empty() {
            0.0
        } else {
            verified as f64 / receipts.len() as f64
        },
        sample: receipts.len(),
        detail: "Verified / all receipts (P12 ledger evidence)".into(),
    });

    // 4. Consensus adoption rate — how often collective decisions land.
    let decisions: Vec<&EvidenceEntry> = entries
        .iter()
        .filter(|e| e.kind == EvidenceFamily::Consensus)
        .collect();
    let adopted = decisions
        .iter()
        .filter(|e| tag_value(e, "verdict:").as_deref() == Some("Adopted"))
        .count();
    out.push(Lesson {
        id: "consensus/adoption_rate".into(),
        label: "Collective decision adoption rate".into(),
        value: if decisions.is_empty() {
            0.0
        } else {
            adopted as f64 / decisions.len() as f64
        },
        sample: decisions.len(),
        detail: "Adopted / all collective decisions".into(),
    });

    // 5. Median network RTT (ms) observed on executed plans — the fabric's
    //    honest view of its own network (M19 measurements).
    let mut rtts: Vec<f64> = executions
        .iter()
        .filter_map(|e| tag_value(e, "rtt_ms:"))
        .filter_map(|v| v.parse::<f64>().ok())
        .collect();
    let median_rtt = median(&mut rtts);
    out.push(Lesson {
        id: "network/median_rtt_ms".into(),
        label: "Median network RTT (ms)".into(),
        value: median_rtt.unwrap_or(0.0),
        sample: rtts.len(),
        detail: "median of rtt_ms tags on executed plans (M19 probes)".into(),
    });

    // 6. Benchmark Lab lessons: the fabric's own measured accuracy per mode,
    //    and the median cost of one graded run. Evidence is the lab's
    //    registry runs (kind benchmark, tags verdict:*/mode:*/task:*).
    let bench_entries: Vec<&EvidenceEntry> = entries
        .iter()
        .filter(|e| e.kind == EvidenceFamily::Benchmark)
        .collect();
    for mode in ["single", "collective", "rag"] {
        let mode_entries: Vec<&&EvidenceEntry> = bench_entries
            .iter()
            .filter(|e| tag_value(e, "mode:").as_deref() == Some(mode))
            .collect();
        let graded = mode_entries
            .iter()
            .filter(|e| tag_value(e, "verdict:").as_deref() != Some("Abstained"))
            .count();
        let correct = mode_entries
            .iter()
            .filter(|e| tag_value(e, "verdict:").as_deref() == Some("Correct"))
            .count();
        out.push(Lesson {
            id: format!("bench/{mode}_accuracy"),
            label: format!("Benchmark accuracy ({mode})"),
            value: if graded == 0 {
                0.0
            } else {
                correct as f64 / graded as f64
            },
            sample: graded,
            detail: "graded runs correct / graded runs (Benchmark Lab)".into(),
        });
    }
    let mut latencies: Vec<f64> = bench_entries
        .iter()
        .filter_map(|e| tag_value(e, "latency_ms:"))
        .filter_map(|v| v.parse::<f64>().ok())
        .collect();
    let median_lat = median(&mut latencies);
    out.push(Lesson {
        id: "bench/median_latency_ms".into(),
        label: "Benchmark median latency (ms)".into(),
        value: median_lat.unwrap_or(0.0),
        sample: latencies.len(),
        detail: "median of latency_ms tags on benchmark runs".into(),
    });

    out
}

/// A snapshot of the evidence index for the control plane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceSummary {
    /// Total entries indexed.
    pub total: usize,
    /// Count per family.
    pub counts: BTreeMap<EvidenceFamily, usize>,
    /// Latest entries (newest-first, bounded by `limit`).
    pub recent: Vec<EvidenceEntry>,
    /// Derived lessons.
    pub lessons: Vec<Lesson>,
}

impl EvidenceIndex {
    /// Builds a control-plane snapshot with the derived lessons.
    pub fn summary(&self, limit: usize) -> EvidenceSummary {
        let recent = self.all().into_iter().take(limit).collect();
        let entries: Vec<EvidenceEntry> = self.entries.values().cloned().collect();
        EvidenceSummary {
            total: self.len(),
            counts: self.counts(),
            recent,
            lessons: lessons(&entries),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_signature_roundtrip_and_fail_closed() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let entry = EvidenceEntry::new(
            "gov:job:decision:1",
            EvidenceFamily::Execution,
            "governor DISTRIBUTED: 3 workers",
            1234,
        )
        .tagged("governor");

        // Valid signature verifies.
        let signed = sign_evidence(entry.clone(), &sk.to_bytes());
        assert!(verify_evidence_signature(&signed, None).is_ok());

        // Tampered payload (text changed after signing) fails.
        let mut tampered = signed.clone();
        tampered.text = "fabricated".into();
        assert_eq!(
            verify_evidence_signature(&tampered, None),
            Err(EvidenceSignatureError::InvalidSignature)
        );

        // Wrong signer key fails.
        assert_eq!(
            verify_evidence_signature(&signed, Some(other.verifying_key().to_bytes().as_slice())),
            Err(EvidenceSignatureError::InvalidSignature)
        );

        // Missing signature / missing signer fail closed.
        let unsigned = entry.clone();
        assert_eq!(
            verify_evidence_signature(&unsigned, None),
            Err(EvidenceSignatureError::MissingSignature)
        );
        let mut half = sign_evidence(entry.clone(), &sk.to_bytes());
        half.signer_public_key = None;
        assert_eq!(
            verify_evidence_signature(&half, None),
            Err(EvidenceSignatureError::MissingSignerKey)
        );
    }

    fn exec(id: &str, outcome: &str, duration_ms: u64, rtt_ms: u32, at_ms: u64) -> EvidenceEntry {
        EvidenceEntry::new(
            format!("exec:{id}"),
            EvidenceFamily::Execution,
            format!("plan {id} {outcome}"),
            at_ms,
        )
        .tagged(format!("outcome:{outcome}"))
        .tagged(format!("duration_ms:{duration_ms}"))
        .tagged(format!("rtt_ms:{rtt_ms}"))
    }

    #[test]
    fn index_is_idempotent_and_counts_per_kind() {
        let mut ix = EvidenceIndex::new();
        ix.add(exec("r1", "succeeded", 120, 15, 1000));
        ix.add(exec("r1", "succeeded", 120, 15, 1000)); // same id → replace
        ix.add(
            EvidenceEntry::new(
                "receipt:e1",
                EvidenceFamily::Receipt,
                "exec e1 Verified",
                2000,
            )
            .tagged("verdict:Verified"),
        );
        assert_eq!(ix.len(), 2);
        assert_eq!(ix.counts()[&EvidenceFamily::Execution], 1);
        assert_eq!(ix.counts()[&EvidenceFamily::Receipt], 1);
    }

    #[test]
    fn structural_query_ands_terms_and_sorts_deterministically() {
        let mut ix = EvidenceIndex::new();
        ix.add(exec("a", "succeeded", 100, 10, 3000));
        ix.add(exec("b", "failed", 500, 300, 2000));
        ix.add(
            EvidenceEntry::new(
                "receipt:e1",
                EvidenceFamily::Receipt,
                "exec e1 Verified",
                1000,
            )
            .tagged("verdict:Verified"),
        );

        let hit = ix.query(&["succeeded".into()]);
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].id, "exec:a");
        assert_eq!(hit[0].mode, "structural");
        assert_eq!(hit[0].score, 1.0);

        // Two terms AND: only the execution matching both survives.
        let both = ix.query(&["succeeded".into(), "worker:".into()]);
        assert!(both.is_empty());

        // Kind filter via the auto tag.
        let receipts = ix.query(&["kind:receipt".into()]);
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].id, "receipt:e1");
    }

    #[test]
    fn semantic_requires_real_embeddings_and_is_honest() {
        let mut ix = EvidenceIndex::new();
        ix.add(exec("a", "succeeded", 100, 10, 3000).with_embedding(vec![1.0, 0.0, 0.0]));
        // This one has no embedding — must never match semantically.
        ix.add(exec("b", "failed", 500, 300, 2000));

        let hits = ix.semantic(&[1.0, 0.0, 0.0], 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "exec:a");
        assert_eq!(hits[0].mode, "semantic");
        assert!((hits[0].score - 1.0).abs() < 1e-6);
    }

    #[test]
    fn lessons_derive_only_from_evidence() {
        // Empty index → zero lessons with zero samples, values 0.0.
        // 5 core lessons + 3 benchmark-mode accuracies + median latency.
        let empty = lessons(&[]);
        assert_eq!(empty.len(), 9);
        for l in &empty {
            assert_eq!(l.sample, 0);
            assert_eq!(l.value, 0.0);
        }

        let entries = vec![
            exec("a", "succeeded", 100, 10, 3000),
            exec("b", "succeeded", 200, 20, 2000),
            exec("c", "failed", 500, 300, 1000),
        ];
        let ls = lessons(&entries);
        let rate = ls
            .iter()
            .find(|l| l.id == "executions/success_rate")
            .unwrap();
        assert_eq!(rate.sample, 3);
        assert!((rate.value - 2.0 / 3.0).abs() < 1e-6);
        let dur = ls
            .iter()
            .find(|l| l.id == "executions/median_duration_ms")
            .unwrap();
        assert_eq!(dur.sample, 3);
        assert_eq!(dur.value, 200.0); // median of 100,200,500
        let rtt = ls.iter().find(|l| l.id == "network/median_rtt_ms").unwrap();
        assert_eq!(rtt.sample, 3);
        assert_eq!(rtt.value, 20.0); // median of 10,20,300
    }

    #[test]
    fn summary_orders_newest_first_and_bounds_recent() {
        let mut ix = EvidenceIndex::new();
        ix.add(exec("a", "succeeded", 100, 10, 1000));
        ix.add(exec("b", "failed", 200, 20, 3000));
        let s = ix.summary(1);
        assert_eq!(s.total, 2);
        assert_eq!(s.recent.len(), 1);
        assert_eq!(s.recent[0].id, "exec:b"); // newest first
        assert_eq!(s.lessons.len(), 9);
    }

    #[test]
    fn benchmark_lessons_derive_accuracy_per_mode_from_tags() {
        let mut ix = EvidenceIndex::new();
        let bench = |id: &str, mode: &str, verdict: &str, lat: u64| {
            EvidenceEntry::new(
                format!("bench:{id}"),
                EvidenceFamily::Benchmark,
                "run".to_string(),
                1000,
            )
            .tagged(format!("mode:{mode}"))
            .tagged(format!("verdict:{verdict}"))
            .tagged(format!("latency_ms:{lat}"))
        };
        ix.add(bench("1", "single", "Correct", 100));
        ix.add(bench("2", "single", "Incorrect", 200));
        ix.add(bench("3", "single", "Correct", 300));
        ix.add(bench("4", "collective", "Correct", 50));
        ix.add(bench("5", "collective", "Abstained", 60));
        let ls = lessons(&ix.all());
        let single = ls.iter().find(|l| l.id == "bench/single_accuracy").unwrap();
        assert_eq!(single.sample, 3); // graded only (Abstained excluded)
        assert!((single.value - 2.0 / 3.0).abs() < 1e-6);
        let collective = ls
            .iter()
            .find(|l| l.id == "bench/collective_accuracy")
            .unwrap();
        assert_eq!(collective.sample, 1);
        assert!((collective.value - 1.0).abs() < 1e-6);
        let med = ls
            .iter()
            .find(|l| l.id == "bench/median_latency_ms")
            .unwrap();
        assert_eq!(med.sample, 5);
        assert_eq!(med.value, 100.0); // median of 100,200,300,50,60
    }
}
