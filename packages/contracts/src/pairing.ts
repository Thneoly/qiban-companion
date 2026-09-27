export interface Pairing {
  id: string;
  desktopId: string;
  desktopName: string;
  controllerId: string | null;
  controllerName: string | null;
  scope: "document_excerpt";
  revision: number;
  status: "pending" | "active" | "revoked" | "expired";
  expiresAt: number;
  currentRole: "desktop" | "controller" | "observer";
  /** Presence lease, display-only (never gates authorization). null = the
   * coordinator predates presence (状态未知), not "offline". */
  desktopOnline: boolean | null;
  desktopLastHeartbeatAt: number | null;
  desktopCapabilities: string[];
}
export interface PairingOffer {
  pairing: Pairing;
  code: string;
}
export function decodePairing(input: unknown): Pairing {
  const v = input as Record<string, unknown> | null;
  const uuid = (v: unknown): v is string =>
    typeof v === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(v);
  const label = (v: unknown): v is string =>
    typeof v === "string" && v.length > 0 && [...v].length <= 40;
  if (
    !v ||
    !uuid(v.id) ||
    !uuid(v.desktopId) ||
    !label(v.desktopName) ||
    !(v.controllerId === null || uuid(v.controllerId)) ||
    !(v.controllerName === null || label(v.controllerName)) ||
    v.scope !== "document_excerpt" ||
    !Number.isSafeInteger(v.revision) ||
    (v.revision as number) < 1 ||
    !Number.isSafeInteger(v.expiresAt) ||
    (v.expiresAt as number) < 0 ||
    !["pending", "active", "revoked", "expired"].includes(v.status as string) ||
    !["desktop", "controller", "observer"].includes(v.currentRole as string) ||
    (v.controllerId === null) !== (v.controllerName === null) ||
    (v.status === "active" && v.controllerId === null) ||
    !("desktopOnline" in v ? typeof v.desktopOnline === "boolean" : true) ||
    !(
      "desktopLastHeartbeatAt" in v
        ? v.desktopLastHeartbeatAt === null ||
          (Number.isSafeInteger(v.desktopLastHeartbeatAt) &&
            (v.desktopLastHeartbeatAt as number) >= 0)
        : true
    ) ||
    !(
      "desktopCapabilities" in v
        ? Array.isArray(v.desktopCapabilities) &&
          v.desktopCapabilities.length <= 8 &&
          v.desktopCapabilities.every(
            (slug) => typeof slug === "string" && /^[a-z][a-z0-9_]{0,31}$/.test(slug),
          )
        : true
    )
  )
    throw new Error("配对响应不兼容，请更新客户端");
  return {
    id: v.id,
    desktopId: v.desktopId,
    desktopName: v.desktopName,
    controllerId: v.controllerId as string | null,
    controllerName: v.controllerName as string | null,
    scope: v.scope,
    revision: v.revision as number,
    status: v.status as Pairing["status"],
    expiresAt: v.expiresAt as number,
    currentRole: v.currentRole as Pairing["currentRole"],
    desktopOnline: "desktopOnline" in v ? (v.desktopOnline as boolean) : null,
    desktopLastHeartbeatAt:
      "desktopLastHeartbeatAt" in v ? (v.desktopLastHeartbeatAt as number | null) : null,
    desktopCapabilities: "desktopCapabilities" in v ? [...(v.desktopCapabilities as string[])] : [],
  };
}
export function decodePairings(v: unknown): Pairing[] {
  if (!Array.isArray(v) || v.length > 100) throw new Error("配对列表不兼容");
  return v.map(decodePairing);
}
export function decodePairingOffer(v: unknown): PairingOffer {
  const input = v as Record<string, unknown>;
  if (
    !input ||
    typeof input.code !== "string" ||
    !/^[0-9]{6}$/.test(input.code)
  )
    throw new Error("配对码无效");
  return { pairing: decodePairing(input.pairing), code: input.code };
}
export const pairingStatus = {
  pending: "等待手机确认",
  active: "已配对",
  revoked: "已撤销",
  expired: "已失效，请重新配对",
};
/** Display labels for capability slugs. Open set: unknown future slugs fall
 * back to the raw slug instead of failing the decode. */
export const capabilityLabels: Record<string, string> = {
  document_excerpt: "文档摘录",
};
