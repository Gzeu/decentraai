//! Fabric landing — a cinematic, scroll-driven WebGL experience of the
//! DecentraAI compute fabric, served as the landing page.
//!
//! Everything is self-contained: the Three.js renderer, all 16 scenes and the
//! narrative UI are bundled into a single HTML file (embedded at compile time
//! via `include_str!`). There are NO external sources fetched at runtime — the
//! page works fully offline once served.
//!
//! The final beat polls the PUBLIC `/status` snapshot (no secrets, safe
//! without a token) and renders live node state: model, status, CPU, requests.
//! It never calls proxied inference endpoints, so watching it cannot reset the
//! engine idle clock.

/// The full landing page: HTML + inline ES-module bundle (Three.js + scenes +
/// UI + live fabric layer). Embedded at compile time, served with no-store.
pub const LANDING_HTML: &str = include_str!("../assets/landing.html");

/// Renders the landing page HTML (no-store).
pub fn fabric_landing_html() -> String {
    LANDING_HTML.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landing_html_is_self_contained_and_live_linked() {
        let html = fabric_landing_html();
        // Single self-contained document: inline module bundle present.
        assert!(html.contains("<script type=\"module\">"));
        // The final beat polls the PUBLIC /status snapshot (no secrets).
        assert!(html.contains("fetch(\"/status\")"));
        // No external script/style CDN includes fetched at runtime.
        assert!(!html.contains("esm.sh"));
        assert!(!html.contains("googleapis.com"));
        assert!(!html.contains("unpkg.com"));
        // Live panel slots + final CTA to the live dashboard are present.
        assert!(html.contains("l-status"));
        assert!(html.contains("Live dashboard"));
        // The 16 narrative beats are all injected.
        assert!(html.contains("beat-tpl"));
    }
}
