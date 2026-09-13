# Config Export/Import: Whitebox as Source (no direct Resin read/write)

Status: ACCEPTED

**Round**: 5 (W2.3) — ticket T07, fractures #3 + #7

## Context

`config_export` / `config_import` (in `src-tauri/src/commands/backup.rs`) were
written before ADR-0036 legislated the whitebox files as the single source of
truth. They bypassed the L2 whitebox entirely:

- `config_export` read Resin live state (`list_platforms` /
  `list_subscriptions`) and assembled a derived `{platforms, subscriptions}`
  document — it never read `egressapikey-strategy.json` or
  `egressapikey-ports.json`.
- `config_import` re-created platforms/subscriptions by direct PATCH to Resin
  (`update_platform` / `create_subscription`) — it never wrote either whitebox
  file.

This is the same "say establish, actually bypass" class of contradiction
ADR-0056 removed from the apply path. It also left a TS-side path drift:
`src/lib/ipc.ts` mapped `config_export` → `/api/v1/config/export` and
`config_import` → `/api/v1/config/import`, endpoints Resin does not expose.

## Decision

1. **`config_export` reads the whitebox, never Resin.** It reads
   `egressapikey-strategy.json` (via `StrategyService`, ADR-0036) and
   `egressapikey-ports.json` (via `WhiteboxConfigStore` snapshot) and wraps
   them verbatim in a versioned JSON container
   (`resin_core::config_transfer::build_export_doc`). The container is:

   ```json
   {
     "format": "egressapikey-config",
     "version": 1,
     "exported_at": "<rfc3339>",
     "strategy": { /* raw StrategyConfig */ },
     "ports": { /* raw WhiteboxConfig */ }
   }
   ```

2. **`config_import` writes the whitebox, never PATCHes Resin.** It parses +
   validates both documents up front (`parse_import_doc`; any violation is a
   clear `IpcError` with **no write**), then persists through the single
   sanctioned write entries: `StrategyService::store` (ADR-0036) and
   `WhiteboxConfigStore::apply` (ADR-0055). It then triggers the one-way
   reconcile (`reconcile_now`'s path, ADR-0054 §A) so Resin converges from the
   imported whitebox via diff-then-skip (ADR-0057).

3. **CMD_TO_HTTP path drift removed.** `config_export` / `config_import` are
   no longer mapped to `/api/v1/config/*` in `src/lib/ipc.ts`. They are
   Tauri-only: local whitebox file I/O has no headless HTTP surface.

## Schema / version compatibility

- The container carries a `version` field (format version 1). An unsupported
  container version fails cleanly — never a silent mis-parse.
- The inner whitebox documents are deserialized through serde (unknown fields
  are ignored), then re-validated through their own validate entries
  (`strategy_service::validate` / `whitebox_config::validate`), which is where
  the version gate lives. When T09 bumps the strategy schema to v2 and relaxes
  that gate, this import path keeps working unchanged because it only forwards
  the typed document. Locked by unit tests in
  `crates/resin-core/src/config_transfer.rs`.

## Consequences

- The former Resin-derived subscription export/import is gone: subscriptions
  are Resin L3 state, not part of the L2 whitebox config layer. `config_import`
  reports `subscriptions_created: 0` and `platforms_created` = strategy
  platforms written.
- "Include Resin derived" (exporting L3 `state.db`/`cache.db`/subscriptions
  alongside the whitebox) is deferred to a later round; the default is
  whitebox-only.
- `backup_create` is untouched (T06 legislates its L3 read exception
  separately via ADR-0050-bis).

## Out of scope

- Changing `backup_create` (T06).
- The "Include Resin derived" export option (default off, future round).
- One-time migration of previously exported Resin-derived config JSON (user
  self-service, not automatic).
