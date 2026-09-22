# ADR-0079: Resin positioning — shell-is-the-product, engine-is-a-seam (evaluation record)

Date: 2026-09-22
Status: Draft (evaluation output, NOT a refactor authorization — R12-06)
Provenance: R12-06 ticket (r12-wave-a D-003④). Resolves the standing
question "is the real product shape just Resin itself, and where does the
shell's value boundary sit?"

## Question

Does EgressAPIKEY collapse to "Resin + a thin shell", i.e. is the shipped
product effectively the vendored Resin fork's Dockerfile/WebUI? If so,
the mere-aggregation REST seam is decorative and the project's own code
is disposable.

## Findings

### 1. The `resin/` directory is NOT a vendored fork

- `resin/` on disk is a **pristine upstream clone**
  (`origin = github.com/Resinat/Resin.git`, clean worktree, no local
  commits/diffs), **gitignored** (`.gitignore` line 96) — it is dev
  reference material, not repo content.
- The shipped binary is fetched **prebuilt from upstream Releases** by
  `scripts/fetch_resin.{ps1,sh}` (pinned by
  `docs/RESIN_UPSTREAM_MANIFEST.yaml`, currently v1.2.0). Nothing in the
  build compiles `resin/`.
- The clone's `Dockerfile` / `webui/` are **upstream's own artifacts** —
  this product ships neither (the WebUI is unused; headless deployment
  uses `docs/how-to/HEADLESS_DEPLOYMENT.md`'s own illustrative
  Dockerfile). Value attribution: zero. The clone could be deleted from
  disk with no product impact; it exists to speed up diagnosis and the
  R12-00 bench harness compiles `resin-core` against its documented API.

### 2. The value boundary is real and thick

The REST seam is narrow (~10 endpoint families, `ResinClient` = 41
methods). Everything the user actually buys lives shell-side:

| Layer | Owner | Content |
|---|---|---|
| Identity model | shell | entry port = (platform, account); Resin has no port-identity concept — its single consolidated endpoint routes by `Platform.Account` credential |
| Mode A forwarder | shell | per-port loopback listeners + credential injection + CONNECT retunnel (neutralizes the Mode B forward-path SSE buffering defect — registered exemption `mode-b-sse-buffering`) |
| Whitebox authority | shell | ports/platforms/strategy whitebox (L2); Resin state is derived (L3); snapshot/divergence reporting |
| Strategy engine | shell | a_class (region/quality/subscription/manual) -> region projection; b_class -> allocation_policy mapping |
| Orchestration | shell | graded autonomy, verdict windows, region metrics, cooldowns |
| Control surface | shell | Tauri GUI + headless SPA (auth token, Host/Origin allowlist, capabilities) |
| Ops plane | shell | lease map, request-log tail, port health, exit-IP probes, backup/restore, converge loop |

Resin supplies: the proxy dataplane (SOCKS5/HTTP + CONNECT), sticky
exit-IP leases, node pool + health + subscription fetch, allocation
policies, request log, node probes. **None of these expose the
port-per-identity model** — that is the product.

### 3. Mere-aggregation seam risk assessment

- **License**: the seam holds the mere-aggregation doctrine (separate
  processes, loopback REST, prebuilt binary — no linking). Shell stays
  MIT-clean; the sidecar binary conveys GPL-3.0 via sing-box
  (THIRD_PARTY.md registry). Re-verified 2026-09-22: no code path links
  or patches Resin internals; the shell consumes REST + spawns the
  process only.
- **Contract risk**: Resin's API has broken compat before (v1.1 -> v1.2:
  allocation_policy enum, region_filters, /nodes pagination — all
  registered in the manifest). Mitigations already live: the manifest
  compat-notes section, the `mode-a-contract-check` CI gate (byte-level
  D4 scenarios against the real binary), and the bench harness.
- **Structural risk**: (a) upstream adds native per-port listeners ->
  shell differentiator narrows but never zeroes (whitebox/orchestration/
  GUI are ours); (b) upstream stagnates -> the engine is replaceable
  behind the seam; Mode A already bypasses the engine forward path for
  CONNECT. Neither requires action today.
- **README §separation (license paragraph)**: accurate — "separate
  processes interacting only over a loopback REST seam (mere
  aggregation)" remains literally true post-R12-05 (new deep-edit IPCs
  are shell-internal; the Resin boundary is unchanged).

## Decision (evaluation conclusion, no code change)

- The product is the **shell**; Resin is a **replaceable engine behind a
  narrow REST seam**. No refactor warranted.
- `resin/` stays a gitignored reference clone; its Dockerfile/webui are
  upstream's and are never shipped. `fetch_resin` remains the only
  sanctioned sidecar acquisition path.
- Keep the seam honest: every Resin API change goes through the manifest
  + contract gate; never patch upstream source in the clone to fix the
  product (fix shell-side, register exemptions like
  `mode-b-sse-buffering`).

## Consequences

- Doc-only: no code changes this ticket.
- Future upstream bumps: bump manifest version, run contract gate +
  bench, update compat notes.
- If upstream ever ships native port-identity, re-open this ADR before
  evaluating adoption.
