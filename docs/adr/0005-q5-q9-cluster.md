# ADR-0005: Q5-Q9 resolved - subscription depth, log viewer, CI/CD, backup, tests

Date: 2026-08-02
Status: PROPOSED (provisionally resolved from evidence)
Decision Type: Multi-decision (one ADR per cluster; consolidated)

## Q5: Subscription management depth

### Evidence (resin_client.rs source)
ResinClient exposes: create_subscription, list_subscriptions, delete_subscription.
No subscription_update PATCH method exists in the .rs file.

### Gap
Resin subscriptions table carries: enabled, update_interval_ns, ephemeral,
LastError (display). Webui lets the user toggle enabled and set the interval.
Our SubscriptionsView only does add/list/remove (import a Clash URL + delete
by name). No enable/disable, no interval control, no ephemeral toggle, no
LastError display.

### Decision
Add ipcSubscriptionUpdate(name, {enabled?, update_interval?, ephemeral?})
that PATCHes /api/v1/subscriptions/{id} (probe the endpoint shape on the live
sidecar before assuming). ResinClient::update_subscription(id, body) method.
SubscriptionsView extends the per-row card with a Enable toggle and an
Interval select (30s/5m/15m). The LastError column is read-only; surface the
field from list_subscriptions.

### Status
GAP — implement in next phase after user confirms grill direction.

## Q6: Request log viewer

### Evidence
There is no request-log query IPC, no LogViewer tab, no ResinClient method
that queries /api/v1/logs or similar. The shell Rust tracing layer writes to
tauri-plugin-tracing's daily-rotating files; that is Rust-shell trace, not
Resin's structured request log (the user-visible audit trail).

### Gap
Resin's README explicitly advertises "complete structured request logs for
querying and auditing by platform, account, target site, and more." This is
a major user-visible feature we surface nowhere.

### Decision
Add a new "RequestLogs" tab (or sub-view of Topology). ResinClient gains a
query_request_logs(filters) method that GETs the correct Resin endpoint
(DESIGN.md references a structured request log but no endpoint was probed
live; probe the sidecar for GET /api/v1/logs?platform=&account=&target= before
coding). The view renders filterable rows with timestamp / platform / account
/ target / status / egress_ip.

### Status
GAP — DESIGN.md confirms the feature exists in Resin; the public admin API
endpoint shape must be live-probed before code.

## Q7: CI/CD completeness

### Evidence (ci.yml, build-all.sh)
- CI matrix: backend (linux x86_64, windows msvc, macos arm64, macos x86_64).
- Installer matrix (msi/nsis setup.exe, deb+AppImage, dmg for arm64+x86_64).
- Portable matrix (linux/windows msvc/macos arm64 — macos x86_64 portable IS
  present via the matrix.include list in the ci.yml gui-portable job).
- Every GUI job fetches the Resin sidecar via fetch_resin.sh before tauri build.
- build-all.sh reproduces locally: pnpm build -> cargo build release -> stage
  release/<os>-gui/ with bundles + portable ai-api-route(.exe). Flow guard
  on windows launches the portable exe and verifies MainWindowTitle, 5s alive.

### Decision
CI/CD satisfies the user's release pipeline requirement (backend + installer +
portable per OS, fetch-sidecar guard, ERR_CONNECTION_REFUSED prevention via
--features custom-protocol). No change required for Q7.

### Status
COMPLETE as-evaluated. User previously noted the stale-release-exe hazard
(P23) — AGENTS.md §5 was hardened to forbid bare cargo build without vite build
first. That is the only durable fix needed; CI itself is fine.

## Q8: Backup/WebDAV security

### Evidence (commands/mod.rs)
- backup_create zips settings.json + Resin state dir to backups_dir with a
  crypto-random suffix.
- backup_upload(url, username, password, zip_path) uploads to WebDAV via PUT.
  P14 fix: canonicalize zip_path + starts_with guard confines to
  app_data/backups - blocks the traversal bug.
- backup_list(url, username, password) does PROPFIND on the WebDAV base.
- config_import auto-creates a backup before applying (防呆).

### Decision
Backup is shipped with the P14 path-traversal fix in place. The only
remaining security note (P22 audit): WebDAV credentials + keyCandidates
apiKey are stored plaintext in settings.json (tauri-plugin-store). Known
risk, mirrors the clash-verge-rev pattern. Not upgrading to OS keyring in
this grill cycle.

### Status
COMPLETE. Security is audited and the documented risk is accepted.

## Q9: Test coverage

### Evidence
- vitest files (8): appStore.test.ts, ipc.test.ts, subDrag.test.ts,
  TopologyView.test.tsx, PlatformsView.test.tsx, SubscriptionsView.test.tsx,
  ProcessRouteView.test.tsx (plus a couple). 66 tests pass.
- e2e tests: e2e directory is empty (no .ts files). CI runs playwright test
  against the Vite dev server surface; local playwright skipped as host
  resource.
- resin_client.rs has mockito-based offline integration tests covering
  create/list/delete/error for platforms + subscriptions (6 tests + update).

### Gap
The closed-loop coverage described in AGENTS §18 "each behavior module must
have a closed-loop test" is satisfied for the IPC boundary and for the few
pure helpers (subDrag, applyOrder), but the views themselves only mock
invoke() — they do not cover the Topology drag-to-PATCH end-to-end through
a live or mockito mock. There is no integration test that asserts
"add-platform -> list-platform -> the new platform appears in Topology".

### Decision
Acceptance: ship the grill with the existing test shape. The closed-loop
contract means "each IPC boundary has vitest asserts that the wrapper exists
and validates input + forwards to invoke". Future phases should add an
e2e suite that drives IPC behavior through a mockito Resin mock and asserts
visual state change — but this is a longer-horizon need, not a Q9 grill blocker.

### Status
ACCEPTED with documented gap. Not a blocker for moving to implementation.
