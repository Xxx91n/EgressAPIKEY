/// Subscription-fold node grouping, shared by NodesView (node pool) and
/// PlatformsView (manual-node picker). A node's subscription name is the
/// first tag's subscription_name/subscriptionName/tag fallback; untagged
/// nodes group under "__untagged__" (rendered via the `untagged` locale key).
///
/// Extracted in r12 wave-d (D4 hygiene): the pattern had drifted into two
/// copies (subName/groupBySub in NodesView, platformSubName/
/// platformGroupBySub in PlatformsView).
export interface SubTagged {
  tags?: ReadonlyArray<{
    subscription_name?: string;
    subscriptionName?: string;
    tag?: string;
  }> | null;
}

export function subName(n: SubTagged): string {
  return n.tags?.[0]?.subscription_name ?? n.tags?.[0]?.subscriptionName ?? n.tags?.[0]?.tag ?? "";
}

export function groupBySub<T extends SubTagged>(nodes: T[]): Map<string, T[]> {
  const m = new Map<string, T[]>();
  for (const n of nodes) {
    const key = subName(n) || "__untagged__";
    const arr = m.get(key);
    if (arr) arr.push(n);
    else m.set(key, [n]);
  }
  return m;
}
