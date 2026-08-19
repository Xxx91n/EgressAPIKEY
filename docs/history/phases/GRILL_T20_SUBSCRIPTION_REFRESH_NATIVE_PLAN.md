# GRILL T20 — Subscription Refresh Sink to Resin Native /actions/refresh

Branch: T20 (parallel to T17/T19 audit)
Canon ADR: docs/adr/0047-subscription-refresh-native-resin-actions.md (ACCEPTED)
Supersedes: ADR-0044 S2 (refresh path only; S1/S3/S4 untouched)
Date: 2026-08-18

## Decision (grill Q1-Q3 all locked)

- **Q1 = A**: Subscription refresh sinks entirely to Resin native POST /api/v1/subscriptions/{id}/actions/refresh. Delete the shell fetch+convert+patch chain (P13 B4 debt: fetch_clash_subscription + clash_yaml_to_proxies_block + refresh_subscription_content + 3 mockito tests, ~200 LoC).
- **Q2 = A-1**: Refresh real semantic = re-pull remote url. Resin v1.2.0 internal/topology/subscription_scheduler.go: if SourceTypeLocal -> body = in-memory content (no HTTP, no-op re-parse); else -> body = Fetcher(url). So source_type MUST be remote for refresh to actually pull new nodes.
- **Q3 = A-1-完整**: Both subscription_add AND subscription_refresh migrate to remote. A half-change would leave local where /actions/refresh is a no-op. Coupled edit, single commit.

### Source-level evidence (Resin v1.2.0, indexed in ctx)

- internal/service/control_plane_subscription.go RefreshSubscription(id) -> sync Scheduler.UpdateSubscription(sub). No source_type branch in the service layer; the branch is in the scheduler.
- internal/topology/subscription_scheduler.go UpdateSubscription: if SourceTypeLocal -> body = content; else -> body, err = s.Fetcher(attemptURL). Local = no-op re-parse; remote = HTTP fetch.
- cmd/resin/main.go const downloadUserAgent = "clash.meta". Resin already ships a clash-family UA. ADR-0044 S2 premise 1 ("Resin uses default UA") is FALSE.
- internal/netutil/downloader.go NewDirectDownloader(timeoutFn, userAgentFn) injects UA via callback. Premise 2 (flow-style choke) is UNVERIFIED in v1.2.0 — the shell-side convert was a defensive workaround that may be unnecessary, but it is deleted regardless because the shell no longer fetches.

### Rejected alternatives (full list in ADR-0047)

1. Keep shell fetch + just add /actions/refresh for local — dead end: /actions/refresh on local = no-op.
2. Shell fetch + POST remote — half-measure: duplicates Resin's own clash.meta fetcher for zero gain.
3. Shell fetch + convert + PATCH content (ADR-0044 S2) — what we are replacing; two wrong premises.
4. Fork Resin to add source_type-agnostic refresh — cosmetic fork, rejected per Ponytail.
5. Do nothing, document the local no-op — violates Q3 user expectation that refresh actually pulls.

---

## Phase P1 — ResinClient: add refresh_subscription_native + delete 3 helpers

File: crates/resin-core/src/resin_client.rs

Add:
- pub async fn refresh_subscription_native(&self, id: &str) -> Result<Value>
  -> POST {api_base}/api/v1/subscriptions/{id}/actions/refresh (bearer auth, no body)
  -> on 2xx return serde_json::Value (empty object ok)
  -> on non-2xx error with upstream status + 256B excerpt (same shape as other methods)
  -> loopback guard inherited from &self (already constructed from SidecarHandle)

Delete (P13 B4 debt, superseded by ADR-0047):
- pub async fn refresh_subscription_content (L283)
- pub async fn fetch_clash_subscription (L415)
- pub fn clash_yaml_to_proxies_block (L480)
- split_flow_fields helper (private, used only by clash_yaml_to_proxies_block)
- 3 mockito tests: mockito_refresh_subscription_content_patches_local_content (L1155)
- 3 cargo tests: clash_yaml_to_proxies_block_extracts_proxies_and_converts_flow_style (L782), clash_yaml_to_proxies_block_handles_unicode_names_and_single_quote_escape (L808), clash_yaml_to_proxies_block_rejects_yaml_without_proxies_key (L818), split_flow_fields_handles_quoted_comma_inside_value (if present)

Add 1 new mockito test:
- mockito_refresh_subscription_native_posts_actions_refresh: mock POST /api/v1/subscriptions/{id}/actions/refresh returns 200 {} -> Ok(Value::Object). Assert the POST path + that no body is sent (Content-Length: 0 or absent).

