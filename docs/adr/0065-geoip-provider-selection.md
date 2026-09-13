# ADR-0065: GeoIP provider selection — third-party IP reputation over Resin built-in GeoIP

Status: ACCEPTED
- **Date**: 2026-09-05 (round5 T20 / 裂痕 #7 — 波次 W4)
- **Supersedes**: nothing; documents a selection left implicit by ADR-0050 —
  coverage rows R40-R43 (`/api/v1/geoip/*`) move from 空白-pending-T20 to
  故意不接 with the reasons recorded here.
- **Research basis**: `resin/internal/api/handler_geoip.go:10-49` (upstream
  handlers, read verbatim) and `crates/resin-core/src/ip_reputation.rs`
  (shell-side providers, read verbatim); reports/03 §5 GeoIP row;
  T13 `docs/architecture/RESIN_API_COVERAGE.md` rows R40-R43.

## Context

Upstream Resin exposes four GeoIP endpoints:

- `GET /api/v1/geoip/status` (R40, handler_geoip.go:11) — GeoIP database status;
- `GET /api/v1/geoip/lookup` (R41, handler_geoip.go:19) — single-IP lookup;
- `POST /api/v1/geoip/lookup` (R42, handler_geoip.go:50) — batch lookup
  (`{"ips": [...]}` → `{"results": [{"ip", "region"}, ...]}`);
- `POST /api/v1/geoip/actions/update-now` (R43, handler_geoip.go:39) — force
  a database refresh.

Every data-returning handler answers with a **region** string per IP
(`{"ip": ..., "region": ...}`) — pure geographic location, nothing else.
ADR-0050 deleted the shell's dead kernel face and any GeoIP integration with
it, but recorded no rationale: AGENTS.md §Storage locations L1 lists the
third-party keys (`ipReputationProvider` / `ipQualityScoreApiKey` /
`abuseIpDbApiKey`) without saying why Resin's built-in GeoIP is not consumed.
A reader could assume the GUI's IP-reputation surface queries Resin when it
actually calls third-party HTTP APIs (round5 crack #7).

## Decision

**D1 — The shell queries third-party IP-reputation providers and does not
wire Resin's built-in GeoIP.** `ip_reputation_snapshot` /
`crates/resin-core/src/ip_reputation.rs` keep calling IPQualityScore and
AbuseIPDB (ip-api as the explicit opt-in keyless fallback) directly; no
ResinClient method, no IPC command, and no TS wrapper is added for
`/api/v1/geoip/*`. Coverage rows R40-R43 move 空白 → 故意不接.

**D2 — Rationale.**

1. **Signal mismatch**: the shell's question is "can this egress IP be
   trusted" — fraud score (`fraud_score` 0-100), abuse confidence
   (`abuseConfidenceScore`), proxy/VPN/Tor flags (`ip_reputation.rs`
   `map_response`). Resin GeoIP answers "where is this IP": a region string
   only — pure geography, no reputation dimension.
2. **Requirement fit**: the feature gates exit-IP trust decisions, not geo
   distribution. Geography adds no decision input the third-party providers
   don't already return — every `ReputationEntry` carries `country_code`
   alongside the reputation fields.
3. **Legislated credentials**: the third-party API keys already live in L1
   `settings.json` (AGENTS.md §Storage locations), so the config-authority
   model is unchanged. Resin GeoIP would add an L3-derived data source for
   the same surface without replacing the third-party need.

**D3 — Scope: documentation only.** Zero code change: `ip_reputation.rs`
behavior, the L1 keys, and the four upstream endpoints (which stay alive
inside Resin for its own WebUI) are untouched. Resin GeoIP is 故意不接 —
classified, not forbidden.

## Consequences

- Positive: "why doesn't the IP-reputation card hit Resin GeoIP" has a
  written answer; the coverage table's GeoIP rows are classified.
- Cost: none at runtime (zero code paths). If upstream Resin ever enriches
  GeoIP with reputation dimensions (fraud/abuse signals), this ADR must be
  revisited via a new one before any wiring.
- Neutral: per ADR-0062 D5 the coverage rows and the D3/D4 ledger moved in
  the same commit as this ADR; the counts in RESIN_API_COVERAGE.md remain a
  point-in-time audit stamp.
