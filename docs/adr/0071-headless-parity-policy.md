# ADR-0071: Headless management-plane parity - same-SPA default with documented curation

Status: ACCEPTED (2026-09-19, round11-grill decision D-003)
> Extends (reopens none): ADR-0043 (headless server build separation), ADR-0049 (headless SPA fallback), round8-grill D-003 (headless security guard).
> Research basis: atomcode (source=atomcode-q3-vps-mgmt) - self-hosted Web UI as the dominant remote-management mental model (Proxmox, Cockpit, Portainer, Pangolin, yacd/metacubexd); curated-subset sustainability evidence (Kubernetes Dashboard deprecation, mihomo panel drift); trust-boundary convergence (token bootstrap + Host/Origin allowlist + reverse-proxy TLS).

## Context

The headless BFF maps 35 of 79 IPC commands to HTTP with 44 commands explicitly disabled, and the /api/v1/ports/{port} write routes are runtime-unreachable (axum 0.7 literal-path bug, round10 backlog P1). The documented "same control surface" promise is therefore not met. Desktop and headless share one React SPA, so UI-level parity is nearly free; the real curation surface is the command-to-route mapping layer.

## Decision

D1 - Form factor: the browser UI served by the headless BFF is the only first-class remote management surface this round. A remote-managing desktop client is deferred behind three trigger conditions (restricted-network environments proven significant; multi-node single-screen aggregation demand; a desktop-exclusive capability demanded remotely). API-first is rejected.
D2 - Parity policy: same-SPA default. Every function reachable in the SPA MUST be reachable headless. A command may be disabled ONLY when it assumes a local desktop environment (tray, OS proxy settings, and the like); pure control-plane state commands have no excuse to be absent from the HTTP surface. The current 44-entry DISABLED_COMMANDS list is re-adjudicated under this criterion.
D3 - Machine-readable capabilities: GET /api/v1/capabilities (BFF-native endpoint, no IPC manifest change) exposes enabled/disabled per command with reason; the UI renders disabled commands as desktop-only with a documentation link instead of a runtime throw.
D4 - Security micro-hardening: X-Forwarded-Proto scheme detection and CSP frame-ancestors none join the existing security guard; zero-trust networking (Tailscale/WireGuard) is documented as a recommended, not required, deployment option in the headless deployment guide.

## Consequences

- The round11 headless completion ticket (R11-03) implements D2/D3/D4 including the ports-route fix; R11-04 performs the preparatory ipc.ts boundary split first (D-005 5b).
- The trust boundary is unchanged: token bootstrap + Host/Origin allowlist + reverse-proxy TLS remains the boundary (validated as the industry convergence point).
