# Q3a: Discovery UI Implementation

## Overview

Q3a implements automatic worker discovery via mDNS (LAN) and P2P gossip, with approval workflow from dashboard.

## Features

✅ **Auto-detection** - Workers discovered automatically on LAN  
✅ **No manual commands** - `decentraai discover` not needed  
✅ **Dashboard UI** - `/workers` page with approve/reject  
✅ **Secure pairing** - Signed trust records  
✅ **Resource visibility** - CPU, RAM, GPU, bandwidth shown  

## Architecture

```
┌─────────────────┐      mDNS       ┌─────────────────┐
│   Worker Node   │◄───────────────►│  Controller Node │
│                 │      P2P        │                  │
│ - SystemProbe   │◄───────────────►│ - DiscoverySvc   │
│ - Announcement  │                 │ - Dashboard      │
└─────────────────┘                 │ - TrustRecords   │
                                    └─────────────────┘
```

## Usage

### 1. Start worker node

```bash
cargo run --bin decentraai -- worker --name "my-gpu-worker"
```

Worker automatically:
- Probes system (CPU, RAM, GPU)
- Broadcasts `WorkerAnnouncement` via mDNS + P2P
- Waits for approval

### 2. Open dashboard

```bash
# If running dashboard server
open http://localhost:3000/workers
```

Dashboard shows:
- All pending workers on LAN
- Resources (CPU cores, RAM, GPU VRAM)
- Approve/Reject buttons

### 3. Approve worker

Click "✓ Approve" in dashboard. Worker becomes:
- Trusted (added to trust records)
- Active (can receive inference requests)
- Scheduled (included in task placement)

## API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/workers` | GET | List pending workers |
| `/api/workers/:peer_id/approve` | POST | Approve worker |
| `/api/workers/:peer_id/reject` | POST | Reject worker |

## Integration

### Rust

```rust
use discovery::{DiscoveryService, DiscoveryConfig};

let config = DiscoveryConfig::default();
let mut discovery = DiscoveryService::new(config, identity, p2p_service);

// Start discovery (blocks)
discovery.start().await?;

// Or get pending workers for dashboard
let pending = discovery.get_pending_workers();
for worker in pending {
    println!("{} - {} CPU, {}GB RAM", 
        worker.node_name,
        worker.resources.cpu_cores,
        worker.resources.ram_gb
    );
}

// Approve worker
let approval = discovery.approve_worker(&worker_peer_id)?;
```

### Python (FastAPI)

See `docs/dashboard/api.py` for FastAPI integration example.

## Next Steps

- **Q3b**: Pairing securizat (QR codes, trust persistence)
- **Q3c**: Worker P2P protocol (InferRequest, InferResponse)
- **Q3d**: Scheduler (task placement, queue depth, fallback)
- **Q3e**: Onboarding wizard (first-run setup)

## Testing

### Test mDNS discovery

```bash
# Terminal 1 - worker
cargo run --bin decentraai -- worker --name "test-worker-1"

# Terminal 2 - controller
cargo run --bin decentraai -- controller

# Open dashboard
open docs/dashboard/workers.html
```

Should see `test-worker-1` in dashboard within 5 seconds.

### Test approval flow

1. Click "✓ Approve" in dashboard
2. Check logs: `Worker approved: 16Uiu8gK...`
3. Worker status changes to "Active"
4. Worker can now receive inference requests

## Troubleshooting

### Workers not appearing

- Check firewall allows mDNS (port 5353 UDP)
- Verify both nodes on same LAN
- Check logs: `RUST_LOG=discovery=debug cargo run`

### Approval fails

- Verify identity keys loaded
- Check signature verification
- Ensure trust records can be written

## Security Notes

- Workers are **pending** by default (not trusted)
- Approval requires signed message from controller identity
- Trust records persisted to disk
- Rejected workers cannot re-announce (TTL expires)
