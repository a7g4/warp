# WARP Aggregation & Redundancy Protocol

- [WARP Architecture](docs/ARCHITECTURE.md)

## Quickstart - Build

```
cargo build --release
```

The binaries `warp`, `warp-keygen`, `warp-print-example-config` and `warp-map` will be built to `target/release`.

## Quickstart - Usage

1. Generate a public/private keypair:

```
warp-keygen
```
2. Generate a config:

```
warp-print-example-config > config
```

3. Modify `config`:

> Replace `private_key` with the private key from `warp-keygen`

> Add/remove patterns to the `exclusion_patterns` list in the `[interfaces]` section

> Set the appropriate address and public key for the warp map in the `[warp_map]` section

> Set the appropriate public key for the peer to establish tunnels with in the `[far_gate]` section

> Set up [tunnels.*] as required

Run warp:

```
warp config
```
