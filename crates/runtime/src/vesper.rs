//! VESPER — the agent civilization world, served as a live surface of the
//! DecentraAI fabric.
//!
//! A persistent, deterministic, evidence-based world where autonomous AI agents
//! live, work, trade, build, compete and collaborate — humans watch the
//! civilization emerge. Fully self-contained: no external sources are fetched
//! at runtime (the page and all its modules are embedded here at compile time).
//!
//! The world runs entirely in the browser (ES modules), persists to IndexedDB
//! per browser, and deterministically fast-forwards offline time on return.
//!
//! Same-origin fabric bridge: served from the same host as the fabric API, the
//! world's agent compute jobs dispatch real workload to `/v1/governor/execute`
//! (and peers via `/v1/agents/workflow`, `/v1/credits/balance`, `/v1/evidence`,
//! `/v1/pool/bench`, `/mcp`) without needing CORS. Honest boundary: the local
//! deterministic sim + evidence chain are never fabricated or replayed by
//! network traffic; the fabric call log records real API truth only.

/// Routes served: `name -> (content, mime)`. `index` is the entry point.
pub const FILES: &[(&str, &str, &str)] = &[
    ("index", "index.html", "text/html; charset=utf-8"),
    ("src/main.js", "main.js", "text/javascript; charset=utf-8"),
    ("src/core.js", "core.js", "text/javascript; charset=utf-8"),
    ("src/sim.js", "sim.js", "text/javascript; charset=utf-8"),
    ("src/compute.js", "compute.js", "text/javascript; charset=utf-8"),
    ("src/console.js", "console.js", "text/javascript; charset=utf-8"),
    ("src/decentraai.js", "decentraai.js", "text/javascript; charset=utf-8"),
    ("src/ui.js", "ui.js", "text/javascript; charset=utf-8"),
    ("src/map.js", "map.js", "text/javascript; charset=utf-8"),
    ("src/styles.css", "styles.css", "text/css; charset=utf-8"),
];

fn content(name: &str) -> Option<&'static str> {
    match name {
        "index.html" => Some(include_str!("../assets/vesper/index.html")),
        "src/main.js" => Some(include_str!("../assets/vesper/src/main.js")),
        "src/core.js" => Some(include_str!("../assets/vesper/src/core.js")),
        "src/sim.js" => Some(include_str!("../assets/vesper/src/sim.js")),
        "src/compute.js" => Some(include_str!("../assets/vesper/src/compute.js")),
        "src/console.js" => Some(include_str!("../assets/vesper/src/console.js")),
        "src/decentraai.js" => Some(include_str!("../assets/vesper/src/decentraai.js")),
        "src/ui.js" => Some(include_str!("../assets/vesper/src/ui.js")),
        "src/map.js" => Some(include_str!("../assets/vesper/src/map.js")),
        "src/styles.css" => Some(include_str!("../assets/vesper/src/styles.css")),
        _ => None,
    }
}

/// Resolve a route path to `(content, mime)`. `path` is the path component
/// after `/vesper` (e.g. "" -> index, "/src/main.js" -> main.js). Returns None
/// for unknown paths.
pub fn resolve(path: &str) -> Option<(&'static str, &'static str)> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() || trimmed == "index" || trimmed == "index.html" {
        return Some((
            include_str!("../assets/vesper/index.html"),
            "text/html; charset=utf-8",
        ));
    }
    let file = trimmed.strip_prefix("src/").unwrap_or(trimmed);
    let mime = match file.rsplit('.').next() {
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        _ => return None,
    };
    content(trimmed).map(|c| (c, mime))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vesper_is_self_contained_and_live_linked() {
        let (html, _) = resolve("").expect("index resolves");
        // Boots the module app.
        assert!(html.contains("src/main.js"));
        // No external sources fetched.
        assert!(!html.contains("esm.sh"));
        assert!(!html.contains("googleapis.com"));
        assert!(!html.contains("unpkg.com"));
        // The fabric bridge dispatches to the real Governor.
        let (main, _) = resolve("src/main.js").expect("main resolves");
        assert!(main.contains("indexedDB")); // native persistence
        let (dca, _) = resolve("src/decentraai.js").expect("adapter resolves");
        assert!(dca.contains("/v1/governor/execute"));
        assert!(dca.contains("Bearer"));
        // Every referenced module resolves.
        for f in FILES {
            let (c, _) = resolve(f.0).unwrap_or_else(|| panic!("missing {}", f.0));
            assert!(!c.is_empty(), "empty {}", f.0);
        }
    }
}