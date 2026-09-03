# Resin upstream API gaps (shell wants, Resin does not provide)

> Round 5 spec §6: shell-side only this round; Resin-side contract changes
> land here as documentation, next round opens the Resin-side ticket.

## GAP-01 — subscription update_interval floor blocks sub-30s defaults

- **Status**: BLOCKED (upstream contract), raised by round5 T04
- **Want**: shell-side `subscription_add` default `update_interval: "5s"`
  so an imported subscription's first fetch is visible within seconds
  (round5 spec §2.2 T04 F2), plus a one-time boot migration refreshing
  existing subscriptions to 5s.
- **Upstream reality**: Resin v1.2.0 enforces `update_interval >= 30s` on
  BOTH `POST /api/v1/subscriptions` (create, control_plane_subscription.go
  :188-192) and `PATCH /api/v1/subscriptions/{id}` (update, :324-330) via
  `minSubscriptionUpdateInterval = 30 * time.Second` (:130); a smaller
  value is rejected with 400 "update_interval: must be >= 30s".
- **Shell behavior now**: default stays 30s; a 5s POST is never sent by
  default and the new write-path retry never re-POSTs a 400 body.
- **Resolution path**: Resin-side ticket to lower the floor (or expose a
  force-refresh endpoint, which would also unblock the refresh-latency
  chain); after the floor drops, shell can flip the default and add the
  boot migration in one round5-followup ticket.
