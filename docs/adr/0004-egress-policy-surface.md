# ADR-0004: Egress policy surface = three Resin-native values; document the gap

Date: 2026-08-02
Status: PROPOSED (provisionally resolved from evidence)
Decision Type: Feature scope

## Context

User requirement #3 / item three requested: fully random / sequential / by
latency / by bandwidth / by quality-weight / by protocol-weighted quality.

## Decision

Surface exactly the three Resin-native allocation_policy values:
- BALANCED (shell label: "balanced / quality weight" - Resin's native default)
- PREFER_LOW_LATENCY (shell label: "by latency")
- PREFER_IDLE_IP (shell label: "by idle IP")

Document the gap: the user's asked random / sequential / bandwidth /
protocol-weighted are NOT resembling Resin-native values in v1.1.2.
The shell does NOT implement them as shell-side logic overriding Resin
(such a layer would fight Resin's own selection and break lease stickiness).

## What we DO surface

- ipChannelPolicySet wrapper maps five UI labels to the three Resin values:
  "random" / "sequential" -> BALANCED
  "latency" -> PREFER_LOW_LATENCY
  "quality" -> PREFER_IDLE_IP
  "bandwidth" -> BALANCED (documented limitation)

- docs/PROTOCOL_WEIGHT_RESEARCH.md already documents how the outbound node
  protocol affects AI SSE/WebSocket (http=1.0, ss=0.7, hysteria2=0.1). This
  is advisory reference, NOT a runtime-injected weight.

## What we DO NOT do

- Do not inject a custom shell-side egress selector over Resin's runtime.
- Do not promise protocol-weighted selection; Resin has no such endpoint.
- Do not hide the limitation from the user. The Nodes view shows an
  egressPolicyNote (i18n) pointing the user to the Topology canvas where
  allocation_policy binds; the info card documents the protocol weight table.

## Consequences

- Users with a hard requirement for true random / sequential / bandwidth must
  fork Resin to add those policies, or wait for a future Resin release.
- The shell reporting is honest: the GUI exposes what Resin actually supports,
  nothing synthetic.
