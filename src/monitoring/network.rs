//! Network interface throughput.

use sysinfo::Networks;

use super::per_second;
use crate::core::models::{InterfaceSnapshot, NetworkSnapshot};

pub struct NetworkSampler {
    networks: Networks,
}

impl NetworkSampler {
    pub fn new() -> Option<Self> {
        Some(Self {
            networks: Networks::new_with_refreshed_list().ok()?,
        })
    }

    pub fn sample(&mut self, elapsed_secs: f64) -> NetworkSnapshot {
        // `true` drops interfaces that disappeared (e.g. a VPN disconnecting).
        self.networks.refresh(true);
        let mut snapshot = NetworkSnapshot::default();
        for (name, data) in &self.networks {
            let iface = InterfaceSnapshot {
                name: name.to_string_lossy().into_owned(),
                received_per_sec: per_second(data.received(), elapsed_secs),
                transmitted_per_sec: per_second(data.transmitted(), elapsed_secs),
                total_received: data.total_received(),
                total_transmitted: data.total_transmitted(),
                mac_address: Some(data.mac_address())
                    .filter(|m| !m.is_unspecified())
                    .map(|m| m.to_string()),
                addresses: data
                    .ip_networks()
                    .iter()
                    .map(|n| format!("{}/{}", n.addr, n.prefix))
                    .collect(),
                link_speed_bps: data
                    .receive_link_speed()
                    .or(data.transmit_link_speed())
                    .filter(|&s| s > 0),
            };
            // Interfaces that have never carried traffic are noise in the UI.
            if iface.total_received == 0 && iface.total_transmitted == 0 {
                continue;
            }
            snapshot.received_per_sec += iface.received_per_sec;
            snapshot.transmitted_per_sec += iface.transmitted_per_sec;
            snapshot.interfaces.push(iface);
        }
        snapshot
            .interfaces
            .sort_by_key(|i| std::cmp::Reverse(i.total_received));
        snapshot
    }
}
