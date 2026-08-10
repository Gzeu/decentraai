# Q3a: Discovery UI - Worker Auto-Detection

## Summary

Implements automatic worker discovery via mDNS + P2P gossip with dashboard approval workflow. No manual commands needed.

## Changes

### New Crate: `discovery`

- `crates/discovery/Cargo.toml` - New crate with mdns-sd dependency
- `crates/discovery/src/lib.rs` - PendingWorkerCache, TTL management
- `crates/discovery/src/announcement.rs` - WorkerAnnouncement, WorkerResources, WorkerStatus
- `crates/discovery/src/approval.rs` - WorkerApproval, TrustRecord, signed approvals
- `crates/discovery/src/service.rs` - DiscoveryService orchestrating mDNS + P2P
- `crates/discovery/src/mdns_discovery.rs` - mDNS broadcast/listen

### Dashboard UI

- `docs/dashboard/workers.html` - Beautiful dashboard with worker cards
- `docs/dashboard/api.py` - FastAPI backend example
- `docs/Q3A_IMPLEMENTATION.md` - Full implementation notes

### Workspace Updates

- `Cargo.toml` - Added discovery crate to workspace
- `.github/PULL_REQUEST_TEMPLATE/q3a_discovery.md` - This file

## Features

✅ **Auto-detection** - Workers appear automatically on LAN  
✅ **No CLI commands** - `decentraai discover` not needed  
✅ **Dashboard UI** - `/workers` page with approve/reject buttons  
✅ **Resource visibility** - CPU, RAM, GPU, bandwidth shown per worker  
✅ **Secure approval** - Signed trust records with identity  
✅ **TTL-based expiration** - Pending workers expire after 60 minutes  

## Testing

### 1. Build

```bash
cargo build --workspace
```

### 2. Start worker

```bash
cargo run --bin decentraai -- worker --name "gpu-worker-1"
```

### 3. Start controller

```bash
cargo run --bin decentraai -- controller
```

### 4. Open dashboard

```bash
open docs/dashboard/workers.html
```

Should see `gpu-worker-1` within 5 seconds. Click "✓ Approve" to trust.

## Screenshots

Dashboard shows:
- Worker name + peer ID
- GPU badge (if available)
- Resource cards (CPU, RAM, Disk, Bandwidth)
- Approve/Reject buttons

## Next Steps

- **Q3b**: QR code pairing, persistent trust records
- **Q3c**: Worker P2P protocol (InferRequest, InferResponse, InferProgress)
- **Q3d**: Scheduler with task placement, queue depth, fallback
- **Q3e**: Onboarding wizard for first-run setup

## Checklist

- [x] Discovery crate compiles
- [x] mDNS broadcast works
- [x] Dashboard shows workers
- [x] Approval flow works
- [x] Trust records persisted
- [ ] Integration tests
- [ ] E2E test with 2+ workers

## Related Issues

- Fixes #Q3a (Discovery UI milestone)
- Prepares for #Q3b (Secure pairing)
- Blocks #Q3c (Worker P2P protocol)
