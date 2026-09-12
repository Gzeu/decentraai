//! GET /account — wallet onboarding page (xPortal self-serve, M16-era).
//!
//! Public, no master required. Flow: pick network intent (testnet/mainnet)
//! → connect wallet (DeFi extension, xPortal QR, or manual paste) → the
//! wallet signs the backend challenge → session → ONE `dca_` consumer key,
//! shown exactly once, opening both doors (OpenAI-compatible API + MCP).
//!
//! Security boundaries (same as the other public pages):
//! - The page is served `no-store`; prompts/outputs are never logged.
//! - Only key PREFIXES ever appear in errors; plaintext lives in the
//!   issuance response + the user's clipboard/localStorage, never in logs.
//! - The signed message binds the SERVER network (`DECENTRAAI_MX_NETWORK`);
//!   the network toggle records intent (`purpose=onboard:<net>`), it cannot
//!   spoof chain binding. Keys are fabric-scoped (chain-agnostic).
//! - The MultiversX SDKs load from pinned jsDelivr `+esm` URLs — the single
//!   external dependency of this page. Offline, or if a provider API
//!   drifts, every flow degrades to manual paste (zero-dep, always works).

/// Pinned MultiversX signing providers (jsDelivr `+esm` browser builds).
pub const MX_EXTENSION_PROVIDER_URL: &str =
    "https://cdn.jsdelivr.net/npm/@multiversx/sdk-extension-provider@5.1.2/+esm";
/// Pinned MultiversX signing providers (jsDelivr `+esm` browser builds).
pub const MX_WALLETCONNECT_PROVIDER_URL: &str =
    "https://cdn.jsdelivr.net/npm/@multiversx/sdk-wallet-connect-provider@6.1.5/+esm";

/// The account onboarding HTML (no-store; all state via the wallet API).
pub fn account_html() -> String {
    ACCOUNT_HTML
        .replace("/*__MX_EXTENSION_URL__*/", MX_EXTENSION_PROVIDER_URL)
        .replace("/*__MX_WC_URL__*/", MX_WALLETCONNECT_PROVIDER_URL)
}

