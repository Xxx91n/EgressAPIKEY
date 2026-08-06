/**
 * Q4: Map Resin allocation_policy enum values to i18n keys.
 * Used at every display site (PlatformsView, TopologyView, NodesView).
 */
export function policyToI18nKey(policy: string): string {
  switch (policy) {
    case "BALANCED":
      return "policy.balanced";
    case "PREFER_LOW_LATENCY":
      return "policy.lowLatency";
    case "PREFER_IDLE_IP":
      return "policy.idleIp";
    default:
      return policy; // unknown - pass through as literal
  }
}