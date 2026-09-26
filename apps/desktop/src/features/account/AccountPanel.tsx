import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  decodeAccountSnapshot,
  decodeTask,
  validateTitle,
  type AccountSnapshot,
} from "@companion/contracts";
import { nativeDesktop } from "../../lib/surface";
import { errorMessage } from "../../lib/client";
import "./account.css";
import { RemoteDocuments } from "./RemoteDocuments";
import { PairingPanel } from "./PairingPanel";

const empty: AccountSnapshot = {
  port: 4318,
  status: "signed_out",
  profile: null,
  tasks: [],
};
const statusNote = (s: AccountSnapshot) =>
  s.status === "logout_pending"
    ? "已从本机退出。服务端撤销待连接恢复后完成；其他设备尚未确认退出。"
    : s.status === "authenticated"
      ? "已接续同一伙伴。共享待办每 10 秒更新，也可手动刷新。"
      : "登录与手机相同的受邀邮箱，接续账号中的伙伴和待办。";
export function AccountPanel() {
  const [snapshot, setSnapshot] = useState(empty);
  const [port, setPort] = useState("4318");
  const [email, setEmail] = useState("");
  const [code, setCode] = useState("");
  const [challenge, setChallenge] = useState(false);
  const [cooldown, setCooldown] = useState(0);
  const [title, setTitle] = useState("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState(
    nativeDesktop
      ? "正在读取账号会话…"
      : "请在原生桌面版登录；浏览器预览不读写系统凭据。",
  );
  const generation = useRef(0);
  const reads = useRef(0);
  const acting = useRef(false);
  const mounted = useRef(false);
  const pending = useRef<{ requestId: string; title: string } | null>(null);
  function accept(next: AccountSnapshot) {
    setSnapshot(next);
    setPort(String(next.port));
    setNote(next.notice || statusNote(next));
    if (next.status !== "authenticated") {
      setTitle("");
      pending.current = null;
    }
    if (next.status === "authenticated") {
      setChallenge(false);
      setCode("");
    }
  }
  function failure(e: unknown) {
    if (
      e &&
      typeof e === "object" &&
      "code" in e &&
      (e.code === "authentication_required" ||
        e.code === "logout_pending" ||
        e.code === "credentials")
    ) {
      generation.current++;
      setSnapshot((s) => ({
        ...s,
        status: e.code === "logout_pending" ? "logout_pending" : "signed_out",
        profile: null,
        tasks: [],
      }));
      setTitle("");
      pending.current = null;
    }
    setNote(errorMessage(e));
  }
  async function refresh() {
    const epoch = generation.current;
    const sequence = ++reads.current;
    try {
      const next = decodeAccountSnapshot(await invoke("account_snapshot"));
      if (
        mounted.current &&
        epoch === generation.current &&
        sequence === reads.current
      )
        accept(next);
    } catch (e) {
      if (
        mounted.current &&
        epoch === generation.current &&
        sequence === reads.current
      )
        failure(e);
    }
  }
  useEffect(() => {
    mounted.current = true;
    if (nativeDesktop) void refresh();
    return () => {
      mounted.current = false;
      generation.current++;
    };
  }, []);
  useEffect(() => {
    if (!nativeDesktop || snapshot.status === "signed_out") return;
    const resume = () => {
      if (!acting.current && document.visibilityState === "visible")
        void refresh();
    };
    const timer = setInterval(resume, 10000);
    window.addEventListener("focus", resume);
    window.addEventListener("online", resume);
    document.addEventListener("visibilitychange", resume);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", resume);
      window.removeEventListener("online", resume);
      document.removeEventListener("visibilitychange", resume);
    };
  }, [snapshot.status]);
  useEffect(() => {
    if (!cooldown) return;
    const timer = setTimeout(() => setCooldown(cooldown - 1), 1000);
    return () => clearTimeout(timer);
  }, [cooldown]);
  async function run(work: () => Promise<void>) {
    if (acting.current) return;
    acting.current = true;
    generation.current++;
    setBusy(true);
    try {
      await work();
    } catch (e) {
      if (e && typeof e === "object" && "code" in e && e.code === "conflict")
        await refresh();
      failure(e);
    } finally {
      acting.current = false;
      setBusy(false);
    }
  }
  async function requestCode() {
    await run(async () => {
      await invoke("account_code_request", { email });
      setChallenge(true);
      setCode("");
      setCooldown(60);
      setNote("如果邮箱在邀请名单内，验证码已发送，有效期 10 分钟。");
    });
  }
  async function login() {
    await run(async () => {
      accept(decodeAccountSnapshot(await invoke("account_login", { code })));
    });
  }
  async function logout(allSessions: boolean) {
    await run(async () => {
      // Clear visible data before waiting on the server; Rust persists the intent.
      setSnapshot((s) => ({
        ...s,
        status: "logout_pending",
        profile: null,
        tasks: [],
      }));
      setTitle("");
      pending.current = null;
      setChallenge(false);
      setCode("");
      setNote("正在退出并核对服务端撤销…");
      try {
        accept(
          decodeAccountSnapshot(
            await invoke("account_logout", { allSessions }),
          ),
        );
      } catch (e) {
        if (
          e &&
          typeof e === "object" &&
          "code" in e &&
          e.code === "logout_not_saved"
        )
          accept(snapshot);
        throw e;
      }
    });
  }
  return (
    <section className="account-panel" aria-labelledby="account-title">
      <div className="section-heading">
        <div>
          <span className="eyebrow">ONE COMPANION, EVERYWHERE</span>
          <h2 id="account-title">账号与共享待办</h2>
        </div>
        <button
          disabled={!nativeDesktop || busy}
          onClick={() => void refresh()}
        >
          刷新账号
        </button>
      </div>
      <p>
        用与手机相同的邮箱登录。这里的待办属于账号，可跨端查看；本地待办、聊天、记忆和外观仍留在本机。
      </p>
      <p className="account-notice" role="status">
        {note}
      </p>
      {snapshot.status === "authenticated" && snapshot.profile ? (
        <>
          <div className="account-identity">
            <strong>栖栖 · 已接续</strong>
            <dl>
              <dt>账号 ID</dt>
              <dd data-testid="desktop-account-id">
                {snapshot.profile.accountId}
              </dd>
              <dt>伙伴 ID</dt>
              <dd data-testid="desktop-companion-id">
                {snapshot.profile.companionId}
              </dd>
            </dl>
          </div>
          <PairingPanel key={snapshot.profile.accountId} onAuthError={failure} />
          <RemoteDocuments key={snapshot.profile.accountId} onAuthError={failure} />
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void run(async () => {
                const cleaned = validateTitle(title);
                if (!pending.current || pending.current.title !== cleaned)
                  pending.current = {
                    requestId: crypto.randomUUID(),
                    title: cleaned,
                  };
                decodeTask(
                  await invoke("account_task_create", pending.current),
                );
                pending.current = null;
                setTitle("");
                await refresh();
              });
            }}
          >
            <label htmlFor="shared-task-title">新增共享待办</label>
            <div className="account-compose">
              <input
                id="shared-task-title"
                required
                maxLength={200}
                disabled={busy}
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="让手机和电脑一起记住的小事"
              />
              <button className="primary" disabled={busy || !title.trim()}>
                保存共享待办
              </button>
            </div>
          </form>
          <p className="helper">这里只记录待办，不自动执行。</p>
          <ul className="account-tasks">
            {snapshot.tasks.map((task) => (
              <li key={task.id}>
                <div>
                  <strong>{task.title}</strong>
                  <small>
                    {task.status === "cancelled"
                      ? "已取消"
                      : task.status === "queued"
                        ? "已记下 · 等待处理"
                        : task.status}
                  </small>
                </div>
                {task.status === "queued" && (
                  <button
                    disabled={busy}
                    aria-label={`取消共享待办：${task.title}`}
                    onClick={() =>
                      void run(async () => {
                        decodeTask(
                          await invoke("account_task_cancel", {
                            id: task.id,
                            revision: task.revision,
                          }),
                        );
                        await refresh();
                      })
                    }
                  >
                    取消
                  </button>
                )}
              </li>
            ))}
          </ul>
          {!snapshot.tasks.length && (
            <p className="helper">
              账号中还没有待办。现有本地待办不会自动上传或合并。
            </p>
          )}
          <div className="account-actions">
            <button disabled={busy} onClick={() => void logout(false)}>
              退出此桌面会话
            </button>
            <button
              disabled={busy}
              onClick={() => {
                if (
                  window.confirm(
                    "退出这个账号的所有会话，包括手机和其他浏览器？",
                  )
                )
                  void logout(true);
              }}
            >
              退出所有设备
            </button>
          </div>
        </>
      ) : snapshot.status === "logout_pending" ? (
        <button disabled={busy} onClick={() => void run(refresh)}>
          重试服务端撤销
        </button>
      ) : (
        <>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void (challenge ? login() : requestCode());
            }}
          >
            <fieldset disabled={!nativeDesktop || busy}>
              <label htmlFor="account-email">账号邮箱</label>
              <input
                id="account-email"
                type="email"
                required
                maxLength={254}
                autoComplete="email"
                disabled={challenge}
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
              {challenge && (
                <>
                  <label htmlFor="account-code">8 位登录验证码</label>
                  <input
                    id="account-code"
                    required
                    inputMode="numeric"
                    autoComplete="one-time-code"
                    pattern="[0-9]{8}"
                    maxLength={8}
                    value={code}
                    onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))}
                  />
                </>
              )}
              <div className="account-actions">
                <button
                  className="primary"
                  disabled={!challenge && cooldown > 0}
                >
                  {busy
                    ? "请稍候…"
                    : challenge
                      ? "登录并接续伙伴"
                      : cooldown
                        ? `${cooldown} 秒后可重发`
                        : "获取登录验证码"}
                </button>
                {challenge && (
                  <>
                    <button
                      type="button"
                      disabled={cooldown > 0}
                      onClick={() => void requestCode()}
                    >
                      {cooldown ? `${cooldown} 秒后重发` : "重新发送验证码"}
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        setChallenge(false);
                        setCode("");
                      }}
                    >
                      更换邮箱
                    </button>
                  </>
                )}
              </div>
            </fieldset>
          </form>
          <details>
            <summary>本机账号服务连接</summary>
            <p className="helper">
              先运行 npm run coordinator。当前连接这台电脑上的协调服务；手机通过
              HTTPS 入口连接同一服务。
            </p>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                void run(async () => {
                  await invoke("account_port_save", { port: Number(port) });
                  setChallenge(false);
                  setCode("");
                  await refresh();
                });
              }}
            >
              <label htmlFor="account-port">协调服务端口</label>
              <input
                id="account-port"
                type="number"
                required
                min={1024}
                max={65535}
                value={port}
                disabled={!nativeDesktop || busy}
                onChange={(e) => setPort(e.target.value)}
              />
              <button disabled={!nativeDesktop || busy}>保存连接</button>
            </form>
          </details>
        </>
      )}
      <p className="helper">
        会话通过系统安全凭据存储保存。换设备或系统时重新登录同一服务，伙伴身份保持一致；会话令牌不参与数据导出。
      </p>
    </section>
  );
}
