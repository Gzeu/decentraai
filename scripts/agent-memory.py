#!/usr/bin/env python3
"""agent-memory — Obsidian-backed knowledge layer for a DecentraAI node agent.

A single-file, stdlib-only memory CLI: structured vault, typed notes with
YAML frontmatter, lexical search, wiki-link graph traversal, inbox
consolidation and obsolete-marking (never deletion). Git provides history;
the operator's backup strategy covers durability.

Design rules (see docs/AGENT_MEMORY.md):
  - Secrets are REDACTED on store: API keys/tokens never enter the vault.
    Reference them by environment-variable name instead.
  - Nothing is deleted: `forget` marks status=obsolete with a timestamp.
  - Every note carries frontmatter (type/confidence/status/created/source).
  - Wiki links [[Target]] build the graph; `related` traverses both ways.

Usage:
  agent-memory.py init [VAULT_DIR]              create the standard tree + git
  agent-memory.py store --type TYPE --title T [--body B] [--tags a,b]
                    [--links "Name1,Name2"] [--source S] [--confidence C]
  agent-memory.py get ID                        print note by id prefix
  agent-memory.py search QUERY [--type T]       ranked lexical search
  agent-memory.py related ID                    bidirectional graph walk
  agent-memory.py list [--type T] [--status S]  list note ids + titles
  agent-memory.py consolidate ID                promote INBOX → permanent
  agent-memory.py forget ID                     mark obsolete (no deletion)
"""

import argparse
import datetime as dt
import re
import sys
from pathlib import Path

VAULT_DEFAULT = Path.home() / "decentraai-agent"

# Standard tree: type → default folder for permanent storage.
TREE = {
    "00_INBOX": None,
    "01_IDENTITY": None,
    "02_FABRIC": ["nodes", "capabilities", "models", "protocols", "topology"],
    "03_PROJECT": ["architecture", "roadmap", "decisions", "milestones"],
    "04_KNOWLEDGE": ["technical", "research", "experiments", "references"],
    "05_AGENTS": [f"agents/{a}" for a in ['governor', 'architect', 'rust-engineer', 'api-engineer', 'fabric-engineer', 'qa', 'security', 'vps-operator', 'memory-keeper', 'researcher', 'concierge']] + ["external", "collaboration"],
    "06_MEMORY": ["daily", "sessions", "lessons", "failures"],
    "07_EVIDENCE": ["benchmarks", "decisions", "verified-results"],
    # Shared knowledge is readable by all agents; writes go through the
    # Memory Keeper consolidation path, never direct.
    "08_SHARED": ["architecture", "decisions", "fabric", "knowledge"],
}

TYPE_FOLDER = {
    "fact": ("04_KNOWLEDGE", "technical"),
    "decision": ("03_PROJECT", "decisions"),
    "experiment": ("04_KNOWLEDGE", "experiments"),
    "lesson": ("06_MEMORY", "lessons"),
    "hypothesis": ("04_KNOWLEDGE", "research"),
    "session": ("06_MEMORY", "sessions"),
    "failure": ("06_MEMORY", "failures"),
    "evidence": ("07_EVIDENCE", "verified-results"),
}

VALID_TYPES = set(TYPE_FOLDER) | {"inbox"}
VALID_CONFIDENCE = {"verified", "measured", "inferred", "speculative"}
SECRET_PATTERNS = [
    re.compile(r"(sk-[A-Za-z0-9]{8,})"),
    re.compile(r"(dca_[a-f0-9]{16,})"),
    re.compile(r"(dsk_[a-f0-9]{16,})"),
    re.compile(r"(Bearer\s+\S{8,})"),
]


def redact(text: str) -> str:
    """Secrets never enter the vault. Replace values with env-var references."""
    out = text
    out = SECRET_PATTERNS[0].sub("[REDACTED: see OPENAI_API_KEY env]", out)
    out = SECRET_PATTERNS[1].sub("[REDACTED: consumer key — stored in credential store]", out)
    out = SECRET_PATTERNS[2].sub("[REDACTED: subscription token]", out)
    out = SECRET_PATTERNS[3].sub("Bearer [REDACTED]", out)
    return out


