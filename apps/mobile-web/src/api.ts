export type Profile = { accountId: string; companionId: string };
export type Task = {
  id: string;
  title: string;
  status: string;
  revision: number;
  createdAt: number;
  updatedAt: number;
};
export class ApiError extends Error {
  constructor(
    public code: string,
    public status: number,
  ) {
    super(code);
  }
}
export async function api<T>(path: string, body?: unknown): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`/api${path}`, {
      method: body === undefined ? "GET" : "POST",
      credentials: "same-origin",
      cache: "no-store",
      redirect: "error",
      signal: AbortSignal.timeout(15000),
      headers:
        body === undefined
          ? {}
          : { "Content-Type": "application/json", "X-Qiban-Request": "1" },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
  } catch {
    throw new ApiError("network", 0);
  }
  let value;
  try {
    value = await response.json();
  } catch {
    throw new ApiError("service_unavailable", 503);
  }
  if (!response.ok)
    throw new ApiError(
      value?.error?.code ?? "service_unavailable",
      response.status,
    );
  return value as T;
}
export function nonce() {
  return btoa(
    String.fromCharCode(...crypto.getRandomValues(new Uint8Array(32))),
  )
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replaceAll("=", "");
}
export function message(error: unknown) {
  const messages: Record<string, string> = {
    pairing_rate_limited: "配对码核对失败次数过多，请五分钟后重新生成配对码再试。",
    pairing_denied: "配对码已失效，或当前登录没有此权限。请使用同账号的另一会话重新配对。",
    pairing_conflict: "配对状态已变化，请刷新后重新核对。",
    pairing_capacity: "配对记录已达上限。",
    network: "连接中断，操作结果尚未确认。恢复网络后重试即可。",
    authentication_required: "登录已过期或被退出，请重新登录。",
    invalid_code: "验证码不正确或已过期，请检查后重试。",
    rate_limited: "请求较频繁，请稍后再试。验证码每分钟最多请求一次。",
    conflict: "这条待办已在其他设备更新，已重新获取列表。",
    capacity: "待办数量已达上限。",
    invalid_request: "请检查填写内容。待办最多 200 字。",
    busy: "服务繁忙，请稍后重试。",
  };
  return error instanceof ApiError
    ? (messages[error.code] ??
        "服务暂时不可用，请确认电脑上的服务仍在运行，然后重试。")
    : "操作未完成，请重试。";
}
