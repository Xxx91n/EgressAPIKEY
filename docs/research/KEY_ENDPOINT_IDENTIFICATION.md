# Key + Endpoint Identification (P19 item 3 - research deliverable)

> Source: Resin v1.1.2 DESIGN.md and README plus 1mcp web research (litellm
> load-balancing docs, mihomo proxy wiki). Verified against the live Resin
> sidecar on this host.

## TL;DR

Resin already identifies each (api key, upstream v1 endpoint) pair at the
gateway in flight. There is **no new code to write** here — the work is to
**expose the existing knobs** to the desktop UI and document the contract.

## Resin native mechanism (live, in-process, proven)

A request reaches the Resin forward proxy at:

```text
http://127.0.0.1:<port>/[<account>/<platform>/https/<upstream-host>/<path>]
```

For zero-intrusion integration (the case we want: omniroute / litellm only
sets one upstream Authorization header and the proxy port), the Platform
config drives the identification:

```yaml
{
  "name": "openai-pool",
  "regex_filters": ["\.openai\.com$|api\.openai\.com"],
  "region_filters": ["us","hk","jp"],
  "reverse_proxy_empty_account_behavior": "ACCOUNT_HEADER_RULE",
  "reverse_proxy_fixed_account_header": "Authorization\nX-Api-Key\nX-Auth-Token",
  "allocation_policy": "PREFER_LOW_LATENCY",
  "sticky_ttl": "168h0m0s"
}
```

Two headers do the work:

1. **Authorization header** → identifies the unique api key (the "Account" in
   Resin terms). Set on each outgoing chat-completion call. The
   `reverse_proxy_fixed_account_header` enum asks Resin to extract this
   value from up to the listed headers in order; whichever is found first
   becomes the per-request account. **Same key → same exit IP** is the
   default sticky session policy (Resin's token->account->IP binding,
   subject to `sticky_ttl`).
2. **Upstream Host header** → identifies the v1 endpoint (litellm/litellm
   upstream or other gateway front-end). `regex_filters` are **Go
   `regexp`** patterns matched against the inbound upstream URL host.

Both are HTTP header parsing — sub-millisecond at the proxy stage, exact,
unique. Resin's token->account hashing is in-process and native; the shell
never has to invent its own hashing.

## Comparison: why Resin beats the alternatives

| Approach | Identification | Latency | Re-key cost |
| --- | --- | --- | --- |
| **Resin header inspection (default here)** | Authorization + Host regex | sub-ms (HTTP header parse) | 0 — platform PATCH |
| LiteLLM router | statically declared model list in YAML | sub-ms | rebuild config file |
| Omniroute native routing | URL-path sentinel (/<model>/) | sub-ms | custom route table |
| Cooperative gateway sets `X-Account` header | the call sets `X-Account` explicitly | sub-ms | per-call change |

Resin header inspection is strictly the most ergonomic: omniroute/litellm
sets the **standard `Authorization` header** it already had to send to
OpenAI; Resin extracts and routes. The user just feeds omniroute the local
proxy port.

## How the Desktop UI surfaces this (Phase R1+R2 facts)

- The **Platform** card on the Topology canvas (column B) displays
  `regex_filters`, `allocation_policy`, and `region_filters`.
- The drag-to-connect wire from a Platform to a node-group-region (column C)
  is a hot PATCH `region_filters` on Resin.
- Resin's `reverse_proxy_fixed_account_header` + `regex_filters` +
  `allocation_policy` are all part of the v1.1.2 platform PATCH endpoint
  so they remain live-editable in the GUI without a restart. The desktop
  Settings > Config import/export surface (R4) keeps these changes
  reversible (`backup_create` fires before every topology drag → backup
  zip in `app_data_dir/backups`, so 防呆 is enforced).

## Where the desktop shell does **not** invent

- We do not implement our own key-hash or routing table. Resin owns it.
- We do not expose the admin token to the webview; the Rust-side ResinClient
  carries it.
- We do not store api keys anywhere in the desktop settings; Resin keeps the
  sticky → exit-IP mapping in its own SQLite state.db under
  `<app_data_dir>/resin-*/state.db`.
