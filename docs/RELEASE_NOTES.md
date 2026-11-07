# Release Notes

Per-release notes for people who download and run EgressAPIKEY. The exhaustive
per-commit record lives in [CHANGELOG.md](CHANGELOG.md) (Keep a Changelog
format); this page adds what that format does not carry: compatibility
declarations and upgrade notes. Keep the alignment by hand: every release
section here must exist in the CHANGELOG with the same version and date, and
every CHANGELOG version must have a section here.

## Unreleased

## 0.2.1 (2026-11-07)

Pre-release (beta) focused on release-pipeline packaging completeness, unified
`0.2.1` manifest versioning, headless reverse-proxy trust hardening, SQLite
read-path separation, and i18n / audit-log integrity guards.

### Compatibility

- **License / sidecar / i18n**: unchanged from 0.2.0 (GPL-3.0-or-later shell;
  vendored Resin v1.2.0 pinned in `docs/RESIN_UPSTREAM_MANIFEST.yaml`; 18
  locales gated on key parity, content fingerprints, and static key
  references).
- **Release artifacts**: a single GitHub Release now carries all three
  distribution tracks together (`prerelease` selectable at dispatch;
  `docs/how-to/RELEASE.md`):
  - **Desktop installers**: Windows MSI + NSIS (`*-setup.exe`), Linux `.deb` +
    `.AppImage`, macOS arm64 `.dmg` (macOS x86_64 `.dmg` opt-in best-effort
    until 2027-08 on `macos-15-intel` via `include_intel_mac`).
  - **Desktop portable archives**: `windows-gui-portable.zip`,
    `linux-gui-portable.tar.gz`, `macos-gui-portable-arm64.tar.gz` (each
    bundling `EgressAPIKEY(.exe)` alongside the required `resin(.exe)`
    sidecar).
  - **Headless backend tarballs**: `linux-backend.tar.gz`,
    `windows-backend.tar.gz`, `macos-arm64-backend.tar.gz` (plus opt-in
    `macos-x86_64-backend.tar.gz`), and `SHA256SUMS.txt` across all bundles.
- **Upgrade surface**: drop-in over 0.2.0 and 0.1.0; whitebox documents and
  `egressapikey.db` carry forward automatically with no manual migration step
  (except headless deployments behind a TLS-terminating reverse proxy — see
  Upgrade notes below).

### Upgrade notes

- **Headless behind a TLS-terminating proxy**: the headless server now ignores
  `X-Forwarded-Proto` unless the socket peer matches the repeatable
  `--trusted-proxy <ip-or-cidr>` flag, and evaluates only the **rightmost**
  segment of the merged `X-Forwarded-Proto` chain (the hop nearest the binary).
  If your reverse proxy terminates TLS and forwards plain HTTP to
  `egressapikey-headless`, configure the proxy to overwrite
  `X-Forwarded-Proto` with the client-facing scheme (for example,
  `proxy_set_header X-Forwarded-Proto $scheme;` in nginx; Caddy's
  `reverse_proxy` sets it automatically), and add
  `--trusted-proxy=<proxy address or CIDR>` so HTTPS bootstrap cookies carry
  `Secure`. Wildcard `/0` CIDRs (`0.0.0.0/0`, `::/0`) refuse startup unless
  `--trusted-proxy-unrestricted` is also supplied. Direct/plain-HTTP
  deployments need no change
  ([docs/how-to/HEADLESS_DEPLOYMENT.md](how-to/HEADLESS_DEPLOYMENT.md)).

### Highlights

Summarized from the CHANGELOG [0.2.1] section:

- **Complete release packaging & verification**: `.github/workflows/ci.yml`
  gates `workflow_dispatch` releases behind both `verify` and the tier-2
  `macos-test` gate, enforces Resin sidecar presence in every portable and
  headless stage, verifies four-manifest version parity (`package.json`,
  `Cargo.toml`, `src-tauri/tauri.conf.json`, `scripts/npm/package.json`), and
  attaches portable archives, headless tarballs, and `SHA256SUMS.txt` to the
  Draft Release with `prerelease` support.
- **SQLite dedicated read connection & lock-wait instrumentation**: `DbPool`
  routes read queries onto a dedicated read connection separate from the writer
  `Mutex`, backed by the opt-in `db-lock-metrics` observation feature and the
  `BEGIN IMMEDIATE` source gate (`scripts/db-immediate-txn-check.cjs`).
- **Three-layer i18n verification**: key-set parity (`i18n-check.cjs`) is now
  joined by committed SHA-256 copy value fingerprints
  (`i18n-content-check.cjs` / `docs/I18N_CONTENT_MANIFEST.json`) and static
  key-reference resolution (`i18n-key-ref-check.cjs`).
- **Encoding & contract hardening**: unified RFC 3986 percent-encoding helpers
  in `resin_core::encoding` and v3 fail-closed Mode A boundary census checks.

### Fixed

- Audit log single-critical-section append, typed tail adoption, stored-byte
  hash chaining, and file-size re-seeding before rotation.
