# ADR-0020: Test-per-capability closed loop

Date: 2026-08-06
Status: ACCEPTED
Context: AGENTS sec4 requires per-behavior tests. Q7 grill establishes the test protocol for the T2 lifecycle/pipeline work.

## Decision

Each new capability from ADR-0016/0017/0018/0019 ships with a closed-loop
test in the SAME commit:

- RingBuffer (ADR-0016): #[test] ring_buffer_evicts_oldest + bound + drain
- Crash restart (ADR-0016): mock child death -> verify retry count + backoff
- Two-phase shutdown (ADR-0016): mock SIGTERM -> wait -> SIGKILL sequence
- stdout/stderr pipe drain (ADR-0016): CommandEvent -> ring buffer assertion
- Manifest YAML parse (ADR-0017): read RESIN_UPSTREAM_MANIFEST.yaml fields
- fetch_resin version from manifest (ADR-0017): PS1 + sh read YAML not hardcoded
- Read-only ResinClient retry (ADR-0018): mockito 503 -> retry -> 200 happy path
- build-all.ps1 stage + SHA256 (ADR-0019): staged exe exists + sha256 matches

No capability commit is complete without its test. This is the internal
review mechanism promised in Q5.

## Rationale

Ponytail: non-trivial logic (branch, loop, parser, security path) leaves
ONE runnable check. Each capability here has a branch or loop that can
silently break. The test is the smallest thing that fails if the logic
decays.

Writing tests at the end risks forgetting edge-case behavior. Writing
them inline forces the implementation to be testable from the start.

## Tradeoffs

- Slightly slower per-commit (write impl + test together)
- Every diff has evidence; no test debt accumulation
