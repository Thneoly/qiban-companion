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
