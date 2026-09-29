import type { Pairing } from "./pairing";

export interface ActionBinding {
  actionId: string;
  resourceId: string;
  resourceVersion: number;
  parametersDigest: string;
  pairRevision: number;
  scope: "document_excerpt";
}
export interface DocumentAction {
  authorization: {
    pairingId: string;
    binding: ActionBinding;
    expiresAt: number;
    state: string;
  };
  sourceName: string;
  preview: string;
  artifactHash: string;
  currentRole: "desktop" | "controller";
}
export const documentStates: Record<string, string> = {
  awaiting_confirmation: "等待手机确认",
  confirmed: "已确认，等待电脑处理",
  admitted: "已准入，等待保存回执",
  cancel_requested: "请求停止，等待电脑核对",
  cancelled: "已取消",
  completed: "已保存并核验",
  failed: "未保存成功",
  unknown: "结果未知，需要核对",
};
export function decodeDocument(value: unknown): DocumentAction {
  const v = value as DocumentAction;
  const a = v?.authorization,
    b = a?.binding;
  const uuid = (s: unknown) =>
    typeof s === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s);
  const hash = (s: unknown) =>
    typeof s === "string" && /^[0-9a-f]{64}$/.test(s);
  if (
    !a ||
    !b ||
    !uuid(a.pairingId) ||
    !uuid(b.actionId) ||
    !uuid(b.resourceId) ||
    b.resourceVersion !== 1 ||
    !Number.isSafeInteger(b.pairRevision) ||
    b.pairRevision < 1 ||
    b.scope !== "document_excerpt" ||
    !hash(b.parametersDigest) ||
    !hash(v.artifactHash) ||
    !Number.isSafeInteger(a.expiresAt) ||
    !Object.hasOwn(documentStates, a.state) ||
    typeof v.sourceName !== "string" ||
    v.sourceName.length > 300 ||
    typeof v.preview !== "string" ||
    v.preview.length > 12288 ||
    !["desktop", "controller"].includes(v.currentRole)
  )
    throw Error("文档协作响应不兼容");
  return {
    authorization: {
      pairingId: a.pairingId,
      binding: {
        actionId: b.actionId,
        resourceId: b.resourceId,
        resourceVersion: b.resourceVersion,
        parametersDigest: b.parametersDigest,
        pairRevision: b.pairRevision,
        scope: b.scope,
      },
      expiresAt: a.expiresAt,
      state: a.state,
    },
    sourceName: v.sourceName,
    preview: v.preview,
    artifactHash: v.artifactHash,
    currentRole: v.currentRole,
  };
}
export function decodeDocuments(v: unknown): DocumentAction[] {
  if (!Array.isArray(v) || v.length > 20) throw Error("文档协作列表不兼容");
  return v.map(decodeDocument);
}

/** Phone-side task phases: server states plus the presence-derived split of
 * confirmed → waiting_desktop and admitted → executing. "executing" is
 * inferred from the pairing lease, never a server claim. */
export type DocumentPhaseKey =
  | "awaiting_confirmation"
  | "waiting_desktop"
  | "executing"
  | "completed"
  | "failed"
  | "cancel_requested"
  | "cancelled"
  | "unknown";

export interface DocumentPhase {
  key: DocumentPhaseKey;
  label: string;
  hint: string;
}

/** "N 秒前" / "N 分钟前" for the last-contact stamp. Phone clock skew only
 * affects this display text; the online boolean is decided by the server
 * clock, never by this computation. */
export function relativeTime(at: number | null, now: number): string {
  if (at === null) return "";
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  return seconds < 60 ? `${seconds} 秒前` : `${Math.round(seconds / 60)} 分钟前`;
}

function windowSuffix(expiresAt: number, now: number): {
  suffix: string;
  expired: boolean;
} {
  if (expiresAt <= now) return { suffix: "（已到有效期）", expired: true };
  const left = Math.floor((expiresAt - now) / 1000);
  const m = Math.floor(left / 60);
  const s = String(left % 60).padStart(2, "0");
  return { suffix: `（剩余 ${m}:${s}）`, expired: false };
}

