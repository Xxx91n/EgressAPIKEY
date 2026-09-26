# Evidence Archive (observation class)

Convention legislated r12-wave-i D-003: small observation-data digests cited by the
trigger-line register or decision ledgers get copied here so conclusions stay
replayable by anyone with repo access (.scratch/.codex-tmp are gitignored).

Every file/dir here is **observation data, not delivery evidence** (r12-wave-h
D-005 standing ruling): delivery evidence remains CI-only per ADR-0072.

Layout: `evidence/<wave-slug>/<artifact>`. Files carry a header line declaring
the observation class. Machine-readable formats that cannot carry a comment
header (e.g. raw JSON) declare the class via a top-level
`"_class": "observation"` field instead - same declaration, reachable shape.
Keep artifacts small - digest/summary preferred over raw
logs (link the source path for full fidelity).