Closed loop verify:
- cargo test -p resin-core --lib baseline 137 -> expect 138 (delete ~4 tests, add 1, keep others)

---

## Phase P2 — subscription_refresh IPC rewrite (commands/mod.rs)

File: src-tauri/src/commands/mod.rs

Current state (L649):
  pub async fn subscription_refresh(name, url, admin_token hint? no — name only)
  body: fetch_clash_subscription(url) -> clash_yaml_to_proxies_block -> client.refresh_subscription_content(id, block) (P13 B4 path)

Rewrite to:
  pub async fn subscription_refresh(app, name) -> Result<(), IpcError>
    validate name (1..128 chars, no control, AGENTS s7.5)
    lock State<SidecarHandle>, build ResinClient
    client.list_subscriptions()
    items_arr -> find item where item.name == name -> extract id
    if not found -> Err(IpcError::Internal) with "subscription not found"
    client.refresh_subscription_native(id).await
    tracing::info!(subscription=%name, id=%id, "subscription refresh sink to /actions/refresh")
    Ok(())

Delete: the fetch+convert+patch block; re-use subscription_id_for_name helper (already exists, items_arr shape).

Add 2 cargo pure-helper tests:
- subscription_refresh_name_to_id_round_trip: given a list JSON with items wrapper + matching name -> returns Ok(id)
- subscription_refresh_unknown_name_returns_err: given a list with no matching name -> returns Err

Delete the url parameter from the IPC signature (no longer needed; the refresh re-pulls the url Resin already has).

Update frontend TS wrapper ipcSubscriptionRefresh (src/lib/ipc.ts L336): drop the url arg, keep name only.

Closed loop verify:
- cargo test -p resin-core --lib (covers pure helpers if they live in resin-core; if they stay in commands, they are smoke-verified by build since cargo test -p egressapikey-app exits STATUS_ENTRYPOINT_NOTFOUND on this host — AGENTS s20)
- vitest: update ipc.test.ts: ipcSubscriptionRefresh forwards name only, no url

---

## Phase P3 — subscription_add retarget to remote (commands/mod.rs)

File: src-tauri/src/commands/mod.rs L544

Current state (L544):
  fetch_clash_subscription(url) -> clash_yaml_to_proxies_block -> POST {name, source_type:"local", content:block, url, update_interval:"30s"}

