/**
 * T14-5: useIpc — unified SWR-backed IPC data hook for Tauri.
 *
 * Wraps SWR around Tauri IPC invoke() calls for automatic:
 * - request dedup (dedupingInterval: 10s default)
 * - cache (stale-while-revalidate)
 * - error retry (SWR internal)
 * - pause when tab hidden (refreshWhenHidden: false)
 *
 * Usage:
 *   const { data, error, isLoading, mutate } = useIpc("platforms", () => ipcPlatformList());
 *   const { data: nodes } = useIpc("nodes", () => ipcNodeList(), { refreshInterval: 10000 });
 *
 * Outside Tauri (vitest), the fetcher throws and SWR silently returns isLoading=false, error set.
 */

import useSWR, { type SWRConfiguration } from "swr";

export function useIpc<T>(
  key: string,
  fetcher: () => Promise<T>,
  opts?: SWRConfiguration<T>
) {
  return useSWR<T>(key, fetcher, {
    // T14-5 defaults: 10s dedup, no refetch on hidden, error retry 2x
    dedupingInterval: 10000,
    revalidateOnFocus: true,
    revalidateOnReconnect: true,
    refreshWhenHidden: false,
    shouldRetryOnError: false,
    errorRetryCount: 0,
    keepPreviousData: true,
    ...opts,
  });
}