const expiredNote = "已到有效期，以服务端刷新核对为准。";

/** Derive the phone display phase from server state + the pairing presence
 * lease. pairing null/undefined (not fetched, join miss, or an old
 * coordinator with desktopOnline null) means NO presence decoration — never
 * an offline claim. The phone never fabricates a terminal state. */
export function deriveDocumentPhase(
  doc: DocumentAction,
  pairing: Pairing | null | undefined,
  now: number,
): DocumentPhase {
  const online =
    pairing && pairing.id === doc.authorization.pairingId
      ? pairing.desktopOnline
      : null;
  const contact = relativeTime(
    pairing && pairing.id === doc.authorization.pairingId
      ? pairing.desktopLastHeartbeatAt
      : null,
    now,
  ) || "从未";
  const { suffix, expired } = windowSuffix(doc.authorization.expiresAt, now);
  const note = expired ? expiredNote : "";
  switch (doc.authorization.state) {
    case "awaiting_confirmation":
      return {
        key: "awaiting_confirmation",
        label: `等待手机确认${suffix}`,
        hint: `请在有效期内确认；过期未确认将自动取消，电脑不会执行。本次确认只授权这一份摘录。${note}`,
      };
    case "confirmed":
      return {
        key: "waiting_desktop",
        label: `已确认，等待电脑保存${suffix}`,
        hint:
          (online === null
            ? "电脑在线状态未知（服务未提供或尚未取到），可点刷新核对。"
            : online
              ? "电脑在线，一般数秒内开始；确认不等于已执行。"
              : `电脑当前离线（最后联系 ${contact}）；电脑需在有效期内联网执行，过期将自动取消。`) + note,
      };
    case "admitted":
      return online === null
        ? {
            key: "executing",
            label: "已准入，等待保存回执",
            hint: "执行状态未知；结果以电脑回报为准。",
          }
        : online
          ? {
              key: "executing",
              label: "执行中（推断）",
              hint: "电脑已取走任务且在线；执行结果以电脑回报为准。",
            }
          : {
              key: "executing",
              label: "已开始，电脑离线",
              hint: `保存可能正在进行或已中断；电脑恢复联网后会自动核对并回报（最后联系 ${contact}）。`,
            };
    case "cancel_requested":
      return {
        key: "cancel_requested",
        label: "已请求停止，等待电脑核对",
        hint:
          online === null
            ? "停止结果以电脑回报为准，可刷新核对。"
            : online
              ? "电脑在线时会核对停止结果并回报。"
              : `电脑当前离线，恢复联网后会核对停止结果（最后联系 ${contact}）。`,
      };
    case "completed":
      return {
        key: "completed",
        label: "已保存并核验",
        hint: "电脑已回报完成；回报的产物哈希与这份预览的摘要一致（服务端接收回报时核对）。展开下方可核对保存结果。",
      };
    case "failed":
      return {
        key: "failed",
        label: "未保存成功",
        hint: "电脑回报未保存成功；如需保存请让电脑重新分享一份。",
      };
    case "cancelled":
      return {
        key: "cancelled",
        label: "已取消",
        hint: "电脑不会执行这份摘录。",
      };
    case "unknown":
      return online === null
        ? {
            key: "unknown",
            label: "结果未知，需要核对",
            hint: "结果以电脑回报为准，可刷新核对。",
          }
        : online
          ? {
              key: "unknown",
              label: "结果未知，待自动核对",
              hint: "电脑在线，稍后会自动核对并更新；可点刷新查看。",
            }
          : {
              key: "unknown",
              label: "结果未知，待电脑核对",
              hint: `电脑当前离线；恢复联网后会自动核对，无需重新确认（最后联系 ${contact}）。`,
            };
    default:
      // decode whitelists states; this only guards a future state string.
      return {
        key: "unknown",
        label: documentStates[doc.authorization.state] ?? doc.authorization.state,
        hint: "状态以服务端回报为准，可刷新核对。",
      };
  }
}
