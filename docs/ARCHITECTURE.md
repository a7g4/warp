# WARP Aggregation & Redundancy Protocol

The "WARP" system is made up of the following pieces:
- warp gates: endpoints for applications to send and receive data from
- warp tunnels: magic transport layer which will deliver application data between warp gates
- warp map: a publicly routable server that facilitates discovery of peers

## Application Abstraction

Warp is built around providing the "warp gate" abstraction. Applications can send and receive data to/from a warp gate
as if it was the remote endpoint that it was designed to communicate with and with no changes to the application, the
data transport will benefit from the aggregation and redundancy of the warp tunnel.

![alt text](warp-tunnel.svg "Figure 1. Warp Tunnel")

<!--- TODO: Update this when warp does something smarter  --->
The warp tunnel will replicate traffic that it receives from the application so that every local interface will send
data to every remote interface.

Warp gates can be configured in two ways:
- Loopback Sockets
- Unix Domain Sockets

### Loopback sockets

Loopback sockets are useful if the application is designed to communicate with a remote endpoint. The only change to the
application will be to change where it sends/receives data to/from to the loopback address of the warp gate
(eg. "127.0.0.1:9000").

### Unix Domain Sockets

Unix Domain Sockets provide some extra feedback to the application based on network congestion.

The `send()` syscall in the application will block until that particular payload has been sent.

## Network Architecture

![alt text](warp-map.svg "Figure 2. Warp Map")

All instances of warp are identified by their public key. While running, warp will periodically update the warp map with
all interfaces that it can use to send & receive as well as querying the warp map for details about the peer it is
establishing warp tunnels with. The frequency of this is controlled by the client's `interface_scan_interval` config.

## NAT Traversal

Warp supports operation through various NAT (Network Address Translation) configurations, including ["symmetric NAT"
(Type 4)](https://en.wikipedia.org/wiki/Network_address_translation#Methods_of_translation).

NOTE: warp **DOES NOT** support establishing tunnels when both sides are hosts with symmetric/Type 4 NATs.

### The Symmetric NAT Problem

With symmetric NAT, the standard peer discovery process fails:

1. **Peer A** (behind symmetric NAT) registers with warp-map → NAT assigns `external_ip:port_X` as the origin port
2. **Peer B** queries warp-map for **Peer A** → receives `external_ip:port_X`
3. **Peer A** sends to **Peer B** → NAT assigns `external_ip:port_Y` as a new origin port (different port from when it
   sent to warp-map)
4. **Peer B** tries to send to **Peer A** at `external_ip:port_X` → traffic rejected; port_X will only accept traffic
   from warp-map

### PeerAddressOverride Solution

Warp solves this using the `PeerAddressOverride` message, which allows peers behind symmetric NAT to inform their peers
of the correct return address:

1. **Peer A** (behind symmetric NAT) registers with warp-map → NAT assigns `external_ip:port_X`
2. **Peer A** learns its external address from warp-map RegisterResponse: `external_ip:port_X`
2. **Peer B** queries warp-map for Peer A → receives `external_ip:port_X`
2. **Peer A** sends `PeerAddressOverride{replace: warp_map_address}` to **Peer B**; NAT assigns `external_ip:port_Y` as
   the origin port
3. **Peer B** receives the override and updates its address mapping: `external_ip:port_X` → `external_ip:port_Y`
4. **Peer B** uses the corrected address (`external_ip:port_Y`) for all future traffic to **Peer A**

