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

In this figure, red boxes represent components running on one computer; green boxes represent components running on
a separate computer.

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
establishing warp tunnels with.