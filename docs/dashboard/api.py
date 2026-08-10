"""
FastAPI backend for worker discovery dashboard

This provides the API endpoints for the /workers dashboard page.
Integrate with your existing node-cli or dashboard server.
"""

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
from typing import List, Optional
import uvicorn

app = FastAPI()

# In-memory store - replace with actual discovery service
pending_workers = {}


class WorkerResources(BaseModel):
    cpu_cores: int
    ram_gb: int
    gpu_vram_gb: Optional[int] = None
    gpu_count: int = 1
    bandwidth_mbps: int = 1000
    disk_available_gb: int = 100


class WorkerAnnouncement(BaseModel):
    peer_id: str
    node_name: str
    resources: WorkerResources
    status: str = "pending"
    loaded_models: List[str] = []
    timestamp: int


class WorkerApproval(BaseModel):
    worker_peer_id: str
    approver_peer_id: str
    signature: str
    approved_at: int
    status: str = "approved"


@app.get("/api/workers")
async def list_workers() -> List[WorkerAnnouncement]:
    """List all pending workers for dashboard"""
    return list(pending_workers.values())


@app.post("/api/workers/{peer_id}/approve")
async def approve_worker(peer_id: str):
    """Approve a worker"""
    if peer_id not in pending_workers:
        raise HTTPException(status_code=404, detail="Worker not found")

    worker = pending_workers[peer_id]
    worker.status = "active"

    # TODO: Call discovery_service.approve_worker(peer_id)
    # TODO: Sign approval with identity
    # TODO: Add to trust records

    return {"status": "approved", "peer_id": peer_id}


@app.post("/api/workers/{peer_id}/reject")
async def reject_worker(peer_id: str):
    """Reject a worker"""
    if peer_id not in pending_workers:
        raise HTTPException(status_code=404, detail="Worker not found")

    del pending_workers[peer_id]

    # TODO: Call discovery_service.reject_worker(peer_id)

    return {"status": "rejected", "peer_id": peer_id}


# Example integration with discovery service
def integrate_discovery_service():
    """
    Example of how to integrate with the Rust discovery service:
    
    from decentraai_discovery import DiscoveryService
    
    discovery = DiscoveryService()
    
    @app.get("/api/workers")
    async def list_workers():
        workers = discovery.get_pending_workers()
        return [serialize_worker(w) for w in workers]
    
    @app.post("/api/workers/{peer_id}/approve")
    async def approve_worker(peer_id: str):
        approval = discovery.approve_worker(peer_id)
        return {"status": "approved"}
    """
    pass


if __name__ == "__main__":
    # Example worker for testing
    pending_workers["16Uiu8gKvJDuLEqNvJ9qNvJ9qNvJ9qNvJ9qNvJ9q"] = WorkerAnnouncement(
        peer_id="16Uiu8gKvJDuLEqNvJ9qNvJ9qNvJ9qNvJ9qNvJ9q",
        node_name="worker-1",
        resources=WorkerResources(
            cpu_cores=8,
            ram_gb=32,
            gpu_vram_gb=24,
            gpu_count=1,
            bandwidth_mbps=1000,
            disk_available_gb=500
        ),
        timestamp=1234567890
    )

    uvicorn.run(app, host="0.0.0.0", port=8000)
