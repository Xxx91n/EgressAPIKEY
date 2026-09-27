# r12-wave-k run digests (observation class - NOT delivery evidence)

Observation data, not delivery evidence (r12-wave-h D-005 standing ruling). Delivery evidence remains CI-only (ADR-0072).

| run | conclusion | note |
|---|---|---|
| ci 36259888091 attempt-1 @7be89b87 | failure | port_forwarder::tests::socks5_userpass_only_offer_is_accepted - wait_bound panic at port_forwarder.rs:1285 (bound set did not reach want=true for port 47996; 396 passed/1 failed) |
| ci 36259888091 attempt-2 (failed-only rerun) | success | same SHA - flake count=1 |
| ci 36256368564 @7be89b87 (r12-wave-j-duties branch) | success | same SHA green pre-merge |
| webview-smoke 36259888095 @7be89b87 | success | merge-push smoke leg |