- IPQS API-key path-segment percent-encoding (`%20` instead of `+`),
  `DiagnosticsView` port-health i18n key mismatches, 18-locale CLDR colon
  spacing on `portBound`, `TopologyView` no-op state reference preservation,
  and Vitest 4 / pnpm 11 test & build compatibility.

### Known issues

Tracked in open issues (retained open for ongoing triage and follow-up):

- **[#13](https://github.com/Xxx91n/EgressAPIKEY/issues/13)** — Strategy
  orchestration `pending.diff` and `pending.reason` strings produced in Rust
  are currently rendered verbatim in English in the UI rather than localized
  through the 18-locale catalog.
- **[#14](https://github.com/Xxx91n/EgressAPIKEY/issues/14)** — Upstream Resin
  v1.2.0 plain-HTTP forward-form requests on headless Mode B ports buffer SSE
  responses (`forward.go` `io.Copy` without per-chunk flush). Standard AI API
  traffic over CONNECT-tunnelled HTTPS is unaffected, and desktop Mode A
  re-tunnels through CONNECT; upstream issue filing is tracked in #14.
- **[#35](https://github.com/Xxx91n/EgressAPIKEY/issues/35)** — `DbPool`'s
  single dedicated read connection eliminates writer-lock contention, but
  stress-level concurrent read fan-out (`list_ports`) still serializes on that
  single reader; read-only connection pooling (`deadpool-sqlite`) is queued for
  evaluation in #35.
- **[#7](https://github.com/Xxx91n/EgressAPIKEY/issues/7)** — Absolute
  acceptance thresholds marked `†` in `scripts/bench/acceptance.json` /
  `bench-selfhosted.yml` remain provisional pending calibration on
  representative self-hosted hardware.
- **[#6](https://github.com/Xxx91n/EgressAPIKEY/issues/6)** —
  `crates/resin-core/src/strategy_service.rs` (~3,600+ lines) remains under
  watch for modular decomposition prior to major feature additions.
- **Distribution notes** — Desktop installers and portable binaries are
  unsigned community pre-release artifacts (Windows SmartScreen / macOS
  Gatekeeper first-run confirmation applies); macOS Intel (`x86_64`) builds are
  opt-in best-effort via `include_intel_mac`; `@egressapikey/server` is not
  published to npm (use the release `<os>-backend.tar.gz` archive or run from
  source).

## 0.2.0 (2026-09-23)

Second public release. The headline is the strategy orchestration
controller with a graded-autonomy signal plane, hardened by real
fault-injection evidence for its environment-suspect suppressor.

### Compatibility

- **License / sidecar / platforms / i18n**: unchanged from 0.1.0
  (GPL-3.0-or-later shell; vendored Resin v1.2.0 pinned in
  `docs/RESIN_UPSTREAM_MANIFEST.yaml`; Windows MSI/NSIS/portable, macOS
  arm64 dmg first-class (x86_64 opt-in best-effort until 2027-08 -
  macOS 15+ Intel only, via the `include_intel_mac` dispatch input),
  Linux deb/AppImage, per-OS headless backend; 18 locales).
- **Upgrade surface**: whitebox `orchestration` section is additive and
  serde-defaulted - an 0.1.0 config loads unchanged with the controller
  disabled until you opt in. `egressapikey.db` carries forward via the
  normal migration chain (no manual step).

### Highlights

Summarized from the CHANGELOG [0.2.0] section:

- **Orchestration controller** (graded autonomy): the whitebox gains an
  `orchestration` section - enabled switch, parameters, per-platform
  state rows - driving a 60s tick on both the desktop and headless
  transports. Suggest tier parks proposals for explicit approve/dismiss;
  Auto tier migrates regions through the authoritative write path only.
- **Signal-plane typed verdicts**: every tick joins loopback
  listener health with a real egress probe per enabled entry port into a
  five-value verdict (ok / local_fail / remote_fail / skipped /
  environment_suspect). When >=80% of probed ports fail loopback in one
  tick the controller stamps the whole tick `environment_suspect` - a
  local outage is one environmental event, not N platform failures - and
  a 3-tick suspect streak emits exactly one audit row.
- **Fault-injection evidence** (the suppressor is no longer a claim, it
  is measured): the new opt-in bench phase `faultinject` boots the real
  headless SUT on a fresh 5-platform fixture, kills entry forwarders and
  the egress node across real process boundaries, and asserts the
  suppressor contract through the control-surface tick endpoint. CI run
  35820172528 (windows-latest + ubuntu-latest, both green) records:
  kill-forwarder -> all-5 `environment_suspect` with streak climbing
  1->2->3 and exactly one
  `signal-plane/environment_suspect/orchestration:tick` audit row
  (`streak=3 local_fail=5/5`); entry restore -> streak resets to 0;
  kill-node -> all-5 `remote_fail` with the suppressor untouched;
  node restore -> all `ok`.
- **Perf bench harness** (dual-profile): idle / paired / rps / sse / soak
  / healthz phases with acceptance gates, plus whole-tree vps-headless
  phases measuring the headless binary's descendant-tree RSS and cold
  start. `--phases` is exact when given; `faultinject` is opt-in only
  and requires real WAN egress.
- **Mode A entry listener**: RFC 1928-compliant method negotiation - a
  credential-free entry now prefers NoAuth when the client offers it and
  accept-any RFC 1929 otherwise, so clients that only offer UserPass
  (reqwest/hyper-util for userinfo URLs) are no longer refused before
  CONNECT.
- **Headless audit parity**: the headless transport now initialises the
  same audit sink as the GUI shell, so signal-plane bookkeeping rows
  persist under the headless state root.
- **Bench hygiene**: shared descendant-tree sampling body for the
  app/headless steady-state phases, single killTree/waitTcpReady
  implementations, and PlatformsView unwrap/gather dedup.

### Fixed

- SOCKS5 UserPass-only clients through Mode A entries (see above).
- e2e-webview side-rail navigation (testid contract for icon-only
  buttons), `lightweight_set` rejection containment, markdownlint in
  the push verify gate.

### Upgrading

- Drop-in over 0.1.0 - config/db migrate forward automatically; the
  orchestration controller arrives disabled until enabled in the
  strategy whitebox.
- If you consume the bench harness on a self-hosted runner,
  `--phases faultinject` additionally requires outbound access to
  1.1.1.1 (the egress probe target is hardcoded).

## 0.1.0 (2026-09-06)

Initial public release of EgressAPIKEY (formerly **ai-api-route**): a
multi-port socks5/http forwarder between AI gateways and upstream providers —
sticky exit-IP routing for AI API keys, with a topology canvas.

### Compatibility

- **License**: the shell is GPL-3.0-or-later; the vendored Resin Go sidecar
  v1.2.0 carries a two-layer license value — declared MIT, dependency-tree
  GPL-3.0-or-later via sing-box. Redistribution obligations:
  [THIRD_PARTY.md](THIRD_PARTY.md).
- **Platforms**: Windows (MSI + NSIS installer + portable exe), macOS (arm64
  and x86_64 dmg), Linux (deb + AppImage), plus a per-OS headless backend
  package; produced by the CI release matrix
  ([docs/how-to/RELEASE.md](how-to/RELEASE.md)).
- **Source runs**: Node.js 20+ with pnpm, Rust stable, and the Tauri v2
  webview prerequisites for your platform.
- **Sidecar seam**: the shell talks to the bundled Resin v1.2.0 over loopback
  REST only; other Resin versions are not drop-in compatible — the compatible
  version is pinned in `docs/RESIN_UPSTREAM_MANIFEST.yaml`.
- **i18n**: 18 locales (en, zh, ja, es, fr, de, ko, ru, pt, ar, hi, id, it,
  nl, pl, th, tr, vi).

### Highlights

Summarized from the CHANGELOG [0.1.0] section (31 Added, 17 Changed, 8 Fixed,
1 Deprecated, 6 Removed entries there):

- L7 proxy gateway for AI API keys with per-(Platform, Account) sticky exit
  IPs, Resin Go sidecar integration, and the three-column topology canvas.
- Config-authority closure: L1/L2/L3 layering, whitebox user-editable config
  with versioned backup/rollback, one-way reconcile + preview, the Effective
  Config view, authoritative snapshots with a three-state desired|live merge,
  and per-drift-episode tray notification.
- Resin integration depth: the API coverage ledger (58 routes), the
  account-header-rules family, metrics throughput/probe cards, and the
  request-log detail drawer.
- Governance: GPL-3.0-or-later LICENSE, THIRD_PARTY.md, the MADR ADR series,
  18-locale i18n, and the community health file set.

### Deprecated

- `account_add` / `account_bind_ip` echo commands: validate-and-echo only —
  Resin owns account semantics. Both emit a deprecation warning on
  every call.

### Removed

- The dead in-shell gateway kernel (mihomo/gateway/lane/lease/tdewma modules,
  ) and the unused dependencies that came with it — the shell is a
  pure control plane over the sidecar REST seam.

### Fixed

- Headless white-screen (isTauri guard + SPA fallback), topology
  race/resync/drag defects, orphan-sidecar process kill, subscription
  drag-reorder via Pointer Events, subscription refresh reporting the real
  post-refresh node count, and ResinClient write-path 5xx retry.

### Upgrading

- First release — no upgrade path to run.
- Coming from pre-rename **ai-api-route** builds: legacy `settings.json`
  network keys (`gatewayBind`, `mihomoApi`) are purged and `processRoutes`
  is migrated into the whitebox once at startup — no manual action.
- Upgrading the bundled Resin sidecar is deliberately manual: bump
  `docs/RESIN_UPSTREAM_MANIFEST.yaml`, re-verify THIRD_PARTY.md against the
  upstream originals at the exact new tag, and land both in the same commit.
