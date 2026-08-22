//! Secret redaction for anything the intelligence layer might surface in
//! errors, logs or telemetry.
//!
//! The rule (privacy + security requirements): API keys, bearer tokens and
//! peer-credential material must never survive into a log line. Providers
//! read keys from the environment at call time and never store them; these
//! helpers are the second line of defense for ERROR PATHS, where a provider
//! might embed a URL with a token or an upstream error might echo a header.

/// Patterns redacted by [`redact_secrets`].
pub fn redact_secrets(input: &str) -> String {
    let mut out = input.to_string();
    // Bearer / api-key headers echoed by an upstream error. The search cursor
    // advances past each replacement — without it, the replacement itself
    // still contains the marker ("Bearer [REDACTED]") and the loop would
    // spin forever rewriting the same span.
    for marker in ["Bearer ", "bearer "] {
        let mut search_from = 0;
        while let Some(rel) = out[search_from..].find(marker) {
            let idx = search_from + rel;
            let start = idx + marker.len();
            let end = out[start..]
                .find(char::is_whitespace)
                .map(|i| start + i)
                .unwrap_or(out.len());
            if end > start {
                out = format!("{}[REDACTED]{}", &out[..start], &out[end..]);
                search_from = start + "[REDACTED]".len();
            } else {
                break;
            }
        }
    }
    // Query-string tokens (?key=… / &api_key=…) that some gateways echo.
    for param in ["key=", "api_key=", "apikey="] {
        let mut search_from = 0;
        while let Some(rel) = out[search_from..].find(param) {
            let start = search_from + rel + param.len();
            let end = out[start..]
                .find(['&', ' ', '\n'])
                .map(|i| start + i)
                .unwrap_or(out.len());
            if end > start {
                out = format!("{}[REDACTED]{}", &out[..start], &out[end..]);
                search_from = start + "[REDACTED]".len();
            } else {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_bearer_tokens() {
        let s = "request failed: Authorization: Bearer sk-live-abcdef123 and retry";
        assert_eq!(
            redact_secrets(s),
            "request failed: Authorization: Bearer [REDACTED] and retry"
        );
    }

    #[test]
    fn redacts_query_string_keys() {
        let s = "GET https://api.example.com/v1/models?api_key=sk-secret123&x=1";
        assert!(!redact_secrets(s).contains("sk-secret123"));
        assert!(redact_secrets(s).contains("[REDACTED]"));
    }

    #[test]
    fn leaves_ordinary_text_untouched() {
        let s = "connection refused to 127.0.0.1:8080";
        assert_eq!(redact_secrets(s), s);
    }
}
