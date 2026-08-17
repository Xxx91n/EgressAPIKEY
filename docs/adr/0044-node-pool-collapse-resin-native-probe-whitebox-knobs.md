# ADR-0044: Node pool default-collapse + Resin-native probe + whitebox probe knobs

Date: 2026-08-17
Status: ACCEPTED
Canon: GRILL_T19_NODE_POOL_FIX_PLAN.md (5-phase execution)

## Context

Three pain points surfaced on the NodesView (node pool page):

1. **Default-expand floods the page**. With 53-node subscription the page shows all leaf cards immediately; users have to manually collapse groups to focus.
2. **Subscription does not update** even after restarting the GUI. The shell uses Resin `source_type:"local"` subscriptions (content inlined at POST time); Resin own scheduler only re-parses the in-memory content for local source. Real remote re-fetch never happens.
3. **No per-node ping** like clash-verge-rev delay card; users cannot tell which node is alive without launching traffic. clash-verge #979 confirms the community pain point in the inverse direction: auto-batch testing on load is also a complaint, so the right default is manual single-probe + opt-in group batch.

Resin v1.2.0 ProbeManager ships `POST /api/v1/nodes/{hash}/actions/probe-egress` and `/actions/probe-latency` (sync handlers `ProbeEgressSync`/`ProbeLatencySync`) — proven endpoints returning exit IP + EWMA latency. Default Resin knobs (research: atomcode-probe-params):
- `RESIN_PROBE_CONCURRENCY` = 1000 (worker pool; env-only)
- `RESIN_PROBE_TIMEOUT` = 15s (env-only)
- `max_consecutive_failures` = 3 (hot-update via `/system/config`)
- `max_latency_test_interval` = 1h / authority = 3h (hot-update)
- `max_egress_test_interval` = 24h (hot-update)
- `latency_test_url` = `https://www.gstatic.com/generate_204` (hot-update)
- `latency_authorities` = [gstatic.com, google.com, cloudflare.com, github.com] (hot-update)
- No exponential backoff on the probe path

clash-verge-rev `delayManager` (research: atomcode-probe-params):
- hard cap `actualConcurrency = Math.min(concurrency, names.length, 10)` — hardcoded 10
- `DEFAULT_DELAY_TIMEOUT = 10000` ms (verge.yaml `default_latency_timeout` override)
- cache TTL 30 min; local hard timeout via `Promise.race`
- default test URL `http://cp.cloudflare.com/generate_204`

## Decision

**S1 — Default collapse + hide-unhealthy + 3-state latency sort + delay> filter syntax**. Seed `collapsed` Set with every subscription name after the first refresh; toggle `hideUnhealthy` excludes `!isHealthy` rows; sort cycles default to latency-asc to latency-desc on the same `reference_latency_ms` already used for card coloring; `parseDelayQuery` recognizes `delay>N` / `delay<N` / `delay=timeout` / `delay=error` so users can type clash-verge-style filter strings into the existing search box.

**S2 — Shell re-fetch remote Clash YAML + PATCH subscription content** for local-source subscriptions. Reuse existing `fetch_clash_subscription` (clash-family UA rotation — many providers 403 a default UA but 200 to clash-verge UA) and `clash_yaml_to_proxies_block` state machine (flow-style to block-style rewrite, drop groups/rules/dns, quote-escaped-comma splitting). New `ResinClient::refresh_subscription_content(id, content)` PATCHes `/api/v1/subscriptions/{id}`. New `subscription_refresh(name)` IPC + per-group refresh button. After PATCH the canvas `refresh()` poll mirrors the new node list.

**S3 — Wire Resin native `probe-egress` and `probe-latency`** through new `ResinClient` methods + `node_probe(node_hash, kind)` IPC + per-card test button. No probe logic in the shell; Resin own EWMA stays the authoritative latency for routing decisions.

**S4 — Whitebox `settings.json#nodeProbe` knobs + Resin `/system/config` plumbing** with defaults from industry sources:
- `nodeProbe.concurrency` = 10 (clash-verge hard cap)
- `nodeProbe.timeout_ms` = 10000 (clash-verge `DEFAULT_DELAY_TIMEOUT`); shell takes min(10s, Resin 15s) = 10s
- `nodeProbe.test_url` = `https://www.gstatic.com/generate_204` (Resin default)
- `nodeProbe.batch_on_load` = false (default manual (甲); auto-batch is clash-verge #979 pain point)
- Defaults: manual single-probe (甲) + per-group batch test button (丙) with concurrency gate

Resin hot-update knobs (`max_consecutive_failures`, `max_latency_test_interval`, `max_authority_latency_test_interval`, `max_egress_test_interval`, `latency_test_url`, `latency_authorities`, `p2c_latency_window`, `latency_decay_window`) exposed via new Settings panel that calls `system_config_get` / `system_config_patch`.

## Rejected alternatives

1. **Virtual scrolling** for node cards — no industry precedent (clash-verge-rev/metacubexd/v2rayN/surfboard/Karing all use group-collapse + filter + hide-unhealthy + latency-sort, not virtual rows). Our `VIRTUAL_THRESHOLD=50` already exists as a fallback.
2. **Auto-batch probe on first load** — clash-verge #979 confirms community rejects; manual is the polite default. `batch_on_load=false` defaults.
3. **Shell-side TCP ping tool** — would diverge from Resin routing-truth EWMA. `probe_node_egress` returns Resin own EWMA so the shell stays consistent with routing decisions.
4. **Fork Resin Go to add new probes/knobs** — `probe-egress` / `probe-latency` already exist in v1.2.0; `/system/config` hot knobs already exist. No fork needed for this phase. Deferring Go-level A4-1 work until A4-3 proves insufficient (per ADR-0003 A4).
5. **Auto-poll subscriptions from shell** — Resin `update_interval` already exists for remote source_type; our shells post as local so this path does not apply. Shell-initiated re-fetch on user demand is the cleanest minimal change.

## Consequences

- NodesView first paint folds long subscription groups, dramatically reducing visual noise (53-node test subscription now shows 1 collapsed row instead of 53 leaf cards).
- Users get real per-node latency on demand, in line with clash-verge UX expectations.
- The Resin Go sidecar receives the same UA-rotated Clash YAML that already worked during import — subscriptions imported this way stay refreshable.
- `nodeProbe` config becomes part of `settings.json`, whitebox-editable alongside ports/strategy; users who want aggressive auto-batch can flip `batch_on_load` to true and raise concurrency up to 50 (Resin env cap is 1000).
- All 4 new CONTEXT.md terms act as references for any future agent touching NodesView or the probe path.
