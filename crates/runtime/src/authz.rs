//! Centralized authorization abstraction for the DecentraAI API layer.
//!
//! This module provides a reusable authorization boundary that future Hub,
//! Society, MCP, A2A and payment integrations can safely use.
//!
//! # Design
//!
//! The core types model:
//!
//! ```text
//! Actor → Scope → Resource → Operation
//! ```
//!
//! An [`Actor`] authenticates a request. A [`Resource`] identifies what is
//! being accessed. An [`Operation`] identifies what the caller wants to do.
//! The [`Authorization`] struct checks whether the actor is permitted.
//!
//! # Current scope vocabulary
//!
//! Only scopes that are actually required by current routes are defined.
//! New scopes are added when a new endpoint needs them — not preemptively.

use std::fmt;

// ─── Actor ───────────────────────────────────────────────────────────────────

/// The authenticated caller. Derived from the existing [`Auth`](crate::api::Auth)
/// enum but modeled for authorization decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// No API token configured — single-user open mode.
    Open,
    /// The master admin token — unlimited.
    Master,
    /// A subscription token with tier and role.
    Subscriber {
        name: String,
        tier: u8,
        role: SubscriberRole,
    },
    /// A consumer API key (`dca_…`) with explicit scopes.
    Consumer {
        key_id: String,
        account: String,
        scopes: Vec<String>,
    },
    /// A wallet-backed session (challenge verified, temporary bearer token).
    Wallet {
        wallet_address: String,
        agent_id: String,
    },
}

/// Subscription role (mirrors [`decentraai_tokens::Role`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriberRole {
    Operator,
    Client,
}

impl Actor {
    /// Whether this actor is effectively an administrator (Master or Open).
    pub fn is_admin(&self) -> bool {
        matches!(self, Self::Master | Self::Open)
    }

    /// Whether this actor is an operator or admin.
    pub fn is_operator_or_admin(&self) -> bool {
        matches!(
            self,
            Self::Master
                | Self::Open
                | Self::Subscriber {
                    role: SubscriberRole::Operator,
                    ..
                }
        )
    }
}

// ─── Scope ───────────────────────────────────────────────────────────────────

/// Named scopes that can be granted to consumer API keys.
/// Only scopes actually used by current routes are listed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Full access (wildcard).
    Wildcard,
    /// Embeddings generation.
    Embeddings,
    /// Compute assist (DFCP).
    Compute,
    /// Arena actions.
    Arena,
    /// Execute decision (fabric mutation).
    Execute,
    /// Hub operations (publish task, bid, propose, etc.).
    Hub,
    /// Society operations (trust, reputation, relationships).
    Society,
    /// Agent personal memory (read/write/search).
    Memory,
}

impl Scope {
    /// Parse a scope from a string. Returns `None` for unknown scopes.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "*" => Some(Self::Wildcard),
            "embeddings" => Some(Self::Embeddings),
            "compute" => Some(Self::Compute),
            "arena" => Some(Self::Arena),
            "execute" => Some(Self::Execute),
            "hub" => Some(Self::Hub),
            "society" => Some(Self::Society),
            "memory" => Some(Self::Memory),
            _ => None,
        }
    }

    /// The string representation of this scope.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Wildcard => "*",
            Self::Embeddings => "embeddings",
            Self::Compute => "compute",
            Self::Arena => "arena",
            Self::Execute => "execute",
            Self::Hub => "hub",
            Self::Society => "society",
            Self::Memory => "memory",
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─── Resource ────────────────────────────────────────────────────────────────

/// Resources that can be accessed through the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resource {
    /// Inference (chat/completions/embeddings).
    Inference,
    /// Fabric (execute decision, workers, network).
    Fabric,
    /// Model Hub (search, pull, serve).
    Hub,
    /// Society (trust, reputation, relationships).
    Society,
    /// Collective memory (read/write/search).
    Memory,
    /// Arena (observe, act, request compute).
    Arena,
    /// Evidence (read).
    Evidence,
    /// Economy (quota, compensation).
    Economy,
    /// Credentials (token create/revoke, consumer keys).
    Credentials,
    /// Admin dashboard (read-only views).
    Admin,
    /// MCP tools.
    Mcp,
}

// ─── Operation ───────────────────────────────────────────────────────────────

