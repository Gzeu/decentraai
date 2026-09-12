//! Node boot loading page — a lightweight splash shown while the engine
//! is still starting, served at `/loading`.
//!
//! Everything is self-contained: inline CSS + JS, no external sources fetched
//! at runtime (works fully offline once served).
//!
//! The page polls ONLY the PUBLIC `/status` snapshot (no secrets, safe without
//! a token) and stages boot as: API online → engine loaded (`model_loaded`)
//! → models registered (`available_models`) → ready. When ready it offers the
//! dashboard at `/ui2` with a short auto-redirect. It never calls proxied
//! inference endpoints, so watching it cannot reset the engine idle clock.

/// The full loading page. Embedded at compile time, served with no-store.
pub const LOADING_HTML: &str = include_str!("../assets/loading.html");

/// Renders the loading page HTML (no-store).
pub fn loading_html() -> String {
    LOADING_HTML.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_html_is_self_contained_and_status_linked() {
        let html = loading_html();
        // Single self-contained document: inline script present.
        assert!(html.contains("<script>"));
        // Stages poll the PUBLIC /status snapshot (no secrets).
        assert!(html.contains("fetch(\"/status\""));
        // No external script/style CDN includes fetched at runtime.
        assert!(!html.contains("esm.sh"));
        assert!(!html.contains("googleapis.com"));
        assert!(!html.contains("unpkg.com"));
        // Boot stage slots are present.
        assert!(html.contains("d-api"));
        assert!(html.contains("d-engine"));
        assert!(html.contains("d-models"));
        assert!(html.contains("d-ready"));
        // Ready state links the live dashboard (never a proxied endpoint).
        assert!(html.contains("\"/ui2\""));
        assert!(!html.contains("/v1/chat/completions"));
        assert!(!html.contains("/v1/models"));
    }
}
