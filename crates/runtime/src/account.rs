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

/// Pinned MultiversX signing provider (jsDelivr `+esm` browser build).
/// First-party MultiversX code only — no external accounts, no relays.
pub const MX_EXTENSION_PROVIDER_URL: &str =
    "https://cdn.jsdelivr.net/npm/@multiversx/sdk-extension-provider@5.1.2/+esm";
/// Pinned cross-window Web Wallet provider (popup, official wallet URLs).
pub const MX_XWINDOW_PROVIDER_URL: &str =
    "https://cdn.jsdelivr.net/npm/@multiversx/sdk-web-wallet-cross-window-provider@3.2.2/+esm";
/// Pinned sdk-core (documented SignableMessage shape for signMessage).
pub const MX_CORE_URL: &str = "https://cdn.jsdelivr.net/npm/@multiversx/sdk-core@15.3.1/+esm";
/// Official Web Wallet URLs (popup target per selected network).
pub const MX_WEB_WALLET_MAINNET: &str = "https://wallet.multiversx.com";
pub const MX_WEB_WALLET_TESTNET: &str = "https://testnet-wallet.multiversx.com";

/// The account onboarding HTML (no-store; all state via the wallet API).
pub fn account_html() -> String {
    ACCOUNT_HTML
        .replace("/*__MX_EXTENSION_URL__*/", MX_EXTENSION_PROVIDER_URL)
        .replace("/*__MX_XWINDOW_URL__*/", MX_XWINDOW_PROVIDER_URL)
        .replace("/*__MX_CORE_URL__*/", MX_CORE_URL)
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
<button id="mWeb" onclick="connectXWindow()">Web Wallet (popup)</button>
<button id="mXpo" onclick="showXportal()">xPortal (aplicație)</button>
<button id="mMan" onclick="showManual()">Manual / alt wallet</button>
</div>
<div id="xpoBox" class="hidden">
<p class="sub" style="margin:4px 0 0">xPortal pe telefon nu se poate împerechea direct fără relay extern — de aceea pagina nu-l cere. Calea first-party: <b>1)</b> tastează adresa mai jos → <b>2)</b> cere mesajul → <b>3)</b> semnează mesajul exact în xPortal → <b>4)</b> lipește semnătura și verifică. Totul mai jos, zero dependențe.</p>
<div class="row" style="margin-top:8px"><button class="primary" onclick="showManual()">Continuă cu semnare manuală →</button></div>
</div>
<div id="manualBox" class="hidden">
<label>Adresă wallet (erd1…)</label><input id="manAddr" placeholder="erd1…" autocomplete="off">
<div class="row" style="margin-top:8px"><button onclick="manualChallenge()">1. Cere mesaj de semnat</button><button onclick="copyMsg()">Copiază mesajul</button></div>
<pre id="manMsg" style="display:none"></pre>
<label>Semnătură (hex sau base64) a mesajului de mai sus — se verifică automat la lipire</label><textarea id="manSig" rows="3" placeholder="semnează mesajul în wallet-ul tău, lipește aici" autocomplete="off" onpaste="setTimeout(manualLogin,300)"></textarea>
<div class="row" style="margin-top:8px"><button class="primary" onclick="manualLogin()">2. Verifică și intră →</button></div>
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
const S={net:'multiversx-testnet',addr:null,chal:null,session:null,key:null,keyId:null,nodeNet:null};
const $=id=>document.getElementById(id);
function say(el,msg,cls){const e=$(el);e.style.display='block';e.textContent=msg;e.className=cls||'';}
function purpose(){return 'onboard:'+(S.net==='multiversx-mainnet'?'mainnet':'testnet');}
function setNet(n){S.net=n;$('netT').className=n.endsWith('testnet')?'sel':'';$('netM').className=n.endsWith('mainnet')?'sel':'';}
setNet(S.net);
// Node chain binding on page load (public endpoint — no session needed).
// The signed challenge binds this server-side value; the toggle below
// only records intent.
(async function loadNetwork(){
  try{
    const r=await fetch('/v1/auth/wallet/network');const j=await r.json();
    if(j&&j.network){S.nodeNet=j.network;$('nodeNet').textContent='nod: '+j.network;$('nodeNet2').textContent=j.network;}
  }catch(_){$('nodeNet').textContent='nod: necunoscut';$('nodeNet2').textContent='necunoscut';}
})();
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
  $('keyInfo').innerHTML='Cont <code>'+esc(r.json.account)+'</code> · wallet <code>'+esc(r.json.wallet)+'</code><br>key_id <code>'+esc(r.json.key_id)+'</code> · cotă '+r.json.quota_ceiling+' · '+r.json.rate_limit_per_minute+'/min · start '+(r.json.starter_granted?r.json.starter_quota+' (grant)':'0 (deja alimentat)')+'<br>scope-uri <code>'+esc((r.json.scopes||[]).join(', '))+'</code> (embeddings + compute + memorie proprie; orchestrare/hub rămân pe admin)';
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
function copyMsg(){if(S.manChal)navigator.clipboard.writeText(S.manChal.message).then(()=>say('out2','Mesaj copiat — semnează-l exact în wallet și lipește semnătura.','ok'));else say('out2','Cere întâi mesajul (pasul 1).','err');}
// Documented SignableMessage shape (sdk-core, cached import). The docs
// pass `new SignableMessage({message})` — providers read the documented
// field; raw {data} stays as fallback.
let _coreMod=null;
async function signableMessage(bytes){
  try{
    if(!_coreMod)_coreMod=await import('/*__MX_CORE_URL__*/');
    const SM=_coreMod.SignableMessage||(_coreMod.default&&_coreMod.default.SignableMessage);
    if(typeof SM==='function')return new SM({message:bytes});
  }catch(_){}
  return null;
}
async function providerSign(p,message){
  const bytes=msgBytes(message);
  if(typeof p.signMessage!=='function')throw new Error('provider fără signMessage(). Semnează manual.');
  const doc=await signableMessage(bytes);
  const shapes=[];
  if(doc)shapes.push(doc);
  shapes.push({data:bytes});
  shapes.push(message);
  let last=null;
  for(const shape of shapes){
    try{const sig=extractSig(await p.signMessage(shape,{}));if(sig)return sig;}
    catch(e){last=e;}
  }
  // Some providers mutate the passed object instead of returning.
  if(doc&&doc.signature){const sig=extractSig(doc);if(sig)return sig;}
  throw new Error('semnare eșuată ('+String((last&&last.message)||last).slice(0,120)+'). Încearcă Manual.');
}
function esc(s){return String(s).replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));}
// ---- metoda: DeFi Extension (SDK pin-uit, import dinamic) ----
// Provider-ul e SINGLETON: getInstance(), nu create/new. init() confirmă
// extensia (window.multiversxWallet); login() populează account.address;
// signMessage({data: bytes}) întoarce sdk-core Message.
async function connectExtension(){
  say('out2','Se încarcă provider-ul DeFi…','');
  try{
    const mod=await import('/*__MX_EXTENSION_URL__*/');
    const Provider=mod.ExtensionProvider||(mod.default&&mod.default.ExtensionProvider);
    if(!Provider)throw new Error('SDK încărcat, dar ExtensionProvider lipsește (exports: '+Object.keys(mod).slice(0,8).join(',')+'). Încearcă Manual.');
    const p=typeof Provider.getInstance==='function'?Provider.getInstance():new Provider();
    if(typeof p.init==='function')await p.init();
    if(typeof p.isInitialized==='function'&&!p.isInitialized())throw new Error('Extensia DeFi/MultiversX nu e instalată sau nu e activată pentru site-ul ăsta.');
    if(typeof p.login!=='function')throw new Error('provider fără login(). Încearcă Manual.');
    await p.login();
    const addr=await providerAddress(p);
    if(!addr)throw new Error('login ok, dar adresa lipsește. Încearcă Manual.');
    // Poartă anti-confuzie: extensia poate semna cu alt cont decât cel
    // întors la login (mai multe adrese). Utilizatorul confirmă explicit.
    if(!confirm('Portofel conectat:\n\n'+addr+'\n\nVerifică în extensia DeFi că ACEASTĂ adresă e cea selectată activ. Dacă ai mai multe adrese, selecteaz-o pe aceasta acum.\n\nContinui cu semnarea?')){say('out2','Oprit de tine. Selectează adresa în extensie și reîncearcă — sau Manual.','warn');return;}
    say('out2','Conectat: '+addr+' — cere challenge…','');
    const chal=await getChallenge(addr);
    // Debug vizibil (semnătura e publică prin construcție — ajunge la
    // server oricum): ce i s-a cerut extensiei vs ce a înapoiat.
    S.lastSig=null;
    const m=$('manMsg');m.style.display='block';m.textContent='Mesaj trimis la semnat (byte-cu-byte):\n'+chal.message;
    // Semnare brută: păstrăm obiectul Message întreg pentru tripla
    // verificare de adrese (login vs ecoul semnăturii vs citire curentă).
    const p2=p;
    let signedRaw=null;
    try{signedRaw=await p2.signMessage({data:msgBytes(chal.message)});}
    catch(e1){signedRaw=await p2.signMessage(chal.message);}
    const sig=extractSig(signedRaw);
    if(!sig)throw new Error('semnătură ilizibilă din provider. Încearcă Manual.');
    S.lastSig=sig;
    const echoAddr=addrOf(signedRaw&&(signedRaw.address||(signedRaw||{}).address));
    const curAddr=await providerAddress(p2);
    if(echoAddr&&echoAddr!==addr)m.textContent+='\n\nATENȚIE: extensia a ecouat adresa '+echoAddr+' (login: '+addr+'). Conturi diferite!';
    if(curAddr&&curAddr!==addr)m.textContent+='\n\nATENȚIE: adresa curentă în extensie e '+curAddr+' (login: '+addr+'). Selecteaz-o pe cea de login!';
    await doVerify(addr,chal.challenge_id,sig);
  }catch(e){
    const dbg=S.lastSig?(' [debug: sig_len='+S.lastSig.length+' sig='+S.lastSig+']'):' [debug: fără semnătură]';
    say('out2','Extension: '+String(e.message||e).slice(0,300)+dbg,'err');
  }
}
// ---- metoda: Web Wallet cross-window (popup oficial, fără conturi) ----
async function connectXWindow(){
  say('out2','Se încarcă provider-ul Web Wallet…','');
  try{
    const mod=await import('/*__MX_XWINDOW_URL__*/');
    const XW=mod.CrossWindowProvider
      ||(mod.default&&(mod.default.CrossWindowProvider||mod.default));
    if(typeof XW!=='function'&&typeof XW.getInstance!=='function')throw new Error('SDK încărcat, dar CrossWindowProvider lipsește (exports: '+Object.keys(mod).slice(0,8).join(',')+'). Încearcă Manual.');
    const p=typeof XW.getInstance==='function'?XW.getInstance():new XW();
    if(typeof p.init==='function')await p.init();
    const wurl=S.net==='multiversx-mainnet'?'https://wallet.multiversx.com':'https://testnet-wallet.multiversx.com';
    if(typeof p.setWalletUrl==='function')p.setWalletUrl(wurl);
    say('out2','Se deschide Web Wallet-ul oficial ('+wurl+') — autentifică-te acolo…','');
    const loginOut=await p.login();
    const addr=(typeof loginOut==='string'&&loginOut)||await providerAddress(p);
    if(!addr)throw new Error('login ok, dar adresa lipsește. Încearcă Manual.');
    say('out2','Conectat: '+addr+' — cere challenge…','');
    const chal=await getChallenge(addr);
    const sig=await providerSign(p,chal.message);
    if(!sig)throw new Error('semnătură ilizibilă din provider. Încearcă Manual.');
    await doVerify(addr,chal.challenge_id,sig);
  }catch(e){say('out2','Web Wallet: '+String(e.message||e).slice(0,300),'err');}
}
function msgBytes(s){return new TextEncoder().encode(s);}
function addrOf(a){
  if(!a)return null;
  if(typeof a==='string')return a;
  try{if(typeof a.bech32==='function')return a.bech32();}catch(_){}
  try{const s=String(a);if(s.startsWith('erd1'))return s;}catch(_){}
  return null;
}
function extractSig(signed){
  if(!signed)return null;
  if(typeof signed==='string')return signed;
  const s=signed.signature||signed;
  if(!s)return null;
  if(typeof s==='string')return s;
  // Buffer/Uint8Array (sdk-core Message.signature) → hex
  if(typeof s.length==='number'&&typeof s.toString==='function'){
    try{const h=s.toString('hex');if(/^[0-9a-fA-F]{128}$/.test(h))return h;}catch(_){}
    try{let h='';for(let i=0;i<s.length;i++)h+=s[i].toString(16).padStart(2,'0');if(/^[0-9a-fA-F]{128}$/.test(h))return h;}catch(_){}
  }
  if(s&&typeof s.hex==='function')try{return s.hex();}catch(_){}
  return null;
}
async function providerAddress(p){
  if(!p)return null;
  if(p.account&&p.account.address)return p.account.address;
  if(typeof p.getAddress==='function'){try{const a=await p.getAddress();if(a)return a;}catch(_){}}
  if(typeof p.address==='string'&&p.address)return p.address;
  if(Array.isArray(p.accounts)&&p.accounts[0])return p.accounts[0];
  if(typeof p.getAccount==='function'){try{const a=await p.getAccount();if(a&&(a.address||typeof a==='string'))return a.address||a;}catch(_){}}
  return null;
}
// ---- metoda: xPortal (aplicație) — ghidare first-party, fără relay extern ----
function showXportal(){$('xpoBox').classList.remove('hidden');say('out2','xPortal: vezi caseta de mai sus — fără conturi externe, fără QR extern.','');}
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
            "Web Wallet (popup)",
            "xPortal (aplicație)",
            "Manual / alt wallet",
            "/v1/auth/wallet/challenge",
            "/v1/auth/wallet/verify",
            "/v1/auth/wallet/key",
            "/v1/auth/wallet/network",
            "getInstance",
            "signMessage({data",
            "xPortal (aplicație)",
            "showXportal",
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
            html.contains(MX_XWINDOW_PROVIDER_URL),
            "cross-window provider URL must be pinned"
        );
        assert!(
            html.contains(MX_CORE_URL),
            "sdk-core URL must be pinned"
        );
        assert!(
            !html.contains("/*__MX_EXTENSION_URL__*/"),
            "URL placeholders must be substituted"
        );
        for bad in ["dsk_", "BEGIN PRIVATE", "seed"] {
            assert!(!html.contains(bad), "template must not contain: {bad}");
        }
        // First-party only: no WalletConnect/QR/external-relay references.
        for bad in ["wallet-connect", "walletconnect", "qrcode", "wss://relay"] {
            assert!(!html.to_lowercase().contains(bad), "must stay first-party: {bad}");
        }
    }

    #[test]
    fn provider_urls_are_pinned_versions() {
        // No @latest / floating tags: reproducible client, no supply-chain drift.
        // First-party MultiversX only.
        for url in [
            MX_EXTENSION_PROVIDER_URL,
            MX_XWINDOW_PROVIDER_URL,
            MX_CORE_URL,
        ] {
            assert!(url.starts_with("https://cdn.jsdelivr.net/npm/@multiversx/"));
            assert!(url.ends_with("/+esm"));
            assert!(!url.contains("@latest"), "must pin exact version: {url}");
        }
    }
}
