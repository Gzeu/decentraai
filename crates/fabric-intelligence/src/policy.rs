//! Provider selection policy — the pure decision of WHICH intelligence
//! source answers a task, given the configured preference and what is
//! actually available. No network calls here: the caller executes whatever
//! this decides.

use serde::{Deserialize, Serialize};
/// How to choose between the local backend (the node's managed llama-server)
/// and an external OpenAI-compatible provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionPolicy {
    /// Local backend first; external only if explicitly allowed as a
    /// fallback AND the local attempt fails or falls below the confidence
    /// threshold. DEFAULT — keeps user content on-node by default (privacy).
    #[default]
    LocalFirst,
    /// External first; local only as fallback.
    ExternalFirst,
    /// Never call an external provider. Air-gapped deployments.
    LocalOnly,
    /// Always external; local only if external is unconfigured.
    ExternalOnly,
    /// Whichever succeeds first, tried in local→external order.
    Fallback,
}

/// Which provider should attempt the task next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderChoice {
    Local,
    External,
    /// Policy + availability leave nothing to try.
    None,
}

/// Pure decision: which provider runs next.
///
/// `external_configured` means an external endpoint exists in config with a
/// resolvable API key environment variable — an unconfigured external can
/// never be selected, whatever the policy says.
///
/// `local_failed` marks that the local attempt already failed (or returned a
/// below-threshold plan), so `LocalFirst` may move on to the external.
pub fn select_provider(
    policy: SelectionPolicy,
    external_configured: bool,
    local_failed: bool,
) -> ProviderChoice {
    use ProviderChoice::{External, Local, None};
    match policy {
        SelectionPolicy::LocalOnly => Local,
        SelectionPolicy::LocalFirst => {
            if !local_failed {
                Local
            } else if external_configured {
                External
            } else {
                None
            }
        }
        SelectionPolicy::Fallback => {
            if !local_failed {
                Local
            } else if external_configured {
                External
            } else {
                // Fallback semantics: still worth one local attempt even when
                // no external exists (caller treats this like LocalOnly).
                Local
            }
        }
        SelectionPolicy::ExternalFirst => {
            if external_configured {
                External
            } else {
                Local
            }
        }
        SelectionPolicy::ExternalOnly => {
            if external_configured {
                External
            } else {
                // External-only with no external configured must FAIL CLOSED:
                // returning Local here would silently leak tasks to the node
                // that the operator explicitly routed away from it.
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_first_stays_local_until_failure() {
        assert_eq!(
            select_provider(SelectionPolicy::LocalFirst, true, false),
            ProviderChoice::Local
        );
        assert_eq!(
            select_provider(SelectionPolicy::LocalFirst, true, true),
            ProviderChoice::External
        );
    }

    #[test]
    fn local_first_without_external_never_leaves_the_node() {
        assert_eq!(
            select_provider(SelectionPolicy::LocalFirst, false, true),
            ProviderChoice::None
        );
    }

    #[test]
    fn local_only_ignores_external_completely() {
        assert_eq!(
            select_provider(SelectionPolicy::LocalOnly, true, true),
            ProviderChoice::Local
        );
    }

    #[test]
    fn external_only_fails_closed_when_unconfigured() {
        assert_eq!(
            select_provider(SelectionPolicy::ExternalOnly, false, false),
            ProviderChoice::None
        );
        assert_eq!(
            select_provider(SelectionPolicy::ExternalOnly, true, false),
            ProviderChoice::External
        );
    }

    #[test]
    fn external_first_prefers_external_but_degrades_to_local() {
        assert_eq!(
            select_provider(SelectionPolicy::ExternalFirst, true, false),
            ProviderChoice::External
        );
        assert_eq!(
            select_provider(SelectionPolicy::ExternalFirst, false, false),
            ProviderChoice::Local
        );
    }

    #[test]
    fn policies_survive_config_roundtrip() {
        for p in [
            SelectionPolicy::LocalFirst,
            SelectionPolicy::ExternalFirst,
            SelectionPolicy::LocalOnly,
            SelectionPolicy::ExternalOnly,
        ] {
            let json = serde_json::to_string(&p).unwrap();
            assert_eq!(serde_json::from_str::<SelectionPolicy>(&json).unwrap(), p);
        }
        assert_eq!(
            serde_json::from_str::<SelectionPolicy>("\"local_only\"").unwrap(),
            SelectionPolicy::LocalOnly
        );
    }
}