/// What the caller wants to do with the resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    /// Read-only access.
    Read,
    /// Create or update (mutation).
    Write,
    /// Execute a fabric operation (inference, execution).
    Execute,
    /// Administrative operation (token management, settings).
    Admin,
}

// ─── Authorization ───────────────────────────────────────────────────────────

/// Result of an authorization check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthzResult {
    /// Access granted.
    Allowed,
    /// Access denied with a human-readable reason.
    Denied(DenyReason),
}

/// Why access was denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenyReason {
    /// No credentials presented.
    Unauthenticated,
    /// Actor lacks the required role.
    InsufficientRole {
        required: &'static str,
        actual: String,
    },
    /// Actor lacks a required scope.
    MissingScope {
        required: Scope,
        available: Vec<String>,
    },
    /// Unknown scope (not a valid scope string).
    UnknownScope { scope: String },
    /// Actor type cannot access this resource.
    WrongActorType { actor: String, resource: Resource },
}

impl fmt::Display for DenyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthenticated => write!(f, "authentication required"),
            Self::InsufficientRole { required, actual } => {
                write!(f, "requires {required} role, got {actual}")
            }
            Self::MissingScope {
                required,
                available,
            } => {
                write!(
                    f,
                    "missing required scope '{required}'; available: [{}]",
                    available.join(", ")
                )
            }
            Self::UnknownScope { scope } => write!(f, "unknown scope: '{scope}'"),
            Self::WrongActorType { actor, resource } => {
                write!(f, "actor type '{actor}' cannot access {resource:?}")
            }
        }
    }
}

impl fmt::Display for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inference => write!(f, "inference"),
            Self::Fabric => write!(f, "fabric"),
            Self::Hub => write!(f, "hub"),
            Self::Society => write!(f, "society"),
            Self::Memory => write!(f, "memory"),
            Self::Arena => write!(f, "arena"),
            Self::Evidence => write!(f, "evidence"),
            Self::Economy => write!(f, "economy"),
            Self::Credentials => write!(f, "credentials"),
            Self::Admin => write!(f, "admin"),
            Self::Mcp => write!(f, "mcp"),
        }
    }
}

/// Centralized authorization check.
///
/// Returns [`AuthzResult::Allowed`] if the actor is permitted to perform the
/// operation on the resource, or [`AuthzResult::Denied`] with a reason.
pub fn authorize(actor: &Actor, resource: Resource, operation: Operation) -> AuthzResult {
    match actor {
        // Open mode: single-user, everything allowed.
        Actor::Open => AuthzResult::Allowed,

        // Master: unlimited.
        Actor::Master => AuthzResult::Allowed,

        // Subscriber: role-based.
        Actor::Subscriber { role, .. } => match operation {
            Operation::Admin => {
                if matches!(role, SubscriberRole::Operator) {
                    AuthzResult::Allowed
                } else {
                    AuthzResult::Denied(DenyReason::InsufficientRole {
                        required: "operator",
                        actual: format!("{role:?}"),
                    })
                }
            }
            // Operator can do everything except admin.
            _ => {
                if matches!(role, SubscriberRole::Operator) {
                    AuthzResult::Allowed
                } else {
                    // Client role: only Read and Execute on Inference.
                    match (resource, operation) {
                        (Resource::Inference, Operation::Read | Operation::Execute) => {
                            AuthzResult::Allowed
                        }
                        _ => AuthzResult::Denied(DenyReason::InsufficientRole {
                            required: "operator",
                            actual: format!("{role:?}"),
                        }),
                    }
                }
            }
        },

        // Consumer: scope-based. Admin/credential operations are ALWAYS denied.
        Actor::Consumer { scopes, .. } => {
            // Consumers can never perform admin or credential operations,
            // regardless of their scopes.
            match (resource, operation) {
                (Resource::Credentials, _) | (Resource::Admin, _) => {
                    return AuthzResult::Denied(DenyReason::WrongActorType {
                        actor: "consumer".into(),
                        resource,
                    });
                }
                _ => {}
            }
            let required_scope = required_scope_for(resource, operation);
            match required_scope {
                Some(scope) => check_scope(scope, scopes),
                None => AuthzResult::Allowed,
            }
        }

        // Wallet: verified public identity, but no administrative authority.
        Actor::Wallet { .. } => match operation {
            Operation::Read => match resource {
                Resource::Credentials | Resource::Admin => {
                    AuthzResult::Denied(DenyReason::WrongActorType {
                        actor: "wallet".into(),
                        resource,
                    })
                }
                _ => AuthzResult::Allowed,
            },
            _ => AuthzResult::Denied(DenyReason::WrongActorType {
                actor: "wallet".into(),
                resource,
            }),
        },
    }
}

