import { decodeTasks, type Task } from "./index";
export interface AccountProfile {
  accountId: string;
  companionId: string;
}
export interface AccountSnapshot {
  port: number;
  status: "signed_out" | "authenticated" | "logout_pending";
  profile: AccountProfile | null;
  tasks: Task[];
  notice?: string | null;
}
export function decodeAccountSnapshot(value: unknown): AccountSnapshot {
  const invalid = () => new Error("账号响应不兼容，请更新客户端");
  if (!value || typeof value !== "object") throw invalid();
  const v = value as Record<string, unknown>;
  if (
    !Number.isInteger(v.port) ||
    (v.port as number) < 1024 ||
    (v.port as number) > 65535 ||
    !["signed_out", "authenticated", "logout_pending"].includes(
      v.status as string,
    )
  )
    throw invalid();
  const tasks = decodeTasks(v.tasks);
  if (
    v.notice !== undefined &&
    v.notice !== null &&
    (typeof v.notice !== "string" || v.notice.length > 512)
  )
    throw invalid();
  const p = v.profile as Record<string, unknown> | null;
  const uuid = (s: unknown): s is string =>
    typeof s === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s);
  if (v.status === "authenticated") {
    if (!p || !uuid(p.accountId) || !uuid(p.companionId)) throw invalid();
  } else if (p !== null || tasks.length !== 0) throw invalid();
  // Explicit projection: never spread arbitrary native fields into UI state.
  return {
    port: v.port as number,
    status: v.status as AccountSnapshot["status"],
    profile: p
      ? {
          accountId: p.accountId as string,
          companionId: p.companionId as string,
        }
      : null,
    tasks,
    ...(v.notice !== undefined ? { notice: v.notice as string | null } : {}),
  };
}
