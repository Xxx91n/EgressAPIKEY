# ADR-0024: Delete Legacy LaneCount / SharedGateway Dead Code

Status: ACCEPTED
**Date**: 2026-08-09

## Context

"出口通道数" (laneCount) is a legacy dead field. ADR-0012 route correction
moved data path to Resin sidecar. SharedGateway/GatewayState not in data path.
Resin uses P2C + TD-EWMA, no lanes concept. Changing laneCount has zero effect.

## Decision

Delete:
1. laneCount control from SettingsView.tsx
2. laneCount field from appStore.ts (and setLaneCount)
3. SharedGateway / GatewayState if no other callers
4. AI_API_ROUTE_LANES env var read in main.rs
5. build_shared_gateway function in lib.rs
6. Related tests