use anyhow::Result;
use decentraai_identity::Identity;
use decentraai_protocol::{InferMessage, InferRequest};
use decentraai_p2p::{P2PNode, RequestHandler};
use libp2p::PeerId;
use std::sync::Arc;
use tokio::sync::mpsc;
use std::env;

struct IncomingHandler {
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl RequestHandler for IncomingHandler {
    fn handle(&self, request: &[u8]) -> Result<Vec<u8>> {
        // Forward any inbound messages to the channel for processing by the app
        let _ = self.tx.send(request.to_vec());
        // Return empty OK so the swarm marks the request handled; some messages
        // expect no response.
        Ok(Vec::new())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 {
        println!("Usage: p2p-invoke <peer-id> <peer-addr> <model-hash> [prompt]");
        return Ok(());
    }
    let peer_id: PeerId = args[1].parse()?;
    let peer_addr = args[2].clone();
    let model_hash = args[3].clone();
    let prompt = if args.len() > 4 { args[4].clone() } else { "Hello from test".to_string() };

    // Generate ephemeral identity for this client
    let identity = Identity::generate();

    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let handler = IncomingHandler { tx };
    let node = P2PNode::new(&identity, 1024 * 1024, 96 * 1024 * 1024, Some(Arc::new(handler)))?;
    let bound = node.listen("/ip4/0.0.0.0/tcp/0").await?;
    println!("Local peer id: {} listening: {}", node.local_peer_id(), bound);

    // Dial target
    node.dial(&peer_addr).await?;
    println!("Dialed {}", peer_addr);

    // Build InferRequest
    let mut req = InferRequest::new(model_hash, prompt, 64);
    req = req.with_sender(node.local_peer_id());
    req = req.with_streaming(true);

    let payload = decentraai_protocol::serialize_message(&InferMessage::InferRequest(req.clone()))?;

    // Send the request and await immediate response (Accept or final)
    let response_bytes = node.request(peer_id, payload).await?;
    if let Ok(msg) = decentraai_protocol::deserialize_message::<InferMessage>(&response_bytes, response_bytes.len()) {
        println!("Immediate response from worker: {:?}", msg);
    } else {
        println!("Immediate response (raw {} bytes)", response_bytes.len());
    }

    // Listen for incoming messages from worker (progress / final)
    println!("Waiting for progress frames (timeout 30s)...");
    use tokio::time::{timeout, Duration};
    loop {
        match timeout(Duration::from_secs(30), rx.recv()).await {
            Ok(Some(bytes)) => {
                if let Ok(msg) = decentraai_protocol::deserialize_message::<InferMessage>(&bytes, bytes.len()) {
                    println!("Received inbound message: {:?}", msg);
                    match msg {
                        InferMessage::InferProgress(_) => continue,
                        InferMessage::InferResponse(_) | InferMessage::InferFailed {..} => break,
                        _ => continue,
                    }
                } else {
                    println!("Received non-protocol bytes len={}", bytes.len());
                }
            }
            Ok(None) => { println!("Channel closed"); break; }
            Err(_) => { println!("Timed out waiting for progress"); break; }
        }
    }

    Ok(())
}