const ACCOUNT_HTML: &str = r##"<!doctype html><html lang="ro"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>DecentraAI — Cont</title>
<style>
:root{--bg:#070a12;--panel:#0f172a;--line:#1f2a44;--text:#e6eef8;--muted:#8aa0b8;--accent:#22d3ee;--ok:#34d399;--warn:#fbbf24;--err:#f87171}
*{box-sizing:border-box;margin:0;padding:0}body{background:radial-gradient(1000px 600px at 30% -10%,#1a2540 0%,transparent 60%),var(--bg);color:var(--text);font:14px/1.6 system-ui,sans-serif;padding:24px;max-width:640px;margin:0 auto}
h1{font-size:24px;margin-bottom:4px}h1 span{color:var(--accent)}p.sub{color:var(--muted);margin-bottom:16px}
.card{background:linear-gradient(180deg,#0f172a 0%,#0b1222 100%);border:1px solid var(--line);border-radius:14px;padding:18px;margin-bottom:14px;box-shadow:0 6px 20px #0006}
.card h2{font-size:15px;margin-bottom:8px}.card h2 .n{display:inline-block;width:22px;height:22px;border-radius:50%;background:#1a2a4a;border:1px solid #2a3a5e;text-align:center;font-size:12px;line-height:20px;margin-right:8px;color:var(--accent)}
.row{display:flex;gap:8px;flex-wrap:wrap}
button{padding:10px 14px;border-radius:10px;border:1px solid #2a3a5e;background:linear-gradient(180deg,#1a2a4a,#12203a);color:var(--text);font-weight:600;cursor:pointer}
button:hover{border-color:var(--accent)}button:disabled{opacity:.5;cursor:not-allowed}
button.primary{border-color:var(--accent)}button.danger{border-color:var(--err)}
button.sel{border-color:var(--ok);box-shadow:0 0 0 1px var(--ok)}
input,textarea{width:100%;padding:10px 12px;border-radius:10px;border:1px solid #22304a;background:#0a0e16;color:var(--text);font-size:13px;margin-top:6px;font-family:ui-monospace,monospace}
label{font-size:12px;color:var(--muted);display:block;margin-top:10px}
pre{margin-top:12px;background:#0a0e16;border:1px solid var(--line);border-radius:10px;padding:12px;font-size:12px;white-space:pre-wrap;word-break:break-all;color:var(--muted)}
#out2{display:none}
.ok{color:var(--ok)}.err{color:var(--err)}.warn{color:var(--warn)}
.keybox{font-size:15px;color:var(--ok);border:1px dashed var(--ok);padding:12px;border-radius:10px;margin-top:10px;word-break:break-all;user-select:all}
.badge{display:inline-block;font-size:11px;padding:2px 8px;border-radius:20px;border:1px solid var(--line);color:var(--muted);margin-left:8px}
a{color:var(--accent);text-decoration:none}
.hidden{display:none}
code{background:#0a0e16;padding:1px 6px;border-radius:6px;border:1px solid var(--line);font-size:12px}
</style></head><body>
<h1>● DecentraAI <span>Cont</span><span class="badge" id="nodeNet">nod: …</span></h1>
<p class="sub">Conectează wallet-ul MultiversX (testnet sau mainnet) și primești cheia de acces în fabrică — <code>dca_</code>, compatibilă OpenAI. Fără cont anterior, fără master.</p>

<div class="card" id="step1"><h2><span class="n">1</span>Rețeaua ta</h2>
<div class="row">
<button id="netT" onclick="setNet('multiversx-testnet')">Testnet</button>
<button id="netM" onclick="setNet('multiversx-mainnet')">Mainnet</button>
</div>
<p class="sub" style="margin:8px 0 0">Intenție înregistrată + semnată în challenge (<code>purpose=onboard:…</code>). Legarea de lanț o face nodul (<code id="nodeNet2">…</code>) — cheia fabricii e valabilă oricum.</p>
</div>

<div class="card" id="step2"><h2><span class="n">2</span>Conectează wallet-ul</h2>
<div class="row">
<button id="mExt" onclick="connectExtension()">DeFi Extension</button>
<button id="mXpo" onclick="connectXportal()">xPortal (QR)</button>
<button id="mMan" onclick="showManual()">Manual / alt wallet</button>
</div>
<div id="manualBox" class="hidden">
<label>Adresă wallet (erd1…)</label><input id="manAddr" placeholder="erd1…" autocomplete="off">
<div class="row" style="margin-top:8px"><button onclick="manualChallenge()">1. Cere mesaj de semnat</button></div>
<pre id="manMsg" style="display:none"></pre>
<label>Semnătură (hex sau base64) a mesajului de mai sus</label><textarea id="manSig" rows="3" placeholder="semnează mesajul în wallet-ul tău, lipește aici" autocomplete="off"></textarea>
<div class="row" style="margin-top:8px"><button class="primary" onclick="manualLogin()">2. Verifică și intră →</button></div>
</div>
<div id="wcBox" class="hidden">
<label>Scanează / împerechează în xPortal:</label>
<pre id="wcUri" style="display:block"></pre>
<div class="row" style="margin-top:8px"><button onclick="copyWc()">Copiază URI împerechere</button></div>
<p class="sub" style="margin:8px 0 0">xPortal → WalletConnect → lipește URI-ul (sau scanează dacă îl vezi ca QR în alt client). Aprobă conectarea, apoi semnează mesajul challenge.</p>
</div>
<pre id="out2"></pre>
</div>

<div class="card hidden" id="step3"><h2><span class="n">3</span>Cheia ta <span class="badge">arătată o singură dată</span></h2>
<div id="keyInfo"></div>
<div class="keybox" id="keyPlain"></div>
<div class="row" style="margin-top:10px">
<button class="primary" onclick="copyKey()">Copiază cheia</button>
<button class="danger" onclick="rotateKey()">Revocă + re-emite</button>
</div>
<p class="sub" style="margin:8px 0 0">Salvată și în browser (localStorage). Serverul păstrează doar hash + prefix. Pierdută = revocă + re-emite, durează 5 secunde.</p>
</div>

<div class="card hidden" id="step4"><h2><span class="n">4</span>Folosește-o — o cheie, două uși</h2>
<label>OpenAI-compatibil (chat, embeddings)</label>
<pre id="snipOpenai" style="display:block"></pre>
<label>MCP (agenți)</label>
<pre id="snipMcp" style="display:block"></pre>
</div>

<p class="sub">Cheia are cote modeste (1000 unități, 50/min) — suficient pentru onboarding real. Pagini: <a href="/ui2">dashboard</a> · <a href="/fabric">fabric</a> · <a href="/world/join">world/join</a></p>

<script>
const S={net:'multiversx-testnet',addr:null,chal:null,session:null,key:null,keyId:null,nodeNet:null,wcUri:null};
const $=id=>document.getElementById(id);
function say(el,msg,cls){const e=$(el);e.style.display='block';e.textContent=msg;e.className=cls||'';}
function purpose(){return 'onboard:'+(S.net==='multiversx-mainnet'?'mainnet':'testnet');}
function setNet(n){S.net=n;$('netT').className=n.endsWith('testnet')?'sel':'';$('netM').className=n.endsWith('mainnet')?'sel':'';}
setNet(S.net);
async function api(path,method,body){const r=await fetch(path,{method:method||'POST',headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined});const j=await r.json().catch(()=>({}));return{status:r.status,json:j};}
async function getChallenge(addr){
  const r=await api('/v1/auth/wallet/challenge','POST',{wallet_address:addr,purpose:purpose()});
  if(r.status!==200)throw new Error('challenge: '+(r.json.error||r.status));
  S.nodeNet=r.json.network;$('nodeNet').textContent='nod: '+r.json.network;$('nodeNet2').textContent=r.json.network;
  if(r.json.network==='multiversx-testnet'&&S.net==='multiversx-mainnet')say('out2','Atenție: nodul e pe TESTNET, tu ai ales MAINNET. Cheia fabricii funcționează oricum.','warn');
  if(r.json.network==='multiversx-mainnet'&&S.net==='multiversx-testnet')say('out2','Atenție: nodul e pe MAINNET, tu ai ales TESTNET. Cheia fabricii funcționează oricum.','warn');
  return r.json;
}
async function doVerify(addr,chalId,sig){
  const r=await api('/v1/auth/wallet/verify','POST',{wallet_address:addr,challenge_id:chalId,signature:sig});
  if(r.status!==200)throw new Error('verify: '+(r.json.error||r.status));
  S.session=r.json.session_token;S.addr=r.json.wallet_address;
  await mintKey();
}
async function mintKey(){
  const r=await api('/v1/auth/wallet/key','POST',{session_token:S.session});
  if(r.status===409){ // deja emisă: arată key_id + oferă rotația
    say('out2','Ai deja o cheie ('+r.json.key_id+'). O poți roti mai jos — plaintext-ul vechi nu se mai arată.','warn');
    S.keyId=r.json.key_id;showManageOnly();return;
  }
  if(r.status!==200||!r.json.ok)throw new Error('key: '+(r.json.error||r.status));
  S.key=r.json.token;S.keyId=r.json.key_id;
  $('keyPlain').textContent=r.json.token;
  $('keyInfo').innerHTML='Cont <code>'+esc(r.json.account)+'</code> · wallet <code>'+esc(r.json.wallet)+'</code><br>key_id <code>'+esc(r.json.key_id)+'</code> · cotă '+r.json.quota_ceiling+' · '+r.json.rate_limit_per_minute+'/min · start '+(r.json.starter_granted?r.json.starter_quota+' (grant)':'0 (deja alimentat)');
  try{localStorage.setItem('decentraai.account.key',r.json.token);localStorage.setItem('decentraai.account.key_id',r.json.key_id);}catch(_){}
  $('step3').classList.remove('hidden');showSnippets(r.json.token);
  say('out2','Autentificat ca '+r.json.wallet+'. Cheia de mai sus NU se mai arată — copiaz-o acum.','ok');
}
function showManageOnly(){
  $('keyPlain').textContent='(ascunsă — emisă anterior; rotește pentru una nouă)';
  $('keyInfo').innerHTML='key_id <code>'+esc(S.keyId)+'</code> · apasă <b>Revocă + re-emite</b> pentru o cheie nouă (cea veche moare instant).';
  $('step3').classList.remove('hidden');
}
function showSnippets(tok){
  const base=location.origin+'/v1';
  $('snipOpenai').textContent='import OpenAI from "openai";\nconst ai=new OpenAI({baseURL:"'+base+'",apiKey:"'+tok+'"});\nawait ai.chat.completions.create({model:"auto",messages:[{role:"user",content:"salut"}]});';
  $('snipMcp').textContent='curl -X POST '+location.origin+'/mcp \\\n -H "Authorization: Bearer '+tok+'" \\\n -H "Content-Type: application/json" \\\n -d \'{"jsonrpc":"2.0","id":1,"method":"tools/list"}\'';
  $('step4').classList.remove('hidden');
}
async function rotateKey(){
  if(!S.session){say('out2','Sesiune expirată — reconectează wallet-ul (pasul 2).','err');return;}
  const d=await api('/v1/auth/wallet/key','DELETE',{session_token:S.session});
  if(d.status!==200){say('out2','revoke: '+(d.json.error||d.status),'err');return;}
  S.key=null;await mintKey();
}
function copyKey(){const t=S.key||'(nemaifișată)';navigator.clipboard.writeText(t).then(()=>say('out2','Cheia e în clipboard.','ok'));}
function copyWc(){if(S.wcUri)navigator.clipboard.writeText(S.wcUri).then(()=>say('out2','URI împerechere copiat — lipește-l în xPortal → WalletConnect.','ok'));}
function esc(s){return String(s).replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));}
// ---- metoda: DeFi Extension (SDK pin-uit, import dinamic) ----
async function connectExtension(){
  say('out2','Se încarcă provider-ul DeFi…','');
  try{
    const mod=await import('/*__MX_EXTENSION_URL__*/');
    const Provider=mod.ExtensionProvider||mod.default;
    if(!Provider||typeof Provider.create!=='function')throw new Error('SDK încărcat, dar ExtensionProvider.create lipsește (formă neașteptată). Încearcă Manual.');
    const p=await Provider.create();
    if(typeof p.init==='function')await p.init();
    if(typeof p.login!=='function')throw new Error('provider fără login(). Încearcă Manual.');
    await p.login();
    const addr=p.account&&p.account.address?p.account.address:(p.address||null);
    if(!addr)throw new Error('login ok, dar adresa lipsește. Încearcă Manual.');
    say('out2','Conectat: '+addr+' — cere challenge…','');
    const chal=await getChallenge(addr);
    if(typeof p.signMessage!=='function')throw new Error('provider fără signMessage(). Semnează manual mesajul din challenge.');
    const signed=await p.signMessage(chal.message);
    const sig=extractSig(signed);
    if(!sig)throw new Error('semnătură ilizibilă din provider. Încearcă Manual.');
    await doVerify(addr,chal.challenge_id,sig);
  }catch(e){say('out2','Extension: '+String(e.message||e).slice(0,300),'err');}
}
function extractSig(signed){
  if(!signed)return null;
  if(typeof signed==='string')return signed;
  const s=signed.signature||signed;
  if(typeof s==='string')return s;
  if(s&&typeof s.hex==='function')try{return s.hex();}catch(_){}
  if(s&&typeof s.toString==='function'&&s.toString()!=='[object Object]')return s.toString();
  return null;
}
// ---- metoda: xPortal prin WalletConnect (QR/URI) ----
async function connectXportal(){
  say('out2','Se încarcă provider-ul WalletConnect…','');
  try{
    const mod=await import('/*__MX_WC_URL__*/');
    const WC=mod.WalletConnectV2Provider||mod.WalletConnectProvider||mod.default;
    if(!WC)throw new Error('SDK încărcat, dar nu găsesc provider-ul (exports: '+Object.keys(mod).slice(0,8).join(',')+'). Încearcă Manual.');
    const p=new WC({chainId:S.net==='multiversx-mainnet'?'1':'T'});
    if(typeof p.init==='function')await p.init();
    if(typeof p.login!=='function')throw new Error('provider fără login(). Încearcă Manual.');
    const out=await p.login();
    let uri=out&&out.uri?out.uri:(typeof out==='string'?out:null);
    let approval=out&&out.approval?out.approval:null;
    if(uri){S.wcUri=uri;$('wcUri').textContent=uri;$('wcBox').classList.remove('hidden');say('out2','Împerechează xPortal, apoi aprobă. Aștept…','');}
    if(approval)await approval();
    const addr=p.account&&p.account.address?p.account.address:(p.address||null);
    if(!addr)throw new Error('conectat, dar adresa lipsește. Încearcă Manual.');
    const chal=await getChallenge(addr);
    if(typeof p.signMessage!=='function')throw new Error('conectat, dar fără signMessage(). Semnează manual.');
    const sig=extractSig(await p.signMessage(chal.message));
    if(!sig)throw new Error('semnătură ilizibilă. Încearcă Manual.');
    await doVerify(addr,chal.challenge_id,sig);
  }catch(e){say('out2','xPortal: '+String(e.message||e).slice(0,300),'err');}
}
// ---- metoda: manual (zero dependențe, merge mereu) ----
function showManual(){$('manualBox').classList.remove('hidden');say('out2','1) tastează adresa → Cere mesaj → 2) semnează mesajul EXACT în wallet → 3) lipește semnătura → Verifică.','');}
async function manualChallenge(){
  const addr=$('manAddr').value.trim();
  if(!/^erd1[0-9a-z]{58}$/.test(addr)){say('out2','Adresă invalidă (erd1 + 58 caractere).','err');return;}
  try{
    S.manChal=await getChallenge(addr);
    const m=$('manMsg');m.style.display='block';m.textContent='Semnează EXACT (byte-cu-byte):\n'+S.manChal.message;
    say('out2','Mesaj emis. Semnează-l în wallet, lipește semnătura, apasă Verifică.','warn');
  }catch(e){say('out2','Manual: '+String(e.message||e).slice(0,300),'err');}
}
async function manualLogin(){
  const addr=$('manAddr').value.trim(),sig=$('manSig').value.trim();
  if(!S.manChal||S.manChal.wallet_address!==addr){say('out2','Cere întâi mesajul (pasul 1) pentru adresa asta.','err');return;}
  if(!sig){say('out2','Lipsește semnătura.','err');return;}
  try{await doVerify(addr,S.manChal.challenge_id,sig);}
  catch(e){say('out2','Manual: '+String(e.message||e).slice(0,300),'err');}
}
</script></body></html>"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_page_markers() {
        // Structural contract for GET /account: title, the three wallet
        // methods, pinned provider URLs, backend endpoint refs, once-only
        // wording, and NO secret-looking material in the template.
        let html = account_html();
        for marker in [
            "DecentraAI — Cont",
            "multiversx-testnet",
            "multiversx-mainnet",
            "DeFi Extension",
            "xPortal (QR)",
            "Manual / alt wallet",
            "/v1/auth/wallet/challenge",
            "/v1/auth/wallet/verify",
            "/v1/auth/wallet/key",
            "arătată o singură dată",
            "Revocă + re-emite",
            "dca_",
        ] {
            assert!(html.contains(marker), "missing marker: {marker}");
        }
        assert!(
            html.contains(MX_EXTENSION_PROVIDER_URL),
            "extension provider URL must be pinned"
        );
        assert!(
            html.contains(MX_WALLETCONNECT_PROVIDER_URL),
            "walletconnect provider URL must be pinned"
        );
        assert!(
            !html.contains("/*__MX_EXTENSION_URL__*/") && !html.contains("/*__MX_WC_URL__*/"),
            "URL placeholders must be substituted"
        );
        for bad in ["dsk_", "BEGIN PRIVATE", "seed"] {
            assert!(!html.contains(bad), "template must not contain: {bad}");
        }
    }

    #[test]
    fn provider_urls_are_pinned_versions() {
        // No @latest / floating tags: reproducible client, no supply-chain drift.
        for url in [MX_EXTENSION_PROVIDER_URL, MX_WALLETCONNECT_PROVIDER_URL] {
            assert!(url.starts_with("https://cdn.jsdelivr.net/npm/@multiversx/"));
            assert!(url.ends_with("/+esm"));
            assert!(!url.contains("@latest"), "must pin exact version: {url}");
        }
    }
}