Rewrite to:
  pub async fn subscription_add(app, name, url) -> Result<(), IpcError>
    validate name (1..128, no control) + url (http(s):// prefix, <=2048 chars, AGENTS s7.5)
    lock State<SidecarHandle>, build ResinClient
    POST {name, source_type:"remote", url, update_interval:"30s"} directly to /api/v1/subscriptions
    tracing::info!(subscription=%name, url=%url, "subscription add source_type=remote")
    Ok(())

Why 30s: ADR-0044 kept 30s to force a fast first tick; Resin has no public force-refresh endpoint. 30s interval keeps the first background fetch within seconds, not the default 5m.

Add 1 cargo mockito test (in resin_client.rs, next to create_subscription):
- mockito_create_subscription_remote_posts_source_type_remote: mock POST with body asserting source_type == "remote", url present, no content field. Returns 201.

Update vitest ipc.test.ts:
- ipcSubscriptionAdd body asserts source_type:"remote" (was "local")

Closed loop verify:
- cargo test -p resin-core --lib: +1 (mockito_create_subscription_remote)
- vitest: 1 assertion updated

---

## Phase P4 — i18n: 4 error keys x 18 locales + i18n_key on IpcError variants

### 4 new i18n keys (all 18 base locales + src/test/setup.ts en-inline)

- error.bindConflict     -> en "Port is already in use; pick a different port." / zh "端口已被占用，请更换端口重试。"
- error.invalidStrategy  -> en "Unknown allocation strategy." / zh "未知的分配策略。"
- error.resinUpstream    -> en "Upstream service returned an error. Retry in a moment." / zh "上游服务异常，请稍后重试。"
- error.internal         -> en "Internal error. Check logs for details." / zh "内部错误，请查看日志。"

src/test/setup.ts en-inline catalog must mirror these 4 keys (the i18n:check gate reads en/*.json as canonical).

### IpcError i18n_key field

File: crates/resin-core/src/ipc_error.rs

Add field pub i18n_key: &'static str to each variant:
- BindConflict { port: u16 } -> i18n_key = "error.bindConflict"
- InvalidStrategy { detail: String } -> i18n_key = "error.invalidStrategy"
- ResinUpstream { status: u16, excerpt: String } -> i18n_key = "error.resinUpstream"
- Internal { msg: String } -> i18n_key = "error.internal"

Update:

- impl Serialize: the existing Serialize impl (or derived serde) must emit i18n_key so the frontend translateError->extractIpcErr can pick it up. If using derive(Serialize), no extra work; if manual, add the field to the serde::serialize impl.
- Frontend src/lib/i18n-error.ts: translateError already calls t(error.i18n_key) for BindConflict; verify that ResinUpstream/InvalidStrategy/Internal also flow through the same path. If extractIpcErr only narrows by variant tag, add a fallback that reads error.i18n_key before falling back to a generic message.

### 2 vitest in i18n-error.test.ts

- translateError uses error.bindConflict key with {{port}} interpolation when IpcError variant is BindConflict
- translateError falls back to error.internal for unknown variant but uses error.resinUpstream for status >= 400

Closed loop verify:
- pnpm i18n:check -> +4 keys x 18 locales (146 -> 150)
- pnpm test -> +2 vitest (baseline 73 -> 75 or counts vary per host)

---

## Phase P5 — map_resin_error demote warn->debug + status==0 -> Internal

### Bug 1: noisy warn on single-probe failure

File: src-tauri/src/commands/mod.rs L58-60

Current:
  pub fn map_resin_error(raw: &str) -> resin_core::IpcError {
    tracing::warn!("Resin error mapped to i18n: {}", raw);

Change tracing::warn! -> tracing::debug!. Single-node probe failures are routine (node offline, timeout); a warn line per failure floods the log and the user-reported bug was "log shows [wrn] Resin error mapped to i18n" during normal batch probe. Debug keeps it searchable via RUST_LOG=debug but not noisy at default INFO.

### Bug 2: non-HTTP failures masquerade as ResinUpstream

File: crates/resin-core/src/ipc_error.rs map_resin_error

Current arms:
  status >= 400 -> ResinUpstream
  unknown -> Internal

Add an explicit arm BEFORE the 400 arm:
  status == 0 -> Internal { msg }

This catches reqwest::Error (connect timeout, DNS, sidecar down) where the status is 0 (no HTTP response). Stops a sidecar-down IPC from surfacing as "Upstream service returned an error. Retry in a moment." which is misleading.

Add 2 cargo tests in ipc_error.rs:
- map_resin_error_status_zero_returns_internal: assert status==0 -> IpcError::Internal
- map_resin_error_status_500_returns_resin_upstream: assert status==500 -> IpcError::ResinUpstream (regression guard)

Closed loop verify:
- cargo test -p resin-core --lib: +2 (139 -> 140-ish)
- Smoke: Start-Process release exe, tail logs, verify NO "[wrn] Resin error mapped to i18n" appears on a single probe failure (bug-1 symptom)

---

## Full-gate verification (AGENTS s5 hard close-loop)

1. pnpm exec tsc --noEmit green
2. pnpm i18n:check: +4 keys x 18 locales (146 -> 150)
3. pnpm test: all vitest green; watch T1-4-* flaky family (pre-existing, retry)
4. cargo test -p resin-core --lib: baseline 137 -> expect ~139-140 (delete 4 P13 tests, add 4 T20 tests)
5. cargo build --release -p egressapikey-app --features custom-protocol green
6. codegraph sync .
7. bash scripts/build-all.sh -> stage release/windows-gui/EgressAPIKEY.exe + resin.exe sidecar
8. Fresh-bundle guard: grep latest Vite chunk hash in staged exe bytes (AGENTS s5 step 4)
9. Smoke: Start-Process -> MainWindowTitle=="EgressAPIKEY", WS 20-40MB, resin child alive, log tail has NO "[wrn] Resin error mapped to i18n" from single probe (bug-1 symptom). Log tail HAS "[info] subscription refresh sink to /actions/refresh" after a refresh button click (P2 signal).

## AGENTS.md section + commit + push

- Append section (check grep -n '^### ' AGENTS.md | tail for next number; likely s60/s61) documenting T20: refresh sinks to Resin native, P13 B4 chain deleted, 4 i18n error keys, map_resin_error demote, status==0 -> Internal.
- Commit per AGENTS s6: "T20: subscription refresh sink to Resin native /actions/refresh -- delete P13 B4 shell fetch/convert/patch + 4 i18n error keys + map_resin_error demote"
- Push.
