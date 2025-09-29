use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::time::Instant;

pub struct ClientStore {
    client_expiry: std::time::Duration,
    // TODO: Replace this with a HashMap (PublicKey doesn't implement Hash, so need to wrap that)
    pubkey_to_addresses: BTreeMap<warp_protocol::PublicKey, HashSet<SocketAddr>>,
    address_to_pubkey: HashMap<SocketAddr, warp_protocol::PublicKey>,
    address_last_seen: HashMap<SocketAddr, Instant>,
}

impl ClientStore {
    pub fn new(client_expiry: std::time::Duration) -> Self {
        Self {
            client_expiry,
            pubkey_to_addresses: BTreeMap::new(),
            address_to_pubkey: HashMap::new(),
            address_last_seen: HashMap::new(),
        }
    }

    pub fn register_client(&mut self, pubkey: warp_protocol::PublicKey, address: SocketAddr, now: Instant) {
        // Clean up old mapping if address was associated with different pubkey
        if let Some(old_pubkey) = self.address_to_pubkey.get(&address) {
            if *old_pubkey != pubkey {
                if let Some(addresses) = self.pubkey_to_addresses.get_mut(old_pubkey) {
                    addresses.remove(&address);
                    if addresses.is_empty() {
                        self.pubkey_to_addresses.remove(old_pubkey);
                    }
                }
            }
        }

        // Insert into set (automatically handles duplicates)
        self.pubkey_to_addresses.entry(pubkey).or_default().insert(address);

        self.address_to_pubkey.insert(address, pubkey);
        self.address_last_seen.insert(address, now);
    }

    pub fn deregister_client(&mut self, pubkey: &warp_protocol::PublicKey, address: SocketAddr) -> bool {
        let mut removed = false;

        // Remove the specific address from the pubkey's address set
        if let Some(addresses) = self.pubkey_to_addresses.get_mut(pubkey) {
            if addresses.remove(&address) {
                removed = true;

                // If this was the last address for this pubkey, remove the pubkey entry
                if addresses.is_empty() {
                    self.pubkey_to_addresses.remove(pubkey);
                }
            }
        }

        // Clean up reverse mappings
        if removed {
            self.address_to_pubkey.remove(&address);
            self.address_last_seen.remove(&address);
        }

        removed
    }

