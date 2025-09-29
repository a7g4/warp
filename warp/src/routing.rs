pub(crate) struct RoutingState {
    interfaces_tx: tokio::sync::watch::Sender<Vec<std::sync::Arc<crate::interface::NetworkInterface>>>,
    interfaces_watch: tokio::sync::watch::Receiver<Vec<std::sync::Arc<crate::interface::NetworkInterface>>>,

    peer_addresses_tx: tokio::sync::watch::Sender<Vec<std::net::SocketAddr>>,
    peer_addresses_watch: tokio::sync::watch::Receiver<Vec<std::net::SocketAddr>>,

    address_overrides_tx: tokio::sync::watch::Sender<std::collections::HashMap<(String, std::net::SocketAddr), std::net::SocketAddr>>,
    address_overrides_watch: tokio::sync::watch::Receiver<std::collections::HashMap<(String, std::net::SocketAddr), std::net::SocketAddr>>,
}

impl RoutingState {
    /// Create a new PacketRoutingState with empty initial state
    pub fn new() -> Self {
        let (interfaces_tx, interfaces_watch) = tokio::sync::watch::channel(Vec::new());
        let (peer_addresses_tx, peer_addresses_watch) = tokio::sync::watch::channel(Vec::new());
        let (address_overrides_tx, address_overrides_watch) = tokio::sync::watch::channel(std::collections::HashMap::new());
        
        Self {
            interfaces_watch,
            peer_addresses_watch,
            address_overrides_watch,
            interfaces_tx,
            peer_addresses_tx,
            address_overrides_tx,
        }
    }
    
    pub fn interfaces(&self) -> tokio::sync::watch::Ref<'_, Vec<std::sync::Arc<crate::interface::NetworkInterface>>> {
        self.interfaces_watch.borrow()
    }

    /// Update the peer addresses from warp-map
    pub fn handle_mapping_response(&self, mapping: &warp_protocol::messages::MappingResponse) {
        self.peer_addresses_tx.send_replace(mapping.endpoints.clone());
        
        // Clean up stale override mappings - remove overrides for addresses no longer in peer list
        self.address_overrides_tx.send_modify(|overrides| {
            let valid_addresses: std::collections::HashSet<std::net::SocketAddr> =
                mapping.endpoints.iter().copied().collect();
            
            overrides.retain(|(_interface_name, replace_addr), _mapped_addr| {
                let should_keep = valid_addresses.contains(replace_addr);
                if !should_keep {
                    tracing::info!(
                        "Expiring override mapping for {} (no longer in warp-map)",
                        replace_addr
                    );
                }
                should_keep
            });
        });
    }

    /// Apply address overrides to resolve the final destination addresses
    /// 
    /// This method takes the base peer addresses and applies any interface-specific
    /// overrides to handle symmetric NAT scenarios correctly.
    pub fn resolve_peer_addresses(&self, outbound_interface_name: &str) -> Vec<std::net::SocketAddr> {
        let peer_addresses = self.peer_addresses_watch.borrow();
        let address_overrides = self.address_overrides_watch.borrow();
        
        peer_addresses
            .iter()
            .map(|addr| {
                // Look for override specific to this (interface, remote_address) pair
                let override_key = (outbound_interface_name.to_string(), *addr);
                address_overrides.get(&override_key).copied().unwrap_or(*addr)
            })
            .collect()
    }
    
    /// This is used when receiving PeerAddressOverride messages to handle symmetric NAT holepunching
    pub fn handle_peer_address_override(&self, override_msg: &warp_protocol::messages::PeerAddressOverride, from: std::net::SocketAddr, interface_name: &str) {
        self.address_overrides_tx.send_modify(|overrides| {
            let key = (interface_name.to_string(), override_msg.replace);
            let old_mapping = overrides.insert(key.clone(), from);
            
            if let Some(old_address_override) = old_mapping {
                if old_address_override != from {
                    tracing::info!(
                        "Updated override mapping for interface {}: {} -> {} (was {})",
                        interface_name,
                        override_msg.replace,
                        from,
                        old_address_override
                    );
                }
            } else {
                tracing::info!(
                    "New override mapping for interface {}: {} -> {}",
                    interface_name,
                        override_msg.replace,
                        from,
                );
            }
        });
    }
    
    
    /// Get the number of active address overrides (for logging/debugging)
    pub fn active_overrides_count(&self) -> usize {
        self.address_overrides_watch.borrow().len()
    }
    
    /// Get the sender for interfaces (for internal use)
    pub(crate) fn interfaces_sender(&self) -> &tokio::sync::watch::Sender<Vec<std::sync::Arc<crate::interface::NetworkInterface>>> {
        &self.interfaces_tx
    }
}

impl Default for RoutingState {
    fn default() -> Self {
        Self::new()
    }
}