//! mDNS-based worker discovery on LAN

use std::collections::HashMap;
use tokio::sync::mpsc;
use mdns_sd::{ServiceDaemon, ServiceInfo, IfKind};
use tracing::{info, warn, error};

use crate::{WorkerAnnouncement, WorkerResources};

/// mDNS service type for DecentraAI workers
const SERVICE_TYPE: &str = "_decentraai._tcp.local.";

/// mDNS discovery service
pub struct MdnsDiscovery {
    daemon: ServiceDaemon,
    service_name: String,
}

impl MdnsDiscovery {
    pub fn new() -> Self {
        let daemon = ServiceDaemon::new().expect("Failed to create mDNS daemon");
        Self {
            daemon,
            service_name: "decentraai-worker".to_string(),
        }
    }

    /// Start mDNS listener and return stream of discovered workers
    pub async fn start(&self) -> anyhow::Result<mpsc::UnboundedReceiver<WorkerAnnouncement>> {
        let (tx, rx) = mpsc::unbounded_channel();

        // Browse for services
        self.daemon.browse(SERVICE_TYPE)?;

        // Listen for events
        let receiver = self.daemon.monitor()?;
        let tx_clone = tx.clone();

        // Spawn listener task
        tokio::spawn(async move {
            use mdns_sd::DaemonEvent;

            while let Ok(event) = receiver.recv_async().await {
                match event {
                    DaemonEvent::ServiceFound(info) => {
                        if let Some(announcement) = parse_service_info(&info) {
                            if tx_clone.send(announcement).is_err() {
                                break;
                            }
                        }
                    }
                    DaemonEvent::ServiceRemoved(info) => {
                        info!("Service removed: {:?}", info);
                    }
                    _ => {}
                }
            }
        });

        Ok(rx)
    }

    /// Register self as mDNS service
    pub fn register_self(
        &self,
        port: u16,
        properties: HashMap<String, String>,
    ) -> anyhow::Result<()> {
        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &self.service_name,
            "decentraai.local.",
            "",
            port,
            Some(properties),
        )?
        .enable_addr_auto();

        self.daemon.register(service_info)?;
        info!("Registered mDNS service: {} on port {}", self.service_name, port);

        Ok(())
    }
}

/// Parse mDNS service info into WorkerAnnouncement
fn parse_service_info(info: &ServiceInfo) -> Option<WorkerAnnouncement> {
    // Extract properties from mDNS
    let peer_id_str = info.get_properties().get("peer_id")?.as_str();
    let peer_id = libp2p::PeerId::try_from_str(peer_id_str).ok()?;

    let cpu_cores = info.get_properties().get("cpu_cores")?.as_str().parse().unwrap_or(4);
    let ram_gb = info.get_properties().get("ram_gb")?.as_str().parse().unwrap_or(8);
    let gpu_vram = info.get_properties().get("gpu_vram_gb")?.as_str().parse().ok();
    let node_name = info.get_properties().get("node_name")?.as_str().to_string();

    let resources = WorkerResources {
        cpu_cores,
        ram_gb,
        gpu_vram_gb: gpu_vram,
        gpu_count: 1,
        bandwidth_mbps: 1000,
        disk_available_gb: 100,
    };

    let mut announcement = WorkerAnnouncement::new(peer_id, resources, node_name);
    announcement.status = WorkerStatus::Pending; // New workers start as pending

    Some(announcement)
}
