# ADR-0018: Read-only IPC auto-retry (ResinClient layer)

Date: 2026-08-06
Status: ACCEPTED
Context: 43 IPC commands, 0 retry, 0 backoff, 0 circuit breaker. ResinClient has 3 reqwest timeouts but no retry. Frontend polling breaks on transient failures.

## Decision

Add bounded auto-retry (2 attempts, 500ms interval) for read-only ResinClient methods only (list_platforms, get_platform, list_subscriptions, node_pool_snapshot, active_leases, list_nodes, list_endpoints). Write methods (create/update/delete) get NO retry to avoid duplicate mutations.

Retry lives in ResinClient, not in the IPC layer or frontend. This keeps the retry logic in one place and the IPC commands stay thin forwarders.

## Rationale

Frontend polls read endpoints on 5s intervals to keep Topology/Nodes/Platforms tabs live. A single transient failure (Resin momentarily busy, GC pause, connection reset) causes a blank render or stale state. Two fast retries cover the common transient case without delaying the UI beyond 1s.

Write methods do not retry because a failed POST/DELETE may have partially succeeded on the Resin side; retrying would duplicate the effect.

The existing patchingRef reentry lock in TopologyView already protects write-path topology edits from double-PATCH.

## Alternatives rejected

- (A) No retry: transient failures cause UI flash, inconsistent with Q3 sidecar crash recovery.
- (C) Full resilience layer (circuit breaker + global throttle): over-engineered for a local loopback sidecar. Resin runs on 127.0.0.1; network jitter is not the primary failure mode.

## Tradeoffs

- +30 lines in resin_client.rs (retry wrapper for GET methods)
- Max added latency per failed read: 1s (2 x 500ms)
- No new crate dependency (hand-rolled retry loop)
