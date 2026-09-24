# Mode A boundary-law census (R12-E1)

`census.json` is the **frozen inventory** of the Mode A dataplane
(`crates/resin-core/src/port_forwarder.rs`): every non-test function, the
rewrite-rule whitelist, the wire-dialect set, the declared entry-protocol
value set, and the StreamSensor classification dimensions.

`scripts/mode-a-contract-check.cjs` asserts the source still matches this
file on every verify run. It is the CI-assertion baseline for the
registered trigger line "Mode A shell forwarder boundary law"
(`docs/agents/trigger-line-register.md`).

## Boundary law (what the gate fails on)

1. **Protocol-family freeze** — wire dialects are exactly
   `{socks5-handshake, http-connect, absolute-form->CONNECT}` and the
   declared protocol value set stays `{http, mixed, socks5}`.
2. **No TLS fronting** — ADR-0068 D4 verbatim: "HTTPS/TLS fronting of
   entry ports is explicitly out of scope for this decision." Any TLS
   implementation token (`rustls`, `TlsAcceptor`, `0x16`, …) fails.
3. **No new L7 features** — the function inventory must equal
   `functions` exactly. A new handshake dialect, a new rewrite rule, or a
   new StreamSensor dimension is net-new dataplane behavior: it belongs
   to the ADR-0068 D2 fork line / trigger-line adjudication, not a quiet
   edit.

`port_forwarder.rs` line count is reported warn-only — a reference
column, never the trigger (register wording).

## Deliberate change path

If a change is adjudicated to extend Mode A, update `census.json` **in
the same commit** and cite the adjudication (decision-ledger D-id or ADR)
in the commit message. A gate failure with no matching adjudication is
the trigger line firing — stop and re-open, do not "fix" the census.
