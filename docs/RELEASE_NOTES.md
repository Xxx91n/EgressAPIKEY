# Release Notes

Per-release notes for people who download and run EgressAPIKEY. The exhaustive
per-commit record lives in [CHANGELOG.md](CHANGELOG.md) (Keep a Changelog
format); this page adds what that format does not carry: compatibility
declarations and upgrade notes. `scripts/upstream-router-check.cjs`
machine-checks the alignment: every release section here must exist in the
CHANGELOG with the same version and date, and every CHANGELOG version must
have a section here.

## 0.2.0 (2026-09-23)

Second public release. The headline is the strategy orchestration
controller with a graded-autonomy signal plane, hardened by real
fault-injection evidence for its environment-suspect suppressor.

### Compatibility

- **License / sidecar / platforms / i18n**: unchanged from 0.1.0
  (GPL-3.0-or-later shell; vendored Resin v1.2.0 pinned in
  `docs/RESIN_UPSTREAM_MANIFEST.yaml`; Windows MSI/NSIS/portable, macOS
  arm64+x86_64 dmg, Linux deb/AppImage, per-OS headless backend; 18
  locales).
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
- **Signal-plane typed verdicts** (ADR-0080): every tick joins loopback
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
  [THIRD_PARTY.md](THIRD_PARTY.md), decision record
  [ADR-0067](adr/0067-license-layering-provenance.md).
- **Platforms**: Windows (MSI + NSIS installer + portable exe), macOS (arm64
  and x86_64 dmg), Linux (deb + AppImage), plus a per-OS headless backend
  package; produced by the CI release matrix
  ([docs/how-to/RELEASE.md](how-to/RELEASE.md)).
- **Source runs**: Node.js 20+ with pnpm, Rust stable, and the Tauri v2
  webview prerequisites for your platform.
- **Sidecar seam**: the shell talks to the bundled Resin v1.2.0 over loopback
  REST only; other Resin versions are not drop-in compatible — the compatible
  version is pinned in `docs/RESIN_UPSTREAM_MANIFEST.yaml` (ADR-0017).
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
  Resin owns account semantics (ADR-0050). Both emit a deprecation warning on
  every call.

### Removed

- The dead in-shell gateway kernel (mihomo/gateway/lane/lease/tdewma modules,
  ADR-0050) and the unused dependencies that came with it — the shell is a
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
- Upgrading the bundled Resin sidecar is deliberately manual (ADR-0017): bump
  `docs/RESIN_UPSTREAM_MANIFEST.yaml`, re-verify THIRD_PARTY.md against the
  upstream originals at the exact new tag, and land both in the same commit.