/// Maps a (resource, operation) pair to the required scope.
/// Returns `None` if no scope is required (e.g., read-only inference for consumers).
fn required_scope_for(resource: Resource, operation: Operation) -> Option<Scope> {
    match (resource, operation) {
        // Inference: read is free, execute requires execute scope, write/admin not applicable.
        (Resource::Inference, Operation::Read) => None,
        (Resource::Inference, Operation::Execute) => Some(Scope::Execute),
        (Resource::Inference, _) => Some(Scope::Execute),
        // Fabric operations: always require execute.
        (Resource::Fabric, _) => Some(Scope::Execute),
        // Hub: read is free, write requires hub scope.
        (Resource::Hub, Operation::Read) => None,
        (Resource::Hub, _) => Some(Scope::Hub),
        // Society: read is free, write requires society scope.
        (Resource::Society, Operation::Read) => None,
        (Resource::Society, _) => Some(Scope::Society),
        // Memory: read is free, write requires memory scope.
        (Resource::Memory, Operation::Read) => None,
        (Resource::Memory, _) => Some(Scope::Memory),
        // Arena: read is free, write/execute requires arena scope.
        (Resource::Arena, Operation::Read) => None,
        (Resource::Arena, _) => Some(Scope::Arena),
        // Evidence: read only, no scope required.
        (Resource::Evidence, Operation::Read) => None,
        (Resource::Evidence, _) => Some(Scope::Execute),
        // Economy: read only for consumers.
        (Resource::Economy, Operation::Read) => None,
        (Resource::Economy, _) => Some(Scope::Execute),
        // Credentials: admin only (never for consumers).
        (Resource::Credentials, _) => Some(Scope::Execute),
        // Admin: admin only (never for consumers).
        (Resource::Admin, _) => Some(Scope::Execute),
        // MCP: depends on the tool.
        (Resource::Mcp, Operation::Read) => None,
        (Resource::Mcp, _) => Some(Scope::Execute),
    }
}

