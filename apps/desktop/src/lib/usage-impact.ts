import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { decodeUsageImpactReport, type UsageImpactReport } from '@companion/contracts';

/** Consultative turn precount for memory confirm dialogs. `active` gates the
 * invoke so panels only consult while the dialog is actually open, and the
 * report refreshes when the queried ids change (dialog reopened for another
 * entry). Fails open on any error or protocol mismatch: the dialog keeps its
 * static wording, and the commit receipt's clearedTurns stays the
 * authoritative number either way. */
export function useUsageImpact(active: boolean, appIds: readonly string[], personalIds: readonly number[]): UsageImpactReport | null {
  const [report, setReport] = useState<UsageImpactReport | null>(null);
  // Callers derive these arrays during render, so raw array identity would
  // retrigger the effect every render; ids contain no commas, keys are exact.
  const appKey = appIds.join(','), personalKey = personalIds.join(',');
  useEffect(() => {
    if (!active) { setReport(null); return; }
    let disposed = false;
    setReport(null);
    void invoke('chat_usage_impact', { request: { appIds: appKey ? appKey.split(',') : [], personalIds: personalKey ? personalKey.split(',').map(Number) : [] } })
      .then(value => { if (!disposed) setReport(decodeUsageImpactReport(value)); })
      .catch(() => { if (!disposed) setReport(null); });
    return () => { disposed = true; };
  }, [active, appKey, personalKey]);
  return report;
}