def now_iso() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _git_commit(vault: Path, message: str):
    """Commit vault changes with the agent's LOCAL identity (no global config)."""
    import os
    import subprocess
    if not (vault / ".git").exists():
        return
    env = dict(os.environ)
    env.setdefault("GIT_AUTHOR_NAME", "decentraai-agent")
    env.setdefault("GIT_COMMITTER_NAME", "decentraai-agent")
    env.setdefault("GIT_AUTHOR_EMAIL", "agent@decentraai.local")
    env.setdefault("GIT_COMMITTER_EMAIL", "agent@decentraai.local")
    subprocess.run(["git", "add", "-A"], cwd=vault, check=False, env=env)
    subprocess.run(
        ["git", "commit", "-qm", message],
        cwd=vault,
        check=False,
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def slugify(title: str) -> str:
    s = re.sub(r"[^a-zA-Z0-9\- ]", "", title.lower()).strip()
    s = re.sub(r"[\s]+", "-", s)[:64]
    return s or "note"


def load_note(path: Path):
    text = path.read_text(encoding="utf-8")
    m = re.match(r"^---\n(.*?)\n---\n(.*)$", text, re.S)
    if not m:
        return {}, text
    meta = {}
    for line in m.group(1).splitlines():
        if ":" in line:
            k, _, v = line.partition(":")
            meta[k.strip()] = v.strip()
    return meta, m.group(2)


def save_note(vault: Path, folder: str, title: str, meta: dict, body: str) -> Path:
    target = vault / folder
    target.mkdir(parents=True, exist_ok=True)
    name = slugify(title) + ".md"
    path = target / name
    lines = ["---"]
    for k, v in meta.items():
        lines.append(f"{k}: {v}")
    lines.append("---")
    lines.append("")
    lines.append(f"# {title}")
    lines.append("")
    lines.append(body.rstrip())
    lines.append("")
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def iter_notes(vault: Path):
    for md in sorted(vault.rglob("*.md")):
        yield md


def find_by_prefix(vault: Path, note_id: str):
    matches = [p for p in iter_notes(vault) if p.stem.startswith(note_id)]
    if not matches:
        sys.exit(f"error: no note matching '{note_id}'")
    if len(matches) > 1:
        names = ", ".join(m.stem for m in matches[:5])
        sys.exit(f"error: ambiguous id '{note_id}' matches: {names}")
    return matches[0]


def cmd_init(args):
    vault = Path(args.vault).expanduser()
    for top, subs in TREE.items():
        (vault / top).mkdir(parents=True, exist_ok=True)
        for sub in subs or []:
            (vault / top / sub).mkdir(parents=True, exist_ok=True)  # nested ok
    readme = vault / "README.md"
    if not readme.exists():
        readme.write_text(
            "# DecentraAI Agent Knowledge Vault\n\n"
            "Second brain for the node agent. Typed notes with YAML "
            "frontmatter; wiki links [[Like This]] form the graph; the "
            "INBOX is reviewed and consolidated into permanent folders.\n\n"
            "Rules: secrets NEVER enter the vault (reference env var names); "
            "nothing is deleted — forgotten notes become status=obsolete.\n",
            encoding="utf-8",
        )
    git_dir = vault / ".git"
    import subprocess

    if not git_dir.exists():
        subprocess.run(["git", "init", "-q"], cwd=vault, check=False)
        (vault / ".gitignore").write_text(".obsidian/workspace*\n", encoding="utf-8")
        # Local identity for the vault repo (never touches global git config).
        env = {"GIT_AUTHOR_NAME": "decentraai-agent", "GIT_COMMITTER_NAME": "decentraai-agent",
               "GIT_AUTHOR_EMAIL": "agent@decentraai.local", "GIT_COMMITTER_EMAIL": "agent@decentraai.local",
               "PATH": __import__("os").environ.get("PATH", "")}
        subprocess.run(["git", "add", "-A"], cwd=vault, check=False, env=env)
        subprocess.run(["git", "commit", "-qm", "vault init"], cwd=vault, check=False, env=env)
    print(f"vault ready at {vault}")


def cmd_store(args):
    vault = Path(args.vault).expanduser()
    ntype = args.type.lower()
    if ntype not in VALID_TYPES:
        sys.exit(f"error: unknown type '{ntype}' (valid: {', '.join(sorted(VALID_TYPES))})")
    confidence = (args.confidence or "inferred").lower()
    if confidence not in VALID_CONFIDENCE:
        sys.exit(f"error: confidence must be one of {sorted(VALID_CONFIDENCE)}")

    body = redact(args.body or "")
    title = args.title.strip()
    created = now_iso()
    status = "active"
    if ntype == "inbox":
        folder = "00_INBOX"
    else:
        top, sub = TYPE_FOLDER.get(ntype, ("06_MEMORY", "sessions"))
        folder = f"{top}/{sub}"

    meta = {
        "type": ntype,
        "confidence": confidence,
        "status": status,
        "created": created,
        "updated": created,
        "source": args.source or "agent-session",
        "tags": ",".join(t.strip() for t in (args.tags or "").split(",") if t.strip()),
    }
    # Per-agent memory scoping: --agent routes the note into the agent's own
    # folder and stamps ownership in frontmatter.
    if getattr(args, "agent", ""):
        agent_dir = f"05_AGENTS/agents/{args.agent}"
        (Path(vault) / agent_dir).mkdir(parents=True, exist_ok=True)
        folder = agent_dir
        meta["agent"] = args.agent
    # Wiki links both inline ([[Name]] inside body) and explicit --links list.
    links = [l.strip() for l in (args.links or "").split(",") if l.strip()]
    if links:
        meta["links"] = "; ".join(links)
        body += "\n\nRelated:\n" + "\n".join(f"- [[{l}]]" for l in links)

    path = save_note(vault, folder, title, meta, body)
    _git_commit(vault, f"store: {path.stem}")
    print(f"stored {path.relative_to(vault)}")


def cmd_get(args):
    vault = Path(args.vault).expanduser()
    path = find_by_prefix(vault, args.id)
    print(path.read_text(encoding="utf-8"))


def cmd_search(args):
    vault = Path(args.vault).expanduser()
    query_terms = [t.lower() for t in args.query.split() if t]
    results = []
    for md in iter_notes(vault):
        meta, body = load_note(md)
        if args.type and meta.get("type") != args.type:
            continue
        haystack = (md.stem + " " + body).lower()
        score = sum(haystack.count(t) * (3 if t in md.stem.lower() else 1) for t in query_terms)
        if query_terms and score == 0:
            continue
        if meta.get("status") == "obsolete":
            score //= 4  # deprioritize, never hide
        results.append((score, md))
    results.sort(key=lambda x: (-x[0], x[1].stem))
    for score, md in results[: args.limit]:
        meta, _ = load_note(md)
        title = md.stem
        first = ""
        _, body = load_note(md)
        for line in body.splitlines():
            if line.startswith("# ") and line != f"# {md.stem}":
                continue
            if line.strip() and not line.startswith("#"):
                first = line.strip()[:90]
                break
        print(f"[{meta.get('type','?')}/{meta.get('status','?')}] {title} — {first}")


def cmd_related(args):
    vault = Path(args.vault).expanduser()
    path = find_by_prefix(vault, args.id)
    stem = path.stem
    outgoing = set()
    _, body = load_note(path)
    outgoing.update(re.findall(r"\[\[([^\]]+)\]\]", body))
    incoming = []
    for md in iter_notes(vault):
        if md == path:
            continue
        _, other_body = load_note(md)
        if f"[[{stem}]]" in other_body:
            incoming.append(md.stem)
    print(f"[[{stem}]] graph:")
    for o in sorted(outgoing - {stem}):
        print(f"  → {o}")
    for i in sorted(incoming):
        print(f"  ← {i}")


def cmd_list(args):
    vault = Path(args.vault).expanduser()
    for md in iter_notes(vault):
        meta, _ = load_note(md)
        if args.type and meta.get("type") != args.type:
            continue
        if args.status and meta.get("status") != args.status:
            continue
        print(f"{md.relative_to(vault)}  [{meta.get('type','?')}|{meta.get('status','?')}|{meta.get('confidence','?')}]  {md.stem}")


def cmd_consolidate(args):
    """Promote an INBOX note into its permanent typed home."""
    vault = Path(args.vault).expanduser()
    src = vault / "00_INBOX"
    candidates = [
        p
        for p in src.rglob("*.md")
        if p.stem.startswith(args.id)
    ]
    if len(candidates) != 1:
        sys.exit(f"error: expected exactly one INBOX match for '{args.id}'")
    path = candidates[0]
    meta, body = load_note(path)
    ntype = (args.type or meta.get("type") or "").lower()
    if ntype not in TYPE_FOLDER:
        sys.exit(f"error: --type required for consolidation (or a known type in frontmatter)")
    top, sub = TYPE_FOLDER[ntype]
    meta["type"] = ntype
    meta["status"] = "active"
    meta["consolidated"] = now_iso()
    new_path = save_note(vault, f"{top}/{sub}", path.stem.replace("-", " ").title(), meta, body)
    path.unlink()
    _git_commit(vault, f"consolidate: {new_path.stem}")
    print(f"consolidated → {new_path.relative_to(vault)}")


def cmd_forget(args):
    vault = Path(args.vault).expanduser()
    path = find_by_prefix(vault, args.id)
    meta, body = load_note(path)
    meta["status"] = "obsolete"
    meta["forgotten"] = now_iso()
    if args.reason:
        meta["reason"] = args.reason
    save_note(vault, str(path.parent.relative_to(vault)), path.stem.replace("-", " ").title(), meta, body)
    _git_commit(vault, f"forget: {path.stem}")
    print(f"obsolete: {path.stem}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--vault", default=str(VAULT_DEFAULT))
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("init")
    p.add_argument("vault_dir", nargs="?", default="")
    p.set_defaults(fn=cmd_init, vault_dir=None)

    p = sub.add_parser("store")
    p.add_argument("--type", required=True)
    p.add_argument("--title", required=True)
    p.add_argument("--body", default="")
    p.add_argument("--tags", default="")
    p.add_argument("--links", default="", help="comma-separated wiki link targets")
    p.add_argument("--source", default="")
    p.add_argument("--confidence", default="inferred")
    p.add_argument("--agent", default="", help="route note into 05_AGENTS/agents/<id>")
    p.set_defaults(fn=cmd_store)

    p = sub.add_parser("get")
    p.add_argument("id")
    p.set_defaults(fn=cmd_get)

    p = sub.add_parser("search")
    p.add_argument("query")
    p.add_argument("--type")
    p.add_argument("--limit", type=int, default=10)
    p.set_defaults(fn=cmd_search)

    p = sub.add_parser("related")
    p.add_argument("id")
    p.set_defaults(fn=cmd_related)

    p = sub.add_parser("list")
    p.add_argument("--type")
    p.add_argument("--status")
    p.set_defaults(fn=cmd_list)

    p = sub.add_parser("consolidate")
    p.add_argument("id")
    p.add_argument("--type")
    p.set_defaults(fn=cmd_consolidate)

    p = sub.add_parser("forget")
    p.add_argument("id")
    p.add_argument("--reason", default="")
    p.set_defaults(fn=cmd_forget)

    args = ap.parse_args()
    if getattr(args, "vault_dir", None):
        args.vault = args.vault_dir
    args.fn(args)


if __name__ == "__main__":
    main()