/// Check whether the actor's scopes include the required scope.
fn check_scope(required: Scope, available: &[String]) -> AuthzResult {
    // Wildcard grants everything.
    if available.iter().any(|s| s == "*") {
        return AuthzResult::Allowed;
    }
    // Exact match.
    if available.iter().any(|s| s == required.as_str()) {
        return AuthzResult::Allowed;
    }
    // Compute scope grants specific capability scopes (backwards compat).
    if required != Scope::Compute && available.iter().any(|s| s == "compute") {
        return AuthzResult::Allowed;
    }
    AuthzResult::Denied(DenyReason::MissingScope {
        required,
        available: available.to_vec(),
    })
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_mode_allows_everything() {
        let actor = Actor::Open;
        assert_eq!(
            authorize(&actor, Resource::Fabric, Operation::Execute),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Credentials, Operation::Admin),
            AuthzResult::Allowed
        );
    }

    #[test]
    fn master_allows_everything() {
        let actor = Actor::Master;
        assert_eq!(
            authorize(&actor, Resource::Fabric, Operation::Execute),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Credentials, Operation::Admin),
            AuthzResult::Allowed
        );
    }

    #[test]
    fn operator_can_do_most_things() {
        let actor = Actor::Subscriber {
            name: "op1".into(),
            tier: 0,
            role: SubscriberRole::Operator,
        };
        assert_eq!(
            authorize(&actor, Resource::Fabric, Operation::Execute),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Admin, Operation::Admin),
            AuthzResult::Allowed
        );
    }

    #[test]
    fn client_cannot_admin() {
        let actor = Actor::Subscriber {
            name: "client1".into(),
            tier: 1,
            role: SubscriberRole::Client,
        };
        assert_eq!(
            authorize(&actor, Resource::Credentials, Operation::Admin),
            AuthzResult::Denied(DenyReason::InsufficientRole {
                required: "operator",
                actual: "Client".into(),
            })
        );
    }

    #[test]
    fn client_can_do_inference() {
        let actor = Actor::Subscriber {
            name: "client1".into(),
            tier: 1,
            role: SubscriberRole::Client,
        };
        assert_eq!(
            authorize(&actor, Resource::Inference, Operation::Read),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Inference, Operation::Execute),
            AuthzResult::Allowed
        );
    }

    #[test]
    fn consumer_with_execute_scope() {
        let actor = Actor::Consumer {
            key_id: "dca_test".into(),
            account: "acct1".into(),
            scopes: vec!["execute".into()],
        };
        assert_eq!(
            authorize(&actor, Resource::Inference, Operation::Execute),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Fabric, Operation::Execute),
            AuthzResult::Allowed
        );
    }

    #[test]
    fn consumer_without_execute_scope() {
        let actor = Actor::Consumer {
            key_id: "dca_test".into(),
            account: "acct1".into(),
            scopes: vec!["embeddings".into()],
        };
        assert_eq!(
            authorize(&actor, Resource::Inference, Operation::Execute),
            AuthzResult::Denied(DenyReason::MissingScope {
                required: Scope::Execute,
                available: vec!["embeddings".into()],
            })
        );
    }

    #[test]
    fn consumer_wildcard_scope() {
        let actor = Actor::Consumer {
            key_id: "dca_test".into(),
            account: "acct1".into(),
            scopes: vec!["*".into()],
        };
        assert_eq!(
            authorize(&actor, Resource::Fabric, Operation::Execute),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Arena, Operation::Write),
            AuthzResult::Allowed
        );
    }

    #[test]
    fn consumer_hub_scope() {
        let actor = Actor::Consumer {
            key_id: "dca_test".into(),
            account: "acct1".into(),
            scopes: vec!["hub".into()],
        };
        assert_eq!(
            authorize(&actor, Resource::Hub, Operation::Read),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Hub, Operation::Write),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Fabric, Operation::Execute),
            AuthzResult::Denied(DenyReason::MissingScope {
                required: Scope::Execute,
                available: vec!["hub".into()],
            })
        );
    }

    #[test]
    fn consumer_unknown_scope_rejected() {
        // Unknown scopes should not match anything.
        let actor = Actor::Consumer {
            key_id: "dca_test".into(),
            account: "acct1".into(),
            scopes: vec!["nonexistent".into()],
        };
        assert_eq!(
            authorize(&actor, Resource::Arena, Operation::Write),
            AuthzResult::Denied(DenyReason::MissingScope {
                required: Scope::Arena,
                available: vec!["nonexistent".into()],
            })
        );
    }

    #[test]
    fn scope_parsing() {
        assert_eq!(Scope::parse("*"), Some(Scope::Wildcard));
        assert_eq!(Scope::parse("embeddings"), Some(Scope::Embeddings));
        assert_eq!(Scope::parse("compute"), Some(Scope::Compute));
        assert_eq!(Scope::parse("arena"), Some(Scope::Arena));
        assert_eq!(Scope::parse("execute"), Some(Scope::Execute));
        assert_eq!(Scope::parse("hub"), Some(Scope::Hub));
        assert_eq!(Scope::parse("society"), Some(Scope::Society));
        assert_eq!(Scope::parse("memory"), Some(Scope::Memory));
        assert_eq!(Scope::parse("bogus"), None);
    }

    #[test]
    fn read_ops_free_for_consumer() {
        let actor = Actor::Consumer {
            key_id: "dca_test".into(),
            account: "acct1".into(),
            scopes: vec![],
        };
        // Read operations should be allowed without any scope.
        assert_eq!(
            authorize(&actor, Resource::Hub, Operation::Read),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Society, Operation::Read),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Arena, Operation::Read),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Memory, Operation::Read),
            AuthzResult::Allowed
        );
    }

    #[test]
    fn scope_cannot_escalate_to_admin() {
        // Having an "execute" scope should NOT grant admin access.
        let actor = Actor::Consumer {
            key_id: "dca_test".into(),
            account: "acct1".into(),
            scopes: vec!["execute".into(), "hub".into(), "society".into()],
        };
        // Admin/credential operations are always denied for consumers.
        assert_eq!(
            authorize(&actor, Resource::Credentials, Operation::Admin),
            AuthzResult::Denied(DenyReason::WrongActorType {
                actor: "consumer".into(),
                resource: Resource::Credentials,
            })
        );
        assert_eq!(
            authorize(&actor, Resource::Admin, Operation::Admin),
            AuthzResult::Denied(DenyReason::WrongActorType {
                actor: "consumer".into(),
                resource: Resource::Admin,
            })
        );
    }

    #[test]
    fn compute_scope_grants_arena_and_memory() {
        // Compute scope should grant access to arena and memory (backwards compat).
        let actor = Actor::Consumer {
            key_id: "dca_test".into(),
            account: "acct1".into(),
            scopes: vec!["compute".into()],
        };
        assert_eq!(
            authorize(&actor, Resource::Arena, Operation::Write),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Memory, Operation::Write),
            AuthzResult::Allowed
        );
    }

    #[test]
    fn society_scope_only_for_society() {
        let actor = Actor::Consumer {
            key_id: "dca_test".into(),
            account: "acct1".into(),
            scopes: vec!["society".into()],
        };
        assert_eq!(
            authorize(&actor, Resource::Society, Operation::Write),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Arena, Operation::Write),
            AuthzResult::Denied(DenyReason::MissingScope {
                required: Scope::Arena,
                available: vec!["society".into()],
            })
        );
    }

    #[test]
    fn memory_scope_only_for_memory() {
        let actor = Actor::Consumer {
            key_id: "dca_test".into(),
            account: "acct1".into(),
            scopes: vec!["memory".into()],
        };
        assert_eq!(
            authorize(&actor, Resource::Memory, Operation::Write),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Arena, Operation::Write),
            AuthzResult::Denied(DenyReason::MissingScope {
                required: Scope::Arena,
                available: vec!["memory".into()],
            })
        );
    }

    #[test]
    fn subscriber_client_cannot_write_fabric() {
        let actor = Actor::Subscriber {
            name: "client1".into(),
            tier: 1,
            role: SubscriberRole::Client,
        };
        // Client can only do Inference Read/Execute.
        assert_eq!(
            authorize(&actor, Resource::Inference, Operation::Read),
            AuthzResult::Allowed
        );
        assert_eq!(
            authorize(&actor, Resource::Inference, Operation::Execute),
            AuthzResult::Allowed
        );
        // Client cannot write fabric, hub, society, etc.
        assert_eq!(
            authorize(&actor, Resource::Fabric, Operation::Write),
            AuthzResult::Denied(DenyReason::InsufficientRole {
                required: "operator",
                actual: "Client".into(),
            })
        );
        assert_eq!(
            authorize(&actor, Resource::Hub, Operation::Write),
            AuthzResult::Denied(DenyReason::InsufficientRole {
                required: "operator",
                actual: "Client".into(),
            })
        );
    }

    #[test]
    fn deny_reason_display() {
        // Verify deny reasons produce human-readable messages.
        let reason = DenyReason::MissingScope {
            required: Scope::Execute,
            available: vec!["embeddings".into(), "arena".into()],
        };
        let msg = reason.to_string();
        assert!(msg.contains("execute"));
        assert!(msg.contains("embeddings"));
        assert!(msg.contains("arena"));

        let reason2 = DenyReason::InsufficientRole {
            required: "operator",
            actual: "Client".into(),
        };
        assert!(reason2.to_string().contains("operator"));
    }

    #[test]
    fn actor_is_admin_helpers() {
        assert!(Actor::Open.is_admin());
        assert!(Actor::Master.is_admin());
        assert!(
            !Actor::Subscriber {
                name: "c".into(),
                tier: 0,
                role: SubscriberRole::Client
            }
            .is_admin()
        );
        assert!(
            !Actor::Consumer {
                key_id: "d".into(),
                account: "a".into(),
                scopes: vec![]
            }
            .is_admin()
        );

        assert!(Actor::Open.is_operator_or_admin());
        assert!(Actor::Master.is_operator_or_admin());
        assert!(
            Actor::Subscriber {
                name: "o".into(),
                tier: 0,
                role: SubscriberRole::Operator
            }
            .is_operator_or_admin()
        );
        assert!(
            !Actor::Subscriber {
                name: "c".into(),
                tier: 0,
                role: SubscriberRole::Client
            }
            .is_operator_or_admin()
        );
        assert!(
            !Actor::Consumer {
                key_id: "d".into(),
                account: "a".into(),
                scopes: vec![]
            }
            .is_operator_or_admin()
        );
    }
}
