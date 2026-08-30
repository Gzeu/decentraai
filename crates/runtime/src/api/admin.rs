//! Auto-extracted admin module from api/mod.rs.
//! Re-exported via `pub(crate) use admin::*` in mod.rs.

use super::*;

pub(crate) fn registry_models(data_dir: &Path) -> Vec<serde_json::Value> {
    let path = data_dir.join("db/registry.json");
    let Ok(registry) = decentraai_registry::ModelRegistry::load(&path) else {
        return Vec::new();
    };
    registry
        .list_models()
        .into_iter()
        .map(|m| serde_json::json!({"name": m.relative_path, "size_bytes": m.size_bytes}))
        .collect()
}
// P3 - Admin dashboard handlers
pub(crate) const ADMIN_HTML: &str = r##"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>DecentraAI · Admin</title>
<style>
:root{--bg:#05070d;--bg-2:#0a0e16;--panel:#0d121c;--panel-2:#0a0f18;--line:#182234;--line-2:#223048;--text:#e8eef6;--muted:#8fa0b3;--faint:#6f8198;--accent:#22d3ee;--accent-2:#6366f1;--accent-soft:rgba(34,211,238,.1);--ok:#34d399;--warn:#fbbf24;--bad:#f87171;--mono:ui-monospace,"SF Mono",SFMono-Regular,Menlo,Consolas,monospace;--sans:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Inter,Helvetica,Arial,sans-serif;--radius:14px;--radius-sm:9px;--shadow:0 14px 44px rgba(0,0,0,.45)}
*{box-sizing:border-box;margin:0;padding:0}
body{font:14px/1.55 var(--sans);background:var(--bg);color:var(--text);padding:26px;max-width:1100px;margin:0 auto}
.page-head{display:flex;align-items:center;gap:12px;margin-bottom:22px;padding-bottom:16px;border-bottom:1px solid var(--line)}
.brand-mark{width:32px;height:32px;border-radius:9px;background:linear-gradient(135deg,var(--accent),var(--accent-2));display:grid;place-items:center;font-weight:800;color:#04121a;box-shadow:0 0 18px rgba(34,211,238,.35)}
.page-head h1{font-size:19px;font-weight:700;letter-spacing:-.01em}
.crumb{font-size:11px;color:var(--faint);text-transform:uppercase;letter-spacing:.14em}
.grid{display:grid;grid-template-columns:1fr 1fr;gap:14px}
@media(max-width:820px){.grid{grid-template-columns:1fr}}
.card{background:linear-gradient(180deg,var(--panel),var(--panel-2));border:1px solid var(--line);border-radius:var(--radius);padding:16px 18px;box-shadow:var(--shadow);margin-bottom:14px;min-width:0}
.card h2{font-size:11px;font-weight:700;text-transform:uppercase;letter-spacing:.14em;color:var(--faint);margin-bottom:12px;display:flex;align-items:center;gap:8px}
form{display:flex;gap:8px;flex-wrap:wrap;margin-bottom:10px}
input,select{background:var(--bg);border:1px solid var(--line);border-radius:var(--radius-sm);color:var(--text);padding:7px 10px;font:inherit;font-size:12.5px;min-width:130px;flex:1}
input:focus,select:focus{border-color:var(--accent);outline:none}
button{background:var(--accent-soft);border:1px solid var(--line-2);color:var(--text);border-radius:var(--radius-sm);padding:7px 14px;font:inherit;font-size:12.5px;cursor:pointer;transition:border-color .15s,background .15s}
button:hover{border-color:var(--accent);background:var(--accent-soft)}
button.primary{background:var(--accent);color:#04121a;font-weight:700;border-color:transparent}
button.primary:hover{filter:brightness(1.1)}
button.danger{background:rgba(248,113,113,.1);border-color:rgba(248,113,113,.3);color:var(--bad)}
button.danger:hover{border-color:var(--bad)}
table{width:100%;border-collapse:collapse;font-size:12.5px}
th{font-size:10.5px;text-transform:uppercase;letter-spacing:.09em;color:var(--faint);text-align:left;padding:6px 8px;border-bottom:1px solid var(--line);white-space:nowrap}
td{padding:7px 8px;border-bottom:1px solid rgba(28,38,52,.6);vertical-align:top}
tbody tr:hover{background:rgba(255,255,255,.028)}
code,.mono{font-family:var(--mono);font-size:11.5px;color:var(--muted)}
.badge{display:inline-flex;align-items:center;gap:5px;border-radius:999px;padding:2px 9px;font-size:11px;font-weight:600;white-space:nowrap}
.badge.ok{background:rgba(52,211,153,.12);color:var(--ok)}
.badge.warn{background:rgba(251,191,36,.12);color:var(--warn)}
.badge.bad{background:rgba(248,113,113,.12);color:var(--bad)}
.badge.faint{background:rgba(111,129,152,.12);color:var(--faint)}
.off{color:var(--faint);font-size:12.5px}
.small{font-size:11px;color:var(--faint)}
#new,#cnew{margin-top:8px;padding:10px 12px;background:rgba(52,211,153,.08);border:1px solid rgba(52,211,153,.3);border-radius:var(--radius-sm);display:flex;align-items:center;gap:10px;flex-wrap:wrap}
#new code,#cnew code{word-break:break-all;color:var(--ok)}
#status,#cstatus{font-size:12px;margin-top:6px}
#status[data-ok],#cstatus[data-ok]{color:var(--ok)}
#status:not([data-ok]),#cstatus:not([data-ok]){color:var(--bad)}
#audit{list-style:none;padding-left:0}
#audit li{padding:8px 0;border-bottom:1px solid rgba(28,38,52,.6);font-size:12px;color:var(--muted);line-height:1.5}
#audit li code{color:var(--accent)}
.api-url{font-family:var(--mono);font-size:11px;color:var(--faint);margin-top:20px;padding-top:12px;border-top:1px solid var(--line)}
.back{margin-left:auto;text-decoration:none;color:var(--muted);font-size:12.5px;padding:7px 14px;border:1px solid var(--line-2);border-radius:var(--radius-sm);background:var(--accent-soft);transition:border-color .15s,color .15s}
.back:hover{border-color:var(--accent);color:var(--accent)}
</style></head><body>
<div class="page-head"><div class="brand-mark">◈</div><div><div class="crumb">DecentraAI · control plane</div><h1>Admin</h1></div><a class="back" href="/">← Dashboard</a></div>
<div class="grid">
<div class="card"><h2>Create Token</h2><form id="f"><input name="name" placeholder="Token name" required><select name="t"><option value="1">Guest</option><option value="2">Contributor</option><option value="3">Core</option></select><select name="role"><option value="client">Client</option><option value="operator">Operator</option></select><button class="primary" type="submit">Create</button></form><div id="new" style="display:none"><code id="token"></code><button onclick="navigator.clipboard.writeText(document.getElementById('token').textContent)">Copy</button><span class="small">shown once</span></div><p id="status"></p></div>
<div class="card"><h2>Consumer API Keys</h2><form id="cf"><input name="account" placeholder="Owner account" required><input name="ceiling" type="number" min="1" placeholder="Quota ceiling" required><input name="rate" type="number" min="1" placeholder="req/min" required><button class="primary" type="submit">Create</button></form><div id="cnew" style="display:none"><code id="ckey"></code><button onclick="navigator.clipboard.writeText(document.getElementById('ckey').textContent)">Copy</button><span class="small">shown once</span></div><p id="cstatus"></p></div>
</div>
<div class="card"><h2>Tokens</h2><table id="tbl"><thead><tr><th>Name</th><th>Tier</th><th>Role</th><th>Action</th></tr></thead><tbody><tr><td colspan="4" class="off">loading&hellip;</td></tr></tbody></table></div>
<div class="card"><h2>Consumer API Key Registry</h2><table id="ctbl"><thead><tr><th>Key</th><th>Account</th><th>Ceiling</th><th>Rate</th><th>Used</th><th>Account quota</th><th>Status</th><th>Action</th></tr></thead><tbody></tbody></table></div>
<div class="card"><h2>Audit Events</h2><ul id="audit"><li class="off">loading&hellip;</li></ul></div>
<p id="api-url" class="api-url"></p><script>
var f=document.getElementById('f'),status=document.getElementById('status'),tbl=document.querySelector('#tbl tbody'),tokenEl=document.getElementById('token'),newDiv=document.getElementById('new');
var cf=document.getElementById('cf'),cstatus=document.getElementById('cstatus'),ctbl=document.querySelector('#ctbl tbody'),ckeyEl=document.getElementById('ckey'),cnewDiv=document.getElementById('cnew');
var esc=function(s){return String(s).replace(/[&<>"]/g,function(c){return{'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]});};
function tierBadge(t){return '<span class="badge '+(t==3?'ok':t==2?'warn':'faint')+'">T'+t+'</span>';}
function setStatus(el,s,ok){el.textContent=s;el.dataset.ok=ok?'1':'';}
f.addEventListener('submit',async e=>{e.preventDefault();var n=f.name.value,t=parseInt(f.t.value),role=f.role.value;setStatus(status,'Creating...',true);var r=await fetch('/api/admin/token/create',{method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')},body:JSON.stringify({name:n,tier:t,role:role})});var d=await r.json();if(r.ok){tokenEl.textContent=d.token;newDiv.style.display='flex';setStatus(status,'Saved! Copy now.',true);f.reset()}else setStatus(status,d.error&&d.error.message||'error',false);});
cf.addEventListener('submit',async e=>{e.preventDefault();var acct=cf.account.value,ceil=parseInt(cf.ceiling.value),rate=parseInt(cf.rate.value);setStatus(cstatus,'Creating...',true);var r=await fetch('/api/admin/consumer-key/create',{method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')},body:JSON.stringify({account:acct,quota_ceiling:ceil,rate_limit_per_minute:rate})});var d=await r.json();if(r.ok){ckeyEl.textContent=d.token;cnewDiv.style.display='flex';setStatus(cstatus,'Saved! Copy now.',true);cf.reset()}else setStatus(cstatus,d.error&&d.error.message||'error',false);loadConsumer();});
async function load(){var r=await fetch('/api/admin/token/list',{headers:{'Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')}});var d=await r.json();tbl.innerHTML=d.tokens.length?'':('<tr><td colspan="4" class="off">no tokens yet</td></tr>');d.tokens.forEach(t=>{var row=document.createElement('tr');row.innerHTML='<td class="mono">'+esc(t.name)+'</td><td>'+tierBadge(t.tier)+'</td><td>'+esc(t.role||'—')+'</td><td><button class="danger" data-n="'+esc(t.name)+'" onclick="revoke(event)">Revoke</button></td>';tbl.appendChild(row)});loadAudit();loadConsumer();}
async function loadConsumer(){var r=await fetch('/api/admin/consumer-key/list',{headers:{'Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')}});var d=await r.json();ctbl.innerHTML='';if(!(d.keys||[]).length){ctbl.innerHTML='<tr><td colspan="8" class="off">no consumer API keys</td></tr>';return;}d.keys.forEach(k=>{var q=k.account_quota||{},row=document.createElement('tr');row.innerHTML='<td><code>'+esc(k.key_id)+'</code></td><td>'+esc(k.account)+'</td><td>'+k.quota_ceiling+'</td><td>'+k.rate_limit_per_minute+'</td><td>'+k.requests+' ('+k.tokens_generated+' tok)</td><td>'+q.available+'/'+q.consumed+'</td><td>'+(k.revoked?'<span class="badge bad">revoked</span>':'<span class="badge ok">active</span>')+'</td><td>'+(k.revoked?'':'<button class="danger" data-id="'+esc(k.key_id)+'" onclick="revokeConsumer(event)">Revoke</button>')+'</td>';ctbl.appendChild(row)});}
var auditEl=document.getElementById('audit');
async function loadAudit(){var r=await fetch('/api/admin/events',{headers:{'Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')}});var d=await r.json();var evs=d.events||[];auditEl.innerHTML=evs.length?'':('<li class="off">no security events yet</li>');evs.forEach(function(e){var li=document.createElement('li');var d2=new Date((e.timestamp||0)*1000).toLocaleString();li.innerHTML='<code>'+esc(e.event||'')+'</code> <span class="small">'+d2+'</span> <span class="small">'+esc(JSON.stringify(e.details||Object()))+'</span>';auditEl.appendChild(li);});}
(async function(){
  // Fetch the master token from /v1/token (same as the dashboard) and cache
  // it so every /api/admin/* call below authenticates. If it is missing, the
  // API calls surface the real auth error — the page never fabricates a token.
  if(!localStorage.getItem('admin-token')){
    try{var t=await (await fetch('/v1/token')).text();if(t&&t.trim()){localStorage.setItem('admin-token',t.trim());}}catch(e){}
  }
  load();
})();
function revoke(e){var n=e.target.dataset.n;fetch('/api/admin/token/revoke',{method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')},body:JSON.stringify({name:n})}).then(_=>load());}
function revokeConsumer(e){var id=e.target.dataset.id;fetch('/api/admin/consumer-key/revoke',{method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')},body:JSON.stringify({key_id:id})}).then(_=>loadConsumer());}
document.getElementById('api-url').textContent='API: http://127.0.0.1:{PORT}/v1';
</script></html>"##;
pub(crate) fn admin_html(port: u16) -> String {
    // {PORT} is unique to the api-url placeholder; a bare "{}" would match the
    // first object literal in the admin JS (catch(e){}) and corrupt the page.
    ADMIN_HTML.replace("{PORT}", &port.to_string())
}
pub(crate) async fn admin_handler(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    // Serve the admin HTML shell without auth — like the dashboard. The
    // security boundary lives on the /api/admin/* endpoints (master-gated);
    // the page fetches the master token from /v1/token and authenticates each
    // call. Requiring auth here made /admin unreachable from a normal browser
    // (there was no way to attach the header).
    let _ = (headers,);
    Html(admin_html(state.info.api_port)).into_response()
}
pub(crate) async fn admin_token_list_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let tokens = match &state.token_store_path {
        Some(p) => decentraai_tokens::TokenStore::load(p)
            .map(|s| s.list())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    // Per-token live usage (requests + generated tokens) collected by
    // note_token_usage during routed inference; absent for tokens that never
    // served a request or when inference bypassed the coordinator.
    let usage = state.token_usage.lock().unwrap().clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let body = serde_json::json!({"tokens": tokens.iter().map(|t| {
        let u = usage.get(&t.name).copied().unwrap_or((0, 0, 0));
        serde_json::json!({
            "name": &t.name,
            "tier": t.tier,
            "role": t.role.name(),
            "created_at": t.created_at,
            "revoked": t.revoked,
            "expires_at": t.expires_at,
            "expired": t.expires_at.is_some_and(|ts| ts <= now),
            "requests": u.0,
            "tokens_generated": u.1,
            "last_used_at": if u.2 > 0 { Some(u.2) } else { None },
        })
    }).collect::<Vec<_>>()});
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}
pub(crate) async fn admin_token_create_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let name = match req.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => return forbidden("missing name"),
    };
    let tier = match req
        .get("tier")
        .and_then(|v| v.as_u64())
        .and_then(|n| u8::try_from(n).ok())
    {
        Some(t) if (1..=3).contains(&t) => t,
        _ => return forbidden("tier 1-3"),
    };
    let role = match req
        .get("role")
        .and_then(|v| v.as_str())
        .and_then(|s| decentraai_tokens::Role::parse(s).ok())
    {
        Some(r) => r,
        None => decentraai_tokens::Role::DEFAULT,
    };
    // Optional unix-seconds expiry (Part 12/22 developer tokens): an expired
    // token stops authenticating even though its record stays listed.
    let expires_at = req.get("expires_at").and_then(|v| v.as_u64());
    let plaintext = match &state.token_store_path {
        Some(p) => {
            let mut s = match decentraai_tokens::TokenStore::load(p) {
                Ok(s) => s,
                Err(_) => return forbidden("load failed"),
            };
            match s.create_with_role(&name, decentraai_tokens::Tier(tier), expires_at, role) {
                Ok(t) => {
                    let a = state.info.repo_root.join("logs/audit.jsonl");
                    let _ = decentraai_audit::record(
                        a.parent().unwrap_or(&state.info.repo_root),
                        "token_created",
                        serde_json::json!({"name": &name, "tier": tier, "role": role.name(), "expires_at": expires_at}),
                    );
                    Some(t)
                }
                Err(_) => return forbidden("name taken"),
            }
        }
        None => return forbidden("no store"),
    };
    let body = serde_json::json!({"token": plaintext, "name": name, "tier": tier, "role": role.name(), "expires_at": expires_at});
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}
pub(crate) async fn admin_token_revoke_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let name = match req.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => return forbidden("missing name"),
    };
    let success = match &state.token_store_path {
        Some(p) => {
            let mut s = match decentraai_tokens::TokenStore::load(p) {
                Ok(s) => s,
                Err(_) => return forbidden("load failed"),
            };
            match s.revoke(&name) {
                Ok(()) => true,
                Err(_) => return forbidden("no such token"),
            }
        }
        None => return forbidden("no store"),
    };
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "token_revoked",
        serde_json::json!({"name": name}),
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({"success": success}).to_string(),
    )
        .into_response()
}
/// Q2 — Create a consumer API key (`dca_…`) for an account, master-gated.
/// Shows the plaintext secret exactly once; only its hash + display prefix
/// are stored. The key carries a per-request quota ceiling, a per-key rate
pub(crate) async fn admin_consumer_key_create_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(path) = &state.consumer_keys_path else {
        return forbidden("consumer keys are not enabled (no shared ledger)");
    };
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let account = match req.get("account").and_then(|v| v.as_str()) {
        Some(a) if !a.trim().is_empty() => a.trim().to_string(),
        _ => return forbidden("missing account"),
    };
    let quota_ceiling = req
        .get("quota_ceiling")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let rate_limit_per_minute = req
        .get("rate_limit_per_minute")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(u32::MAX as u64) as u32;
    let scopes = req
        .get("scopes")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if quota_ceiling == 0 {
        return forbidden("quota_ceiling must be > 0");
    }
    if rate_limit_per_minute == 0 {
        return forbidden("rate_limit_per_minute must be > 0");
    }
    let mut store = match decentraai_tokens::ConsumerKeyStore::load(path) {
        Ok(s) => s,
        Err(_) => return forbidden("consumer key store unreadable"),
    };
    let plaintext = match store.create(
        &account,
        quota_ceiling,
        rate_limit_per_minute,
        scopes.clone(),
    ) {
        Ok(p) => p,
        Err(e) => return forbidden(&e.to_string()),
    };
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "consumer_key_created",
        serde_json::json!({
            "account": account,
            "key_prefix": decentraai_tokens::key_prefix(&plaintext),
            "quota_ceiling": quota_ceiling,
            "rate_limit_per_minute": rate_limit_per_minute,
            "scopes": scopes,
        }),
    );
    let key_id = store
        .lookup(&plaintext)
        .map(|r| r.key_id.clone())
        .unwrap_or_default();
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "token": plaintext,
            "key_id": key_id,
            "key_prefix": decentraai_tokens::key_prefix(&plaintext),
            "account": account,
            "quota_ceiling": quota_ceiling,
            "rate_limit_per_minute": rate_limit_per_minute,
            "note": "shown once; only its hash and prefix are stored",
        })
        .to_string(),
    )
        .into_response()
}
pub(crate) async fn admin_consumer_key_revoke_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(path) = &state.consumer_keys_path else {
        return forbidden("consumer keys are not enabled");
    };
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let key_id = match req.get("key_id").and_then(|v| v.as_str()) {
        Some(k) if !k.is_empty() => k.to_string(),
        _ => return forbidden("missing key_id"),
    };
    let mut store = match decentraai_tokens::ConsumerKeyStore::load(path) {
        Ok(s) => s,
        Err(_) => return forbidden("consumer key store unreadable"),
    };
    if store.revoke(&key_id).is_err() {
        return forbidden("no active consumer key with that id");
    }
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "consumer_key_revoked",
        serde_json::json!({"key_id": key_id}),
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({"success": true, "key_id": key_id}).to_string(),
    )
        .into_response()
}
pub(crate) async fn admin_consumer_key_list_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let keys = match &state.consumer_keys_path {
        Some(p) => decentraai_tokens::ConsumerKeyStore::load(p)
            .map(|s| s.list())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let usage = state.consumer_usage.lock().unwrap().clone();
    let ledger = state.quota_ledger.clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let body = serde_json::json!({"keys": keys.iter().map(|k| {
        let u = usage.get(&k.key_id).copied().unwrap_or((0, 0, 0));
        // Live account balance for the key's owner (authoritative ledger).
        let (available, reserved, consumed) = ledger.as_ref().map(|l| {
            let l = l.lock().unwrap();
            let acc = l.account(&k.owner_account);
            (
                acc.map(|a| a.available).unwrap_or(0),
                acc.map(|a| a.reserved).unwrap_or(0),
                acc.map(|a| a.consumed).unwrap_or(0),
            )
        }).unwrap_or((0, 0, 0));
        serde_json::json!({
            "key_id": &k.key_id,
            "prefix": &k.prefix,
            "account": &k.owner_account,
            "created_at": k.created_at,
            "revoked": k.revoked,
            "quota_ceiling": k.quota_ceiling,
            "rate_limit_per_minute": k.rate_limit_per_minute,
            "scopes": &k.scopes,
            "requests": u.0,
            "tokens_generated": u.1,
            "last_used_at": if u.2 > 0 { Some(u.2) } else { None },
            "account_quota": { "available": available, "reserved": reserved, "consumed": consumed },
            "expired_or_revoked": k.revoked,
            "age_secs": now.saturating_sub(k.created_at),
        })
    }).collect::<Vec<_>>()});
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}
/// Model Hub search (Part 16/22): `GET /api/admin/hub/search?query=…&limit=…`
/// queries HuggingFace for GGUF models. Master-gated; the dashboard Model Hub
/// card calls this to let operators discover models to pull, on-device.
pub(crate) async fn admin_hub_search_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let q = query.get("query").cloned().unwrap_or_default();
    if q.trim().is_empty() {
        return forbidden("missing query");
    }
    let limit = query
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8)
        .min(30);
    // Optional capability filter: keep only models whose real metadata supports
    // the requested capability (e.g. `capability=ocr`). Unsupported or invalid
    // capability values yield an empty filtered set, never a false positive.
    let capability = query
        .get("capability")
        .and_then(|v| v.parse::<decentraai_hub::CapabilityKind>().ok());
    let catalog = decentraai_hub::HubCatalog::new();
    match catalog.search(&q, limit).await {
        Ok(models) => {
            let body = hub_search_body(&q, &models, capability);
            (
                [(header::CONTENT_TYPE, "application/json")],
                body.to_string(),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({"error": {"message": e.to_string(), "type": "hub_error"}})
                .to_string(),
        )
            .into_response(),
    }
}
pub(crate) fn hub_search_body(
    query: &str,
    models: &[decentraai_hub::HubModel],
    capability: Option<decentraai_hub::CapabilityKind>,
) -> serde_json::Value {
    let req = capability.map(|cap| {
        vec![decentraai_hub::CapabilityRequirement {
            capability: cap,
            evidence: decentraai_hub::EvidenceLevel::Any,
        }]
    });

    let filtered: Vec<&decentraai_hub::HubModel> = models
        .iter()
        .filter(|m| match &req {
            Some(r) => {
                // Classify this model from its own metadata and check the
                // required capability. A model that does not claim it is
                // dropped (UNKNOWN is not satisfied, never fabricated).
                let caps = decentraai_hub::capability::classify(
                    m.pipeline_tag.as_ref().map(|t| t.as_str()),
                    &m.tags,
                    &m.id,
                );
                decentraai_hub::satisfies_any(&caps, r)
            }
            None => true,
        })
        .collect();

    serde_json::json!({
        "query": query,
        "capability_filter": capability.map(|c| c.label()),
        "total": models.len(),
        "matched": filtered.len(),
        "models": filtered.iter().map(|m| serde_json::json!({
            "id": m.id,
            "pipeline_tag": m.pipeline_tag.as_ref().map(|t| t.as_str()),
            "tags": m.tags,
            "downloads": m.downloads,
        })).collect::<Vec<_>>(),
    })
}
pub(crate) fn refresh_registry_after_pull(
    models_dir: &std::path::Path,
    registry_path: &std::path::Path,
) -> Result<usize> {
    let mut registry = if registry_path.exists() {
        match decentraai_registry::ModelRegistry::load(registry_path) {
            Ok(r) => r,
            Err(_) => decentraai_registry::ModelRegistry::new(models_dir.to_path_buf())?,
        }
    } else {
        decentraai_registry::ModelRegistry::new(models_dir.to_path_buf())?
    };
    let count = registry.scan_directory(models_dir)?;
    registry.save(registry_path)?;
    Ok(count)
}
pub(crate) fn capability_records_from_hub(
    caps: &decentraai_hub::ModelCapabilities,
) -> Vec<decentraai_registry::CapabilityClaimRecord> {
    caps.claims
        .iter()
        .map(|c| decentraai_registry::CapabilityClaimRecord {
            capability: serde_json::to_string(&c.capability)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string(),
            provenance: serde_json::to_string(&c.provenance)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string(),
        })
        .collect()
}
/// Compute `file`'s path relative to `base` by canonicalizing both and
/// stripping the prefix. Returns `None` when the file lies outside `base` or
pub(crate) fn relative_path_of(base: &Path, file: &Path) -> Option<String> {
    let base = std::fs::canonicalize(base).ok()?;
    let file = std::fs::canonicalize(file).ok()?;
    let rel = file.strip_prefix(&base).ok()?;
    Some(rel.to_string_lossy().to_string())
}
/// Find a model's persisted capability claims in a registry by its file name.
/// The registry `relative_path` is a path under models/ whose final component
pub(crate) fn claims_for_file_name(
    registry: &decentraai_registry::ModelRegistry,
    file_name: &str,
) -> Vec<decentraai_registry::CapabilityClaimRecord> {
    registry
        .models
        .values()
        .find(|r| r.relative_path.ends_with(file_name))
        .map(|r| r.capability_claims.clone())
        .unwrap_or_default()
}
pub(crate) fn registry_variants_for_model(
    reg: &decentraai_registry::ModelRegistry,
    model: &str,
) -> Vec<(String, u64)> {
    let model_lower = model.to_lowercase();
    let mut variants: Vec<(String, u64)> = reg
        .models
        .values()
        .filter(|r| {
            let f = r.relative_path.to_lowercase();
            f.contains(&model_lower)
                || model_lower.contains(&f)
                || f.ends_with(&model_lower)
                || model_lower.ends_with(&f)
        })
        .map(|r| (r.relative_path.clone(), r.size_bytes))
        .collect();
    variants.sort_by(|a, b| a.0.cmp(&b.0));
    variants
}
/// Persist the Hub's authoritative capability claims for a freshly pulled
/// model into the local registry. Best-effort by design: a Hub/registry/IO
/// failure surfaces as an error the caller turns into a warning — capabilities
/// are a projection, never a gate on the pull. Returns the number of claims
pub(crate) async fn persist_capability_claims_after_pull(
    models_dir: &Path,
    registry_path: &Path,
    download_path: &Path,
    repo: &str,
) -> Result<usize> {
    let catalog = decentraai_hub::HubCatalog::new();
    let detail = catalog.model_detail(repo).await?;
    let caps = detail.capabilities();
    let claims = capability_records_from_hub(&caps);
    if claims.is_empty() {
        return Ok(0);
    }
    // Map the pulled file to its registry relative path under models_dir.
    let Some(relative_path) = relative_path_of(models_dir, download_path) else {
        anyhow::bail!(
            "could not map pulled file {} to a path under {}",
            download_path.display(),
            models_dir.display()
        );
    };
    let persisted = claims.len();
    let mut registry = decentraai_registry::ModelRegistry::load(registry_path)?;
    match registry.set_capability_claims(&relative_path, claims)? {
        true => {
            registry.save(registry_path)?;
            Ok(persisted)
        }
        false => {
            anyhow::bail!("pulled model not present in registry: {relative_path}");
        }
    }
}
pub(crate) async fn admin_hub_pull_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let reference = match req.get("reference").and_then(|v| v.as_str()) {
        Some(r) if !r.is_empty() => r.to_string(),
        _ => return forbidden("missing reference (hf:org/repo[:file])"),
    };
    // Optional explicit file variant (Issue #26 §22): when present it is
    // appended to the reference so the verified downloader fetches exactly
    // that GGUF instead of the auto-selected largest one.
    let file = req.get("file").and_then(|v| v.as_str()).map(str::to_string);
    let mut hf_ref = match decentraai_hub::HfRef::parse(&reference) {
        Ok(r) => r,
        Err(e) => return forbidden(&format!("bad reference: {e}")),
    };
    if let Some(f) = file {
        if !f.to_lowercase().ends_with(".gguf") {
            return forbidden("file must end with .gguf");
        }
        hf_ref.file = Some(f);
    }
    let models_dir = state.info.repo_root.join("models");
    if let Err(e) = std::fs::create_dir_all(&models_dir) {
        return forbidden(&format!("creating models dir: {e}"));
    }
    // Register the pull so the dashboard can show real byte progress. The
    // `total` is updated once the catalog reports the LFS size (the callback
    // receives bytes downloaded; total stays 0 until we know the file size).
    let repo_key = hf_ref.repo.clone();
    {
        let mut pulls = state.hub_pulls.lock().unwrap();
        pulls.insert(repo_key.clone(), (0, 0));
    }
    let pulls = state.hub_pulls.clone();
    let progress_key = repo_key.clone();
    let progress = Box::new(move |bytes: u64| {
        if let Ok(mut pulls) = pulls.lock() {
            if let Some(e) = pulls.get_mut(&progress_key) {
                e.0 = bytes;
            }
        }
    });
    let download =
        match decentraai_hub::download_model_with_progress(&hf_ref, &models_dir, Some(progress))
            .await
        {
            Ok(d) => d,
            Err(e) => {
                state.hub_pulls.lock().unwrap().remove(&repo_key);
                let body =
                    serde_json::json!({"error": {"message": e.to_string(), "type": "hub_error"}});
                return (
                    StatusCode::BAD_GATEWAY,
                    [(header::CONTENT_TYPE, "application/json")],
                    body.to_string(),
                )
                    .into_response();
            }
        };
    // Pull completed: remove from the in-flight registry (dashboard stops
    // polling; the final result below is authoritative).
    state.hub_pulls.lock().unwrap().remove(&repo_key);
    // Refresh the local registry so the new model is immediately usable.
    let registry_path = state.info.repo_root.join("db/registry.json");
    if let Some(cm) = &state.compute {
        cm.set_registry_path(registry_path.clone());
    }
    let _count = match refresh_registry_after_pull(&models_dir, &registry_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("registry refresh after pull failed: {e:#}");
            0
        }
    };
    // Persist the Hub's authoritative capability claims for the pulled model
    // into the local registry. Best-effort: a Hub/registry/IO failure only
    // degrades to a warning — capability persistence must never break a pull.
    let persisted = match persist_capability_claims_after_pull(
        &models_dir,
        &registry_path,
        &download.path,
        hf_ref.repo.as_str(),
    )
    .await
    {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("persisting capability claims after pull failed: {e:#}");
            0
        }
    };
    if persisted > 0 {
        tracing::info!(
            claims = persisted,
            file = %download.path.display(),
            "persisted hub capability claims for pulled model"
        );
    }
    // Issue #26 §25: make the fabric see the new model without a restart.
    // Rebuild `available_models` from the refreshed registry and re-advertise
    // through the compute manager; the periodic broadcaster picks up the new
    // advertisement on its next heartbeat.
    if let Some(cm) = &state.compute {
        let ctx = cm
            .last_local_advertisement_sync()
            .and_then(|a| a.capability.served_models.first().map(|m| m.context_tokens))
            .unwrap_or(0);
        match cm.refresh_local_models(&registry_path, ctx).await {
            Ok(adv) => {
                tracing::info!(
                    on_disk = adv.capability.available_models.len(),
                    "re-advertised local model set after pull"
                );
            }
            Err(e) => tracing::warn!("refresh_local_models after pull failed: {e:#}"),
        }
    }
    decentraai_audit::record_best_effort(
        &state.info.repo_root.join("logs"),
        "model_pulled",
        serde_json::json!({
            "reference": reference,
            "path": download.path.display().to_string(),
            "bytes": download.bytes,
            "sha256": download.sha256,
        }),
    );
    let body = serde_json::json!({
        "reference": reference,
        "file": hf_ref.file,
        "path": download.path.display().to_string(),
        "bytes": download.bytes,
        "sha256": download.sha256,
    });
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}
/// Live Model Hub pull progress: `GET /api/admin/hub/pull/status` returns the
/// in-flight pulls (repo -> bytes downloaded + total when known). Empty when
/// no pull is running. Master-gated. The dashboard polls this while a pull is
pub(crate) async fn admin_hub_pull_status_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let pulls = state.hub_pulls.lock().unwrap().clone();
    let body: Vec<serde_json::Value> = pulls
        .into_iter()
        .map(|(repo, (bytes, total))| {
            serde_json::json!({
                "repo": repo,
                "bytes_downloaded": bytes,
                "total_bytes": if total > 0 { serde_json::json!(total) } else { serde_json::Value::Null },
                "done": false,
            })
        })
        .collect();
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "pulls": body }).to_string(),
    )
        .into_response()
}
/// Model Hub detail (Issue #26 §7–§8, §22, §31): `GET
/// /api/admin/hub/model/{repo}` returns the enriched model card —
/// real Hub metadata (description, license, context, params), the honest
/// capability taxonomy with provenance, and every GGUF file variant with
/// size + SHA-256 — plus the live fabric view of which nodes can run it.
pub(crate) async fn admin_hub_model_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
    AxumPath(repo): AxumPath<String>,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    // Optional capability fit query: `?requires=ocr` asks the model card for
    // an honest provenance-aware verdict of whether this model can do OCR.
    let requires = query
        .get("requires")
        .and_then(|v| v.parse::<decentraai_hub::CapabilityKind>().ok());
    let catalog = decentraai_hub::HubCatalog::new();
    let detail = match catalog.model_detail(&repo).await {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"error": {"message": e.to_string(), "type": "hub_error"}})
                    .to_string(),
            )
                .into_response();
        }
    };
    let files = match catalog.list_gguf_files(&repo).await {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"error": {"message": e.to_string(), "type": "hub_error"}})
                    .to_string(),
            )
                .into_response();
        }
    };
    let caps = detail.capabilities();
    let body = hub_model_body(&detail, &files, &caps, &state, &repo, requires).await;
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}
/// Model Hub comparison (Model Comparison feature): `GET
/// /api/admin/hub/compare?repos=org/repo1,org/repo2` compares multiple models
/// side-by-side with honest metadata, capabilities, variants, resource fit,
pub(crate) async fn admin_hub_compare_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    // Optional capability fit query: `?requires=ocr` asks each compared
    // model for an honest provenance-aware verdict of whether it can do OCR.
    let requires = params
        .get("requires")
        .and_then(|v| v.parse::<decentraai_hub::CapabilityKind>().ok());
    let repos_str = params.get("repos").map(|s| s.as_str()).unwrap_or("");
    if repos_str.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({"error": {"message": "missing 'repos' query parameter", "type": "validation_error"}})
                .to_string(),
        )
            .into_response();
    }

    let repos: Vec<&str> = repos_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if repos.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({"error": {"message": "no valid repositories provided", "type": "validation_error"}})
                .to_string(),
        )
            .into_response();
    }

    let catalog = decentraai_hub::HubCatalog::new();
    let mut compared_models = Vec::new();

    for repo in repos {
        let detail = match catalog.model_detail(repo).await {
            Ok(d) => d,
            Err(e) => {
                compared_models.push(serde_json::json!({
                    "id": repo,
                    "error": e.to_string(),
                }));
                continue;
            }
        };
        let files = match catalog.list_gguf_files(repo).await {
            Ok(f) => f,
            Err(e) => {
                compared_models.push(serde_json::json!({
                    "id": repo,
                    "error": e.to_string(),
                }));
                continue;
            }
        };
        let caps = detail.capabilities();
        let model_json =
            hub_compare_model_body(&detail, &files, &caps, &state, repo, requires).await;
        compared_models.push(model_json);
    }

    let body = serde_json::json!({
        "models": compared_models,
        // What capability the comparison was asked about, so the UI can label
        // the fit verdicts. Null when no `requires` was supplied.
        "requires": requires.map(|cap| cap.label()),
    });
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}
pub(crate) async fn hub_compare_model_body(
    detail: &decentraai_hub::HubModelDetail,
    files: &[decentraai_hub::HubModelFile],
    caps: &decentraai_hub::ModelCapabilities,
    state: &ApiState,
    repo: &str,
    requires: Option<decentraai_hub::CapabilityKind>,
) -> serde_json::Value {
    let mut fabric_nodes = Vec::new();
    if let Some(cm) = &state.compute {
        let workers = cm.workers().await;
        for w in workers {
            let served = w
                .capability
                .served_models
                .iter()
                .any(|m| m.file_name == repo);
            let available = w
                .capability
                .available_models
                .iter()
                .any(|m| m.file_name == repo);
            if served || available {
                fabric_nodes.push(serde_json::json!({
                    "node_id": w.node_id,
                    "node_name": w.node_name,
                    "peer_id": w.peer_id.to_string(),
                    "status": format!("{:?}", w.availability.status),
                    "served": served,
                    "available": available,
                    "trusted": cm.is_trusted(&w.peer_id).await,
                }));
            }
        }
    }
    let local_snapshot = decentraai_system_probe::SystemSnapshot::collect();
    let local_avail_ram_mb = local_snapshot.available_memory_bytes / (1024 * 1024);
    let local_vram_mb = match decentraai_system_probe::probe_gpu() {
        decentraai_system_probe::GpuProbeStatus::Nvidia(gpu) => Some(gpu.free_vram_mib),
        _ => None,
    };
    let has_gpu = local_vram_mb.is_some();

    let mut fabric_variants = Vec::new();
    for f in files {
        let size = f.size.unwrap_or(0);
        let est_ram_mb = decentraai_compute::ServedModel::estimate_ram_mb(size);
        let est_vram_mb = (size * 105 / 100) / (1024 * 1024);
        let mut can_run_workers = Vec::new();
        let mut trusted_worker_count = 0usize;
        if let Some(cm) = &state.compute {
            for w in cm.workers().await {
                let w_ram = w.availability.available_ram_mb;
                let w_vram = w.availability.available_vram_mb.unwrap_or(0);
                if w_ram >= est_ram_mb || w_vram >= est_vram_mb {
                    can_run_workers.push(serde_json::json!({
                        "node_id": w.node_id,
                        "node_name": w.node_name,
                        "peer_id": w.peer_id.to_string(),
                        "ram_ok": w_ram >= est_ram_mb,
                        "vram_ok": w_vram >= est_vram_mb,
                    }));
                    if cm.is_trusted(&w.peer_id).await {
                        trusted_worker_count += 1;
                    }
                }
            }
        }
        let model_available_on_fabric = !fabric_nodes.is_empty();

        // Pure decision: keeps the honesty invariants (separate RAM/VRAM
        // estimates, trusted-only worker credit, classification) testable
        // without I/O.
        let fit = resource_fit(
            local_avail_ram_mb,
            local_vram_mb,
            est_ram_mb,
            est_vram_mb,
            trusted_worker_count,
        );
        let ram_sufficient = fit.ram_sufficient;
        let vram_sufficient = fit.vram_sufficient;
        let local_fit = fit.local_fit;
        let trusted_worker_can_run = fit.trusted_worker_can_run;

        let mut reasons = Vec::new();
        reasons.push(serde_json::json!({
            "check": "ram_sufficient",
            "pass": ram_sufficient,
            "provenance": "ESTIMATED",
            "reason": format!("Local available RAM ({} MB) vs estimated requirement ({} MB)", local_avail_ram_mb, est_ram_mb)
        }));
        if has_gpu {
            reasons.push(serde_json::json!({
                "check": "vram_sufficient",
                "pass": vram_sufficient,
                "provenance": "ESTIMATED",
                "reason": format!("Local free VRAM ({} MB) vs estimated requirement ({} MB)", local_vram_mb.unwrap_or(0), est_vram_mb)
            }));
        } else {
            reasons.push(serde_json::json!({
                "check": "vram_sufficient",
                "pass": false,
                "provenance": "MEASURED",
                "reason": "No discrete GPU detected on local node (CPU-only execution)"
            }));
        }
        // Honest pass: only *trusted* workers count toward "compatible worker
        // found on fabric" — an untrusted worker's advertised capacity is not a
        // resource this operator can actually use yet.
        reasons.push(serde_json::json!({
            "check": "trusted_worker_available",
            "pass": trusted_worker_can_run,
            "provenance": "VERIFIED",
            "reason": format!("{} compatible worker(s) found on fabric ({} trusted)", can_run_workers.len(), trusted_worker_count)
        }));
        reasons.push(serde_json::json!({
            "check": "model_available",
            "pass": model_available_on_fabric || local_fit,
            "provenance": "VERIFIED",
            "reason": if model_available_on_fabric { "Model reported on fabric nodes" } else { "Model not yet present on fabric" }
        }));
        // The bundled engine is llama-server and serves GGUF by construction.
        // That is an ESTIMATED capability derived from the fixed engine
        // choice, not a per-model runtime verification — never VERIFIED.
        reasons.push(serde_json::json!({
            "check": "compatible_engine",
            "pass": true,
            "provenance": "ESTIMATED",
            "reason": "llama-server (bundled engine) serves GGUF models"
        }));

        let fit_classification = fit.classification;

        fabric_variants.push(serde_json::json!({
            "file": f.path,
            "size_bytes": f.size,
            "sha256": f.lfs.as_ref().map(|l| l.oid.clone()),
            "est_ram_mb": est_ram_mb,
            "local_fit": local_fit,
            "fabric_fit_nodes": can_run_workers,
            "fit_classification": fit_classification,
            "fit_reasons": reasons,
        }));
    }

    serde_json::json!({
        "id": repo,
        "metadata": {
            "pipeline_tag": detail.pipeline_tag.as_ref().map(|t| t.as_str()),
            "tags": detail.tags,
            "downloads": detail.downloads,
            "likes": detail.likes,
            "description": detail.description,
            "license": detail.license,
            "context_length": detail.context_length,
            "params": detail.params,
        },
        "capabilities": {
            "claims": caps.claims.iter().map(|c| serde_json::json!({
                "capability": c.capability,
                "label": c.capability.label(),
                "provenance": c.provenance,
            })).collect::<Vec<_>>(),
            "tasks": caps.tasks.iter().map(|t| serde_json::json!({
                "capability": t.capability,
                "task": t.task,
            })).collect::<Vec<_>>(),
            // When a `requires` capability was supplied, an honest,
            // provenance-aware verdict of whether this model satisfies it.
            "fit": requires.map(|cap| {
                let m = decentraai_hub::match_requirements(
                    caps,
                    &[decentraai_hub::CapabilityRequirement {
                        capability: cap,
                        evidence: decentraai_hub::EvidenceLevel::Verified,
                    }],
                );
                serde_json::json!({
                    "capability": cap,
                    "label": cap.label(),
                    "satisfied": m.is_satisfied(),
                    "checks": m.checks.iter().map(|c| serde_json::json!({
                        "capability": c.capability,
                        "status": serde_json::to_value(&c.status).unwrap_or_default(),
                        "reason": c.reason,
                    })).collect::<Vec<_>>(),
                })
            }),
        },
        "variants": fabric_variants,
        "fabric": fabric_nodes,
    })
}
pub(crate) async fn hub_model_body(
    detail: &decentraai_hub::HubModelDetail,
    files: &[decentraai_hub::HubModelFile],
    caps: &decentraai_hub::ModelCapabilities,
    state: &ApiState,
    repo: &str,
    requires: Option<decentraai_hub::CapabilityKind>,
) -> serde_json::Value {
    // Live fabric view: which trusted, ready workers have this model on disk
    // or loaded, and their honest capacity for it. Never fabricated — if a
    // worker is not in the compute registry, it is simply not listed.
    let mut fabric_nodes = Vec::new();
    if let Some(cm) = &state.compute {
        let workers = cm.workers().await;
        for w in workers {
            let served = w
                .capability
                .served_models
                .iter()
                .any(|m| m.file_name == repo);
            let available = w
                .capability
                .available_models
                .iter()
                .any(|m| m.file_name == repo);
            if served || available {
                fabric_nodes.push(serde_json::json!({
                    "node_id": w.node_id,
                    "node_name": w.node_name,
                    "peer_id": w.peer_id.to_string(),
                    "status": format!("{:?}", w.availability.status),
                    "served": served,
                    "available": available,
                    "trusted": cm.is_trusted(&w.peer_id).await,
                }));
            }
        }
    }
    let local_snapshot = decentraai_system_probe::SystemSnapshot::collect();
    let local_avail_ram_mb = local_snapshot.available_memory_bytes / (1024 * 1024);
    let local_vram_mb = match decentraai_system_probe::probe_gpu() {
        decentraai_system_probe::GpuProbeStatus::Nvidia(gpu) => Some(gpu.free_vram_mib),
        _ => None,
    };

    let mut fabric_variants = Vec::new();
    for f in files {
        let size = f.size.unwrap_or(0);
        let est_ram_mb = (size * 120 / 100) / (1024 * 1024);
        let mut can_run_workers = Vec::new();
        if let Some(cm) = &state.compute {
            for w in cm.workers().await {
                let w_ram = w.availability.available_ram_mb;
                let w_vram = w.availability.available_vram_mb.unwrap_or(0);
                if w_ram >= est_ram_mb || w_vram >= est_ram_mb {
                    can_run_workers.push(serde_json::json!({
                        "node_id": w.node_id,
                        "node_name": w.node_name,
                    }));
                }
            }
        }
        fabric_variants.push(serde_json::json!({
            "file": f.path,
            "size_bytes": f.size,
            "sha256": f.lfs.as_ref().map(|l| l.oid.clone()),
            "est_ram_mb": est_ram_mb,
            "local_fit": local_avail_ram_mb >= est_ram_mb || local_vram_mb.unwrap_or(0) >= est_ram_mb,
            "fabric_fit_nodes": can_run_workers,
        }));
    }

    serde_json::json!({
        "id": repo,
        "metadata": {
            "pipeline_tag": detail.pipeline_tag.as_ref().map(|t| t.as_str()),
            "tags": detail.tags,
            "downloads": detail.downloads,
            "likes": detail.likes,
            "description": detail.description,
            "license": detail.license,
            "context_length": detail.context_length,
            "params": detail.params,
        },
        "capabilities": {
            "claims": caps.claims.iter().map(|c| serde_json::json!({
                "capability": c.capability,
                "label": c.capability.label(),
                "provenance": c.provenance,
            })).collect::<Vec<_>>(),
            "tasks": caps.tasks.iter().map(|t| serde_json::json!({
                "capability": t.capability,
                "task": t.task,
            })).collect::<Vec<_>>(),
            // When a `requires` capability was supplied, an honest,
            // provenance-aware verdict of whether this model satisfies it.
            "fit": requires.map(|cap| {
                let m = decentraai_hub::match_requirements(
                    caps,
                    &[decentraai_hub::CapabilityRequirement {
                        capability: cap,
                        evidence: decentraai_hub::EvidenceLevel::Verified,
                    }],
                );
                serde_json::json!({
                    "capability": cap,
                    "label": cap.label(),
                    "satisfied": m.is_satisfied(),
                    "checks": m.checks.iter().map(|c| serde_json::json!({
                        "capability": c.capability,
                        "status": serde_json::to_value(&c.status).unwrap_or_default(),
                        "reason": c.reason,
                    })).collect::<Vec<_>>(),
                })
            }),
        },
        "variants": fabric_variants,
        "fabric": fabric_nodes,
    })
}
pub(crate) async fn admin_models_remove_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }

    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };

    let relative_path = match req.get("path").and_then(|v| v.as_str()) {
        Some(p) if !p.is_empty() => p,
        _ => return forbidden("missing path (relative file name)"),
    };

    // Load the registry to check existence and current state.
    let registry_path = state.info.repo_root.join("db/registry.json");
    let mut registry = match decentraai_registry::ModelRegistry::load(&registry_path) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("failed to load registry for removal: {e:#}");
            return (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"error": {"message": "registry load failed", "type": "registry_error"}})
                    .to_string(),
            )
                .into_response();
        }
    };

    // Check if the model is currently served by comparing the path.
    let current_model_path_opt = {
        let manager_guard = state.manager.lock().await;
        manager_guard.current_model_path().map(|p| p.to_path_buf())
    };

    if let Some(current_model_path) = current_model_path_opt {
        let full_target_path = state.info.repo_root.join("models").join(relative_path);
        if let Ok(canonical_target) = std::fs::canonicalize(&full_target_path) {
            if canonical_target == current_model_path {
                return (
                    StatusCode::CONFLICT,
                    [(header::CONTENT_TYPE, "application/json")],
                    serde_json::json!({"error": {"message": "model is currently served; unload before removal", "type": "conflict"}})
                        .to_string(),
                )
                    .into_response();
            }
        } else {
            // If we can't canonicalize, proceed with removal anyway.
            // This is defensive but risky.
            tracing::debug!(
                "failed to canonicalize target path for removal check: {}",
                full_target_path.display()
            );
        }
    }

    // Remove the model file and entry from registry.
    let record = match registry.remove_model(relative_path) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"error": {"message": e.to_string(), "type": "registry_error"}})
                    .to_string(),
            )
                .into_response();
        }
    };

    // Save the updated registry.
    if let Err(e) = registry.save(&registry_path) {
        tracing::warn!("failed to save registry after removal: {e:#}");
        return (
            StatusCode::BAD_GATEWAY,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({"error": {"message": "registry save failed", "type": "registry_error"}})
                .to_string(),
        )
            .into_response();
    }

    // Refresh the local fabric advertisement so other nodes see the model
    // as removed from the registry. This ensures the model is no longer
    // advertised to the fabric.
    if let Some(cm) = &state.compute {
        cm.set_registry_path(registry_path.clone());
        let ctx = (record.size_bytes / 1024 / 1024) as u32; // Estimate context from size
        if let Err(e) = cm.refresh_local_models(&registry_path, ctx).await {
            tracing::warn!("failed to refresh fabric advertisement after removal: {e:#}");
            // Continue anyway, the model removal succeeded.
        }
    }

    // Record the audit event.
    let audit_path = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        audit_path.parent().unwrap_or(&state.info.repo_root),
        "model_removed",
        serde_json::json!({
            "relative_path": relative_path,
            "size_bytes": record.size_bytes,
            "canonical_path": record.canonical_path,
        }),
    );

    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({"success": true, "message": "model removed"}).to_string(),
    )
        .into_response()
}
pub(crate) async fn admin_settings_generation_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let mut g = state.runtime_generation.write().await;
    if let Some(v) = req.get("temperature").and_then(|v| v.as_f64()) {
        g.temperature = v.clamp(0.0, 2.0) as f32;
    }
    if let Some(v) = req.get("top_p").and_then(|v| v.as_f64()) {
        g.top_p = v.clamp(0.0, 1.0) as f32;
    }
    if let Some(v) = req.get("top_k") {
        g.top_k = v.as_i64().map(|n| n as i32).filter(|k| *k > 0);
    }
    if let Some(v) = req.get("repeat_penalty").and_then(|v| v.as_f64()) {
        g.repeat_penalty = v.clamp(0.0, 4.0) as f32;
    }
    if let Some(v) = req.get("system_prompt").and_then(|v| v.as_str()) {
        g.system_prompt = v.to_string();
    }
    let a = state.info.repo_root.join("logs/audit.jsonl");
    // Persist to the node config (best-effort): the live override already
    // applies immediately, but writing it to node.yaml makes it survive a
    // restart. A write failure only warns — the live override stays valid for
    // the current process.
    let persisted = persist_generation_config(&state.info.repo_root, &g);
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "settings_generation_updated",
        serde_json::json!({
            "temperature": g.temperature,
            "top_p": g.top_p,
            "top_k": g.top_k,
            "repeat_penalty": g.repeat_penalty,
            "persisted": persisted,
        }),
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "success": true,
            "generation": {
                "temperature": g.temperature,
                "top_p": g.top_p,
                "top_k": g.top_k,
                "repeat_penalty": g.repeat_penalty,
                "system_prompt": g.system_prompt,
            },
            "persisted": persisted,
            "note": if persisted { "applied live and persisted to node.yaml" } else { "applied live only (could not persist config)" },
        })
        .to_string(),
    )
        .into_response()
}
/// Best-effort, text-based persistence of the runtime generation defaults into
/// `<data_dir>/node.yaml` under the `generation:` block. Handles a missing
/// file (returns false — live override only) and rewrites atomically
/// (tmp + rename). Kept simple on purpose: the config crate's YAML round-trip
/// would need `NodeConfig: Serialize`; editing the known keys directly is
pub(crate) fn persist_generation_config(
    repo_root: &std::path::Path,
    g: &GenerationSection,
) -> bool {
    use std::io::Write;
    let path = repo_root.join("node.yaml");
    if !path.exists() {
        return false;
    }
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return false;
    };
    let temp = |line: &str, v: String| -> String {
        let trimmed = line.trim_start();
        if trimmed.starts_with("temperature:") {
            format!("{}temperature: {}", &line[..line.len() - trimmed.len()], v)
        } else if trimmed.starts_with("top_p:") {
            format!("{}top_p: {}", &line[..line.len() - trimmed.len()], v)
        } else if trimmed.starts_with("top_k:") {
            format!("{}top_k: {}", &line[..line.len() - trimmed.len()], v)
        } else if trimmed.starts_with("repeat_penalty:") {
            format!(
                "{}repeat_penalty: {}",
                &line[..line.len() - trimmed.len()],
                v
            )
        } else if trimmed.starts_with("system_prompt:") {
            format!(
                "{}system_prompt: {}",
                &line[..line.len() - trimmed.len()],
                serde_json::to_string(&g.system_prompt).unwrap_or_else(|_| "\"\"".to_string())
            )
        } else {
            line.to_string()
        }
    };
    let in_gen = raw.lines().any(|l| l.trim() == "generation:");
    if !in_gen {
        return false;
    }
    let mut after_gen = false;
    let mut out: Vec<String> = Vec::new();
    let mut wrote = false;
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if trimmed == "generation:" {
            after_gen = true;
            out.push(line.to_string());
            continue;
        }
        // Once past `generation:` we only keep replacing until we hit a key at
        // the same or lower indentation (i.e. a sibling key ends the block).
        if after_gen {
            let indent = line.len() - trimmed.len();
            if indent <= 2 && !trimmed.is_empty() && !trimmed.starts_with('#') {
                after_gen = false;
            } else if indent > 2 {
                let is_key = trimmed.starts_with("temperature:")
                    || trimmed.starts_with("top_p:")
                    || trimmed.starts_with("top_k:")
                    || trimmed.starts_with("repeat_penalty:")
                    || trimmed.starts_with("system_prompt:");
                if is_key {
                    out.push(match trimmed.split_once(':').map(|x| x.0.trim()) {
                        Some("temperature") => {
                            wrote = true;
                            temp(line, format!("{}", g.temperature))
                        }
                        Some("top_p") => {
                            wrote = true;
                            temp(line, format!("{}", g.top_p))
                        }
                        Some("top_k") => {
                            wrote = true;
                            temp(
                                line,
                                g.top_k
                                    .map(|k| k.to_string())
                                    .unwrap_or_else(|| "null".into()),
                            )
                        }
                        Some("repeat_penalty") => {
                            wrote = true;
                            temp(line, format!("{}", g.repeat_penalty))
                        }
                        Some("system_prompt") => {
                            wrote = true;
                            temp(line, String::new()) // uses serde_json in temp()
                        }
                        _ => line.to_string(),
                    });
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }
    // If none of the known keys existed, nothing to persist.
    if !wrote {
        return false;
    }
    let content = out.join("\n");
    let tmp = path.with_extension("yaml.tmp");
    let mut f = match std::fs::File::create(&tmp) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "failed to create config tmp file");
            return false;
        }
    };
    if f.write_all(content.as_bytes()).is_err() || f.sync_all().is_err() {
        return false;
    }
    drop(f);
    std::fs::rename(&tmp, &path).is_ok()
}
/// Master-gated runtime settings: update resource admission limits in the node
/// config (`node.yaml` under `resources:`). These are read at engine startup /
/// admission, so the change is persisted for the NEXT start (live-apply is not
/// possible without a restart — resource limits gate the pre-flight check).
/// `gpu_enabled` is intentionally NOT editable here (changing the GPU policy
pub(crate) async fn admin_settings_resources_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let path = state.info.repo_root.join("node.yaml");
    if !path.exists() {
        return forbidden("node.yaml not found — cannot persist resource settings");
    }
    let persisted = persist_resource_config(&path, &req);
    if !persisted {
        return forbidden("could not persist resource settings (no matching keys / write failed)");
    }
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "settings_resources_updated",
        serde_json::json!({
            "applied_keys": req,
            "note": "persisted for next start (resource limits gate startup admission)",
        }),
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "success": true,
            "persisted": true,
            "note": "resource limits saved to node.yaml; applied on the next start",
        })
        .to_string(),
    )
        .into_response()
}
/// Text-based, atomic persistence of resource limit keys under `resources:`
/// in the node config. Edits the known numeric keys only; unknown keys are
/// ignored. Returns false when no known key matched (nothing to write) or the
pub(crate) fn persist_resource_config(path: &std::path::Path, req: &serde_json::Value) -> bool {
    use std::io::Write;
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let keys: [&str; 7] = [
        "cpu_max_percent",
        "memory_max_percent",
        "reserve_cpu_cores",
        "reserve_ram_mb",
        "reserve_vram_mb",
        "gpu_max_vram_percent",
        "stop_gpu_temperature_celsius",
    ];
    let mut wrote = false;
    let mut out: Vec<String> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim_start();
        // Only rewrite lines inside the `resources:` block.
        let mut replaced = None;
        if trimmed.starts_with("resources:") {
            // keep the header
        } else if line.len() - trimmed.len() >= 2 {
            for k in keys {
                if let Some(rest) = trimmed.strip_prefix(k).and_then(|r| r.strip_prefix(':')) {
                    if let Some(v) = req.get(k) {
                        let indent = &line[..line.len() - trimmed.len()];
                        replaced = Some(format!(
                            "{indent}{k}: {}",
                            serde_json::to_string(v)
                                .unwrap_or_default()
                                .replace('"', "")
                        ));
                        wrote = true;
                    }
                    let _ = rest;
                    break;
                }
            }
        }
        out.push(replaced.unwrap_or_else(|| line.to_string()));
    }
    if !wrote {
        return false;
    }
    let content = out.join("\n");
    let tmp = path.with_extension("yaml.tmp");
    let mut f = match std::fs::File::create(&tmp) {
        Ok(f) => f,
        Err(_) => return false,
    };
    if f.write_all(content.as_bytes()).is_err() || f.sync_all().is_err() {
        return false;
    }
    drop(f);
    std::fs::rename(&tmp, path).is_ok()
}
/// Master-gated model selector: picks the GGUF file this node serves.
/// Persists `node.model` in the node config (atomic, survives restarts) and —
/// when a local engine with a restart spec is running — swaps the model and
/// respawns llama-server live. Remote-backend / no-engine nodes get the
pub(crate) async fn admin_model_select_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let name = match req.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.is_empty() => n,
        _ => return forbidden("missing model name"),
    };
    // Path safety: the model must be a plain file name inside models/ — never
    // accept separators or absolute paths (same rule as the registry).
    let file_name = std::path::Path::new(name)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");
    if file_name.is_empty() || file_name != name {
        return forbidden("model name must be a plain file name (no path separators)");
    }
    let models_dir = state.info.repo_root.join("models");
    let model_path = models_dir.join(file_name);
    if !model_path.is_file() {
        return (
            StatusCode::NOT_FOUND,
            serde_json::json!({
                "success": false,
                "error": format!("model '{}' not found in {}", file_name, models_dir.display()),
            })
            .to_string(),
        )
            .into_response();
    }
    // Persist node.model atomically in the node config. Remember the previous
    // model so a failed respawn can roll back to a known-good engine.
    let config_path = state.info.repo_root.join("node.yaml");
    let previous_model = read_node_model(&config_path);
    let persisted = if config_path.exists() {
        persist_model_config(&config_path, file_name)
    } else {
        false
    };
    // Live respawn when a local engine with a restart spec is attached.
    let mut respawned = false;
    let note;
    {
        let mut manager = state.manager.lock().await;
        if manager.set_restart_model(model_path.clone()) {
            // Swap the model: stop the current engine, then let the M24
            // supervisor respawn it from the updated restart spec.
            let _ = manager.shutdown().await;
            match manager.ensure_healthy().await {
                Ok(true) => {
                    respawned = true;
                    *state.active_model.write().await = file_name.to_string();
                    note = if persisted {
                        "model saved to node.yaml and llama-server respawned with it".to_string()
                    } else {
                        "llama-server respawned live (node.yaml not writable — restart needed to persist)".to_string()
                    };
                }
                Ok(false) => {
                    // Rollback: the new model failed to load (wrong arch,
                    // corrupt file, too slow to be ready). Restore the
                    // previous model in node.yaml and respawn with it so the
                    // node is never left without an engine.
                    let rollback = previous_model
                        .as_deref()
                        .filter(|prev| prev != &file_name)
                        .map(|prev| {
                            let path = state.info.repo_root.join("models").join(prev);
                            if path.is_file() {
                                (prev.to_string(), path)
                            } else {
                                (String::new(), PathBuf::new())
                            }
                        })
                        .filter(|(p, _)| !p.is_empty());
                    match rollback {
                        Some((prev_name, prev_path)) => {
                            let restored = persist_model_config(&config_path, &prev_name);
                            let _ = manager.shutdown().await;
                            let _ = manager.set_restart_model(prev_path);
                            let ok = manager.ensure_healthy().await.unwrap_or(false);
                            if restored && ok {
                                *state.active_model.write().await = prev_name.clone();
                            }
                            note = if restored && ok {
                                format!(
                                    "model '{file_name}' failed to load — rolled back to '{prev_name}' and the engine is serving again"
                                )
                            } else {
                                format!(
                                    "model '{file_name}' failed to load; rollback to '{prev_name}' attempted (persisted={restored}, engine={ok}) — check logs"
                                )
                            };
                        }
                        None => {
                            note = "model saved; engine respawn failed — no previous model to roll back to; check logs and restart the node".to_string();
                        }
                    }
                }
                Err(e) => {
                    note =
                        format!("model saved; engine respawn error: {e:.200} — restart the node");
                }
            }
        } else {
            note = if persisted {
                "model saved to node.yaml — restart the node to serve it".to_string()
            } else {
                "no local engine / node.yaml not writable — restart the node after editing node.yaml manually".to_string()
            };
        }
    }
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "model_selected",
        serde_json::json!({ "model": file_name, "persisted": persisted, "respawned": respawned }),
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "success": true,
            "model": file_name,
            "persisted": persisted,
            "respawned": respawned,
            "note": note,
        })
        .to_string(),
    )
        .into_response()
}
/// Atomically rewrites the `model:` line under the `node:` block in the node
pub(crate) fn persist_model_config(path: &std::path::Path, model_name: &str) -> bool {
    use std::io::Write;
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let mut out: Vec<String> = Vec::new();
    let mut wrote = false;
    let mut in_node_block = false;
    for line in raw.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if in_node_block {
            // Leaving the node block: a line at indent 0 that is not a comment.
            if indent == 0 && !trimmed.is_empty() && !trimmed.starts_with('#') {
                in_node_block = false;
            } else if let Some(rest) = trimmed.strip_prefix("model:") {
                let value = rest.trim();
                let _ = value;
                out.push(format!(
                    "{}{}model: \"{}\"",
                    &line[..indent],
                    "",
                    model_name
                ));
                wrote = true;
                continue;
            }
        } else if trimmed.starts_with("node:") && trimmed.len() == "node:".len() {
            in_node_block = true;
        }
        out.push(line.to_string());
    }
    if !wrote {
        return false;
    }
    let content = out.join("\n");
    let tmp = path.with_extension("yaml.tmp");
    let mut f = match std::fs::File::create(&tmp) {
        Ok(f) => f,
        Err(_) => return false,
    };
    if f.write_all(content.as_bytes()).is_err() || f.sync_all().is_err() {
        return false;
    }
    drop(f);
    std::fs::rename(&tmp, path).is_ok()
}
/// Reads the `model:` value under the `node:` block in the node config —
/// the model that was active before a model-select swap, used for rollback.
/// Returns `None` when the file is missing, has no `node:` block, or has no
pub(crate) fn read_node_model(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut in_node_block = false;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if in_node_block {
            if let Some(rest) = trimmed.strip_prefix("model:") {
                let value = rest.trim().trim_matches('"').trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
            // A non-empty, non-comment line at a smaller indent ends the block.
            if !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !line.starts_with(char::is_whitespace)
            {
                break;
            }
        } else if trimmed.starts_with("node:") && trimmed.len() == "node:".len() {
            in_node_block = true;
        }
    }
    None
}
/// Shared body-parsing + peer-id extraction for the worker trust / revoke
pub(crate) fn parse_worker_peer_id(body: &Bytes) -> Result<decentraai_p2p::PeerId, String> {
    let req: serde_json::Value = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(_) => return Err("invalid JSON".to_string()),
    };
    let peer = match req.get("peer_id").and_then(|v| v.as_str()) {
        Some(p) if !p.is_empty() => p,
        _ => return Err("missing peer_id".to_string()),
    };
    decentraai_p2p::PeerId::from_str(peer).map_err(|_| "invalid peer_id".to_string())
}
/// Master-gated read-only view of the contribution report + suggested tiers.
/// Reuses the live `ComputeManager::contribution_report` (the same data the
/// CLI `decentraai tier suggest` prints) so the dashboard can show why each
pub(crate) async fn admin_contribution_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(compute) = &state.compute else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "compute manager not attached"})),
        )
            .into_response();
    };
    let rows = compute.contribution_report().await;
    // Pair each contribution row to its token suggestion (reuse the same
    // planner the CLI uses).
    let suggestions: Vec<decentraai_tokens::SuggestedTier> = rows
        .iter()
        .map(|r| decentraai_tokens::SuggestedTier {
            name: r.node_name.clone(),
            suggested: r.suggested_tier,
        })
        .collect();
    let tokens: Vec<decentraai_tokens::TokenRecord> = match &state.token_store_path {
        Some(p) => decentraai_tokens::TokenStore::load(p)
            .map(|s| s.list())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let changes = decentraai_tokens::plan_tier_changes(&suggestions, &tokens);
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "rows": rows,
            "suggested": suggestions,
            "changes": changes,
        })
        .to_string(),
    )
        .into_response()
}
/// Master-gated mutation: apply the contribution-suggested tiers to the token
/// registry (the same action as `decentraai tier apply --yes`). Only pairs an
/// active token to its same-named worker's suggested tier; a token with no
pub(crate) async fn admin_tier_apply_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    // Explicit confirmation for the mutation.
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    if req.get("confirm").and_then(|c| c.as_bool()) != Some(true) {
        return forbidden("tier apply requires \"confirm\": true (mutation)");
    }
    let Some(compute) = &state.compute else {
        return forbidden("no compute manager attached");
    };
    let Some(store_path) = &state.token_store_path else {
        return forbidden("no token registry (tiers disabled)");
    };
    let rows = compute.contribution_report().await;
    let suggestions: Vec<decentraai_tokens::SuggestedTier> = rows
        .iter()
        .map(|r| decentraai_tokens::SuggestedTier {
            name: r.node_name.clone(),
            suggested: r.suggested_tier,
        })
        .collect();
    let mut store = match decentraai_tokens::TokenStore::load(store_path) {
        Ok(s) => s,
        Err(_) => return forbidden("token registry unreadable"),
    };
    let tokens = store.list();
    let changes = decentraai_tokens::plan_tier_changes(&suggestions, &tokens);
    let mut applied = 0usize;
    for c in &changes {
        if store
            .set_tier(&c.name, decentraai_tokens::Tier(c.to))
            .is_ok()
        {
            applied += 1;
            let a = state.info.repo_root.join("logs/audit.jsonl");
            let _ = decentraai_audit::record(
                a.parent().unwrap_or(&state.info.repo_root),
                "tier_changed",
                serde_json::json!({ "name": c.name, "from": c.from, "to": c.to }),
            );
        }
    }
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "success": true,
            "applied": applied,
            "total_changes": changes.len(),
            "changes": changes,
        })
        .to_string(),
    )
        .into_response()
}
/// P3/M10 — Approve a worker: adds it to the coordinator's trust set so it
/// becomes eligible to run workloads. Master-gated like the other admin
/// endpoints. Guards gracefully (a clear OpenAI-style error, not a panic)
pub(crate) async fn admin_worker_trust_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let peer = match parse_worker_peer_id(&body) {
        Ok(p) => p,
        Err(msg) => return forbidden(&msg),
    };
    let Some(compute) = &state.compute else {
        return forbidden("no compute manager attached; worker trust unavailable");
    };
    compute.add_trusted(peer).await;
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "worker_trusted",
        serde_json::json!({"peer_id": peer.to_string()}),
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({"success": true, "peer_id": peer.to_string(), "trusted": true})
            .to_string(),
    )
        .into_response()
}
/// P3/M10 — Revoke a worker: removes it from the coordinator's trust set.
pub(crate) async fn admin_worker_revoke_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let peer = match parse_worker_peer_id(&body) {
        Ok(p) => p,
        Err(msg) => return forbidden(&msg),
    };
    let Some(compute) = &state.compute else {
        return forbidden("no compute manager attached; worker trust unavailable");
    };
    compute.remove_trusted(&peer).await;
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "worker_revoked",
        serde_json::json!({"peer_id": peer.to_string()}),
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({"success": true, "peer_id": peer.to_string(), "trusted": false})
            .to_string(),
    )
        .into_response()
}
/// P3/M10 — Recent audit events for the Admin page, master-gated. Reuses the
pub(crate) async fn admin_audit_events_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let events = recent_audit_events(&state.info.repo_root);
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({"events": events}).to_string(),
    )
        .into_response()
}
pub(crate) async fn admin_quota_grant_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(ledger) = &state.quota_ledger else {
        return forbidden("quota ledger is not attached");
    };
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let account = match req.get("account").and_then(|v| v.as_str()) {
        Some(a) if !a.trim().is_empty() => a.trim().to_string(),
        _ => return forbidden("missing account"),
    };
    let amount = req.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);
    if amount == 0 {
        return forbidden("amount must be > 0");
    }
    let ref_id = format!(
        "admin-grant-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let mut l = ledger.lock().unwrap();
    let credited = l.credit(&account, &ref_id, Some(amount as u32), None);
    drop(l);
    tracing::info!(
        account,
        amount,
        credited,
        "admin granted quota to consumer account"
    );
    (
        StatusCode::OK,
        serde_json::json!({"account": account, "amount": amount, "credited": credited}).to_string(),
    )
        .into_response()
}
pub(crate) fn recent_audit_events(data_dir: &Path) -> Vec<serde_json::Value> {
    let path = data_dir.join("logs/audit.jsonl");
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .rev()
        .take(DASHBOARD_EVENT_LIMIT)
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
