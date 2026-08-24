// Live smoke (ruleat manual, nu în CI): adapterul vs devnet-ul real.
use decentraai_economy::multiversx_devnet::{MxDevnetClient, ReqwestTransport};

fn main() {
    let c = MxDevnetClient::devnet(ReqwestTransport::new());
    let agents = c.list_agents(0, 5).expect("list_agents live");
    println!("LIVE list_agents: {} items", agents.len());
    for a in &agents {
        println!("  nonce={:?} name={:?} pk={:?}", a.nonce, a.name,
            a.public_key.as_deref().map(|k| k.get(..12).unwrap_or(k)));
    }
    if let Some(first) = agents.first() {
        if let Some(n) = first.nonce {
            let rep = c.reputation(n).expect("reputation live");
            println!("LIVE reputation nonce={} avg={} count={}", n, rep.average, rep.count);
        }
    }
}
