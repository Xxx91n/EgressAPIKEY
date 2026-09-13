# ADR-0023: Node Pool Collapsible Tree View

Status: ACCEPTED
**Date**: 2026-08-09

## Context

NodesView.tsx displays flat table, no subscription grouping, no latency.

## Decision

Adopt clash-verge-dev ProxyGroup pattern:
- Level 1: Subscription (collapsible), sub name + node count + health rate
- Level 2: Node rows with display_tag / region / latency(ms) / health
- Latency color: <200ms green, 200-500ms yellow, >500ms red, timeout gray
- Click row to expand details (egress_ip, failure_count, last_probe_time)
- Search box filters across subscriptions
- 10s auto-refresh (existing)

## Scope
Pure frontend (~150 JSX + tests). NodeItem needs reference_latency_ms added.