    pub fn get_addresses(&self, pubkey: &warp_protocol::PublicKey, now: Instant) -> Vec<SocketAddr> {
        self.pubkey_to_addresses
            .get(pubkey)
            .map(|addresses| {
                addresses
                    .iter()
                    .filter(|&&addr| {
                        self.address_last_seen
                            .get(&addr)
                            .map(|&last_seen| now.duration_since(last_seen) < self.client_expiry) // Changed <= to <
                            .unwrap_or(false)
                    })
                    .copied()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get_pubkey(&self, address: &SocketAddr) -> Option<warp_protocol::PublicKey> {
        self.address_to_pubkey.get(address).copied()
    }

    pub fn garbage_collect(&mut self, now: Instant) {
        let _span = tracing::span!(tracing::Level::INFO, "garbage collection").entered();

        let mut expired_addresses = 0;
        let mut expired_pubkeys = 0;

        self.address_last_seen.retain(|&addr, &mut last_seen| {
            let expired = now.duration_since(last_seen) >= self.client_expiry;
            if expired {
                expired_addresses += 1;
                // Clean up reverse mapping with O(1) HashSet removal
                if let Some(pubkey) = self.address_to_pubkey.remove(&addr) {
                    if let Some(addresses) = self.pubkey_to_addresses.get_mut(&pubkey) {
                        addresses.remove(&addr); // O(1) instead of O(n)
                        if addresses.is_empty() {
                            self.pubkey_to_addresses.remove(&pubkey);
                            expired_pubkeys += 1;
                        }
                    }
                }
            }
            !expired
        });

        tracing::event!(
            tracing::Level::INFO,
            expired_addresses,
            expired_public_keys = expired_pubkeys
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use std::time::Duration;

    // Helper functions for creating test data
    fn create_test_pubkey(seed: u8) -> warp_protocol::PublicKey {
        let mut bytes = [1u8; 32];
        bytes[0] = seed;
        let secret_key = warp_protocol::PrivateKey::from_bytes(&bytes.into()).unwrap();
        secret_key.public_key()
    }

    fn create_test_address(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
    }

    fn create_test_store() -> ClientStore {
        ClientStore::new(Duration::from_secs(60))
    }

    #[test]
    fn test_new_client_store() {
        let store = create_test_store();
        assert_eq!(store.client_expiry, Duration::from_secs(60));
        assert!(store.pubkey_to_addresses.is_empty());
        assert!(store.address_to_pubkey.is_empty());
        assert!(store.address_last_seen.is_empty());
    }

    #[test]
    fn test_register_single_client() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let address = create_test_address(8080);
        let now = Instant::now();

        store.register_client(pubkey, address, now);

        // Check all data structures are updated
        assert_eq!(store.pubkey_to_addresses.len(), 1);
        assert_eq!(store.address_to_pubkey.len(), 1);
        assert_eq!(store.address_last_seen.len(), 1);

        // Check correct mappings
        assert!(store.pubkey_to_addresses.get(&pubkey).unwrap().contains(&address));
        assert_eq!(store.address_to_pubkey.get(&address), Some(&pubkey));
        assert_eq!(store.address_last_seen.get(&address), Some(&now));
    }

    #[test]
    fn test_register_multiple_addresses_same_pubkey() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let addr1 = create_test_address(8080);
        let addr2 = create_test_address(8081);
        let now = Instant::now();

        store.register_client(pubkey, addr1, now);
        store.register_client(pubkey, addr2, now);

        let addresses = store.pubkey_to_addresses.get(&pubkey).unwrap();
        assert_eq!(addresses.len(), 2);
        assert!(addresses.contains(&addr1));
        assert!(addresses.contains(&addr2));
    }

    #[test]
    fn test_register_duplicate_address_same_pubkey() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let address = create_test_address(8080);
        let now = Instant::now();

        store.register_client(pubkey, address, now);
        store.register_client(pubkey, address, now);

        // Should only have one entry
        let addresses = store.pubkey_to_addresses.get(&pubkey).unwrap();
        assert_eq!(addresses.len(), 1);
        assert!(addresses.contains(&address));
    }

    #[test]
    fn test_register_same_address_different_pubkeys() {
        let mut store = create_test_store();
        let pubkey1 = create_test_pubkey(1);
        let pubkey2 = create_test_pubkey(2);
        let address = create_test_address(8080);
        let now = Instant::now();

        store.register_client(pubkey1, address, now);
        store.register_client(pubkey2, address, now);

        // Address should be removed from first pubkey and added to second
        assert!(store.pubkey_to_addresses.get(&pubkey1).is_none());
        assert!(store.pubkey_to_addresses.get(&pubkey2).unwrap().contains(&address));
        assert_eq!(store.address_to_pubkey.get(&address), Some(&pubkey2));
    }

    #[test]
    fn test_get_addresses_existing_pubkey() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let addr1 = create_test_address(8080);
        let addr2 = create_test_address(8081);
        let now = Instant::now();

        store.register_client(pubkey, addr1, now);
        store.register_client(pubkey, addr2, now);

        let addresses = store.get_addresses(&pubkey, now);
        assert_eq!(addresses.len(), 2);
        assert!(addresses.contains(&addr1));
        assert!(addresses.contains(&addr2));
    }

    #[test]
    fn test_get_addresses_nonexistent_pubkey() {
        let store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let now = Instant::now();

        let addresses = store.get_addresses(&pubkey, now);
        assert!(addresses.is_empty());
    }

    #[test]
    fn test_get_addresses_filters_expired() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let addr1 = create_test_address(8080);
        let addr2 = create_test_address(8081);
        let now = Instant::now();
        let past = now - Duration::from_secs(120); // 2 minutes ago, beyond expiry

        store.register_client(pubkey, addr1, past); // Expired
        store.register_client(pubkey, addr2, now); // Fresh

        let addresses = store.get_addresses(&pubkey, now);
        assert_eq!(addresses.len(), 1);
        assert!(addresses.contains(&addr2));
        assert!(!addresses.contains(&addr1));
    }

    #[test]
    fn test_get_pubkey_existing_address() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let address = create_test_address(8080);
        let now = Instant::now();

        store.register_client(pubkey, address, now);

        assert_eq!(store.get_pubkey(&address), Some(pubkey));
    }

    #[test]
    fn test_get_pubkey_nonexistent_address() {
        let store = create_test_store();
        let address = create_test_address(8080);

        assert_eq!(store.get_pubkey(&address), None);
    }

    #[test]
    fn test_garbage_collect_removes_expired() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let address = create_test_address(8080);
        let now = Instant::now();
        let past = now - Duration::from_secs(120); // Beyond expiry

        store.register_client(pubkey, address, past);

        // Verify entry exists before GC
        assert!(!store.address_last_seen.is_empty());
        assert!(!store.address_to_pubkey.is_empty());
        assert!(!store.pubkey_to_addresses.is_empty());

        store.garbage_collect(now);

        // Verify all data structures are cleaned up
        assert!(store.address_last_seen.is_empty());
        assert!(store.address_to_pubkey.is_empty());
        assert!(store.pubkey_to_addresses.is_empty());
    }

    #[test]
    fn test_garbage_collect_keeps_fresh_entries() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let address = create_test_address(8080);
        let now = Instant::now();

        store.register_client(pubkey, address, now);
        store.garbage_collect(now);

        // Entry should still exist
        assert_eq!(store.address_last_seen.len(), 1);
        assert_eq!(store.address_to_pubkey.len(), 1);
        assert_eq!(store.pubkey_to_addresses.len(), 1);
    }

    #[test]
    fn test_garbage_collect_partial_cleanup() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let addr1 = create_test_address(8080);
        let addr2 = create_test_address(8081);
        let now = Instant::now();
        let past = now - Duration::from_secs(120);

        store.register_client(pubkey, addr1, past); // Expired
        store.register_client(pubkey, addr2, now); // Fresh

        store.garbage_collect(now);

        // Only expired address should be removed
        assert_eq!(store.address_last_seen.len(), 1);
        assert!(store.address_last_seen.contains_key(&addr2));
        assert!(!store.address_last_seen.contains_key(&addr1));

        // Pubkey should still exist with one address
        let addresses = store.pubkey_to_addresses.get(&pubkey).unwrap();
        assert_eq!(addresses.len(), 1);
        assert!(addresses.contains(&addr2));
    }

    #[test]
    fn test_garbage_collect_removes_empty_pubkey_entries() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let address = create_test_address(8080);
        let now = Instant::now();
        let past = now - Duration::from_secs(120);

        store.register_client(pubkey, address, past);
        store.garbage_collect(now);

        // Pubkey entry should be completely removed
        assert!(!store.pubkey_to_addresses.contains_key(&pubkey));
    }

    #[test]
    fn test_data_consistency_after_operations() {
        let mut store = create_test_store();
        let pubkey1 = create_test_pubkey(1);
        let pubkey2 = create_test_pubkey(2);
        let addr1 = create_test_address(8080);
        let addr2 = create_test_address(8081);
        let now = Instant::now();

        // Register multiple clients
        store.register_client(pubkey1, addr1, now);
        store.register_client(pubkey2, addr2, now);
        store.register_client(pubkey1, addr2, now); // Move addr2 to pubkey1

        // Check consistency
        assert_eq!(store.address_to_pubkey.len(), store.address_last_seen.len());

        let total_addresses: usize = store.pubkey_to_addresses.values().map(|addrs| addrs.len()).sum();
        assert_eq!(total_addresses, store.address_to_pubkey.len());

        // Verify specific mappings
        assert_eq!(store.get_pubkey(&addr1), Some(pubkey1));
        assert_eq!(store.get_pubkey(&addr2), Some(pubkey1));
        assert!(store.pubkey_to_addresses.get(&pubkey2).is_none());
    }

    #[test]
    fn test_expiry_boundary_conditions() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let address = create_test_address(8080);
        let now = Instant::now();
        let exactly_expired = now - Duration::from_secs(60);
        let just_before_expiry = now - Duration::from_secs(59);

        store.register_client(pubkey, address, exactly_expired);

        // Should be filtered out (expired at exactly 60 seconds)
        let addresses = store.get_addresses(&pubkey, now);
        assert!(addresses.is_empty());

        // Update with fresh timestamp
        store.register_client(pubkey, address, just_before_expiry);

        // Should be included (not expired)
        let addresses = store.get_addresses(&pubkey, now);
        assert_eq!(addresses.len(), 1);
    }

    #[test]
    fn test_deregister_client_existing_address() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let address = create_test_address(8080);
        let now = Instant::now();

        // Register first
        store.register_client(pubkey, address, now);
        assert!(store.get_pubkey(&address).is_some());

        // Deregister
        let removed = store.deregister_client(&pubkey, address);
        assert!(removed);

        // Verify complete removal
        assert_eq!(store.get_pubkey(&address), None);
        assert!(store.pubkey_to_addresses.get(&pubkey).is_none());
        assert!(!store.address_last_seen.contains_key(&address));
    }

    #[test]
    fn test_deregister_client_nonexistent_address() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let address = create_test_address(8080);

        let removed = store.deregister_client(&pubkey, address);
        assert!(!removed);
    }

    #[test]
    fn test_deregister_client_partial_addresses() {
        let mut store = create_test_store();
        let pubkey = create_test_pubkey(1);
        let addr1 = create_test_address(8080);
        let addr2 = create_test_address(8081);
        let now = Instant::now();

        // Register multiple addresses for same pubkey
        store.register_client(pubkey, addr1, now);
        store.register_client(pubkey, addr2, now);

        // Deregister one address
        let removed = store.deregister_client(&pubkey, addr1);
        assert!(removed);

        // Verify partial removal
        assert_eq!(store.get_pubkey(&addr1), None);
        assert_eq!(store.get_pubkey(&addr2), Some(pubkey));
        assert!(!store.address_last_seen.contains_key(&addr1));
        assert!(store.address_last_seen.contains_key(&addr2));

        // Pubkey should still exist with remaining address
        let addresses = store.pubkey_to_addresses.get(&pubkey).unwrap();
        assert_eq!(addresses.len(), 1);
        assert!(addresses.contains(&addr2));
        assert!(!addresses.contains(&addr1));
    }

    #[test]
    fn test_deregister_client_wrong_pubkey() {
        let mut store = create_test_store();
        let pubkey1 = create_test_pubkey(1);
        let pubkey2 = create_test_pubkey(2);
        let address = create_test_address(8080);
        let now = Instant::now();

        // Register with pubkey1
        store.register_client(pubkey1, address, now);

        // Try to deregister with wrong pubkey
        let removed = store.deregister_client(&pubkey2, address);
        assert!(!removed);

        // Verify nothing was removed
        assert_eq!(store.get_pubkey(&address), Some(pubkey1));
        assert!(store.pubkey_to_addresses.get(&pubkey1).unwrap().contains(&address));
    }

    #[test]
    fn test_deregister_maintains_data_consistency() {
        let mut store = create_test_store();
        let pubkey1 = create_test_pubkey(1);
        let pubkey2 = create_test_pubkey(2);
        let addr1 = create_test_address(8080);
        let addr2 = create_test_address(8081);
        let addr3 = create_test_address(8082);
        let now = Instant::now();

        // Set up complex scenario
        store.register_client(pubkey1, addr1, now);
        store.register_client(pubkey1, addr2, now);
        store.register_client(pubkey2, addr3, now);

        // Deregister one address from pubkey1
        store.deregister_client(&pubkey1, addr1);

        // Check data consistency
        assert_eq!(store.address_to_pubkey.len(), store.address_last_seen.len());

        let total_addresses: usize = store.pubkey_to_addresses.values().map(|addrs| addrs.len()).sum();
        assert_eq!(total_addresses, store.address_to_pubkey.len());

        // Verify specific state
        assert_eq!(store.get_pubkey(&addr1), None);
        assert_eq!(store.get_pubkey(&addr2), Some(pubkey1));
        assert_eq!(store.get_pubkey(&addr3), Some(pubkey2));
    }
}
