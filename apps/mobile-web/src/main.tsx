import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { api, ApiError, message, nonce, type Profile, type Task } from "./api";
import "./style.css";
import { DocumentPanel } from "./DocumentPanel";
import { PairingPanel } from "./PairingPanel";

function Companion() {
  return (
    <svg
      className="companion"
      viewBox="0 0 220 190"
      role="img"
      aria-label="栖栖，随身陪伴你的小伙伴"
    >
      <ellipse cx="110" cy="167" rx="66" ry="10" fill="#d5dfcc" />
      <path
        d="M57 76 Q25 14 76 39 Q110 20 144 39 Q195 14 163 76"
        fill="#e8c58e"
        stroke="#665747"
        strokeWidth="3"
      />
      <path
        d="M51 97 Q46 45 110 44 Q174 45 169 97 L164 130 Q162 163 110 165 Q58 163 56 130Z"
        fill="#fff4df"
        stroke="#665747"
        strokeWidth="3"
      />
      <ellipse cx="84" cy="98" rx="5" ry="7" fill="#493e36" />
      <ellipse cx="137" cy="98" rx="5" ry="7" fill="#493e36" />
      <path
        d="M102 113 Q110 121 118 113"
        fill="none"
        stroke="#665747"
        strokeWidth="3"
        strokeLinecap="round"
      />
      <ellipse cx="72" cy="111" rx="10" ry="5" fill="#ecc0ac" />
      <ellipse cx="148" cy="111" rx="10" ry="5" fill="#ecc0ac" />
      <path d="M98 44 Q102 20 125 25 Q124 45 98 44" fill="#86a77a" />
      <path
        d="M86 150 Q110 135 134 150"
        fill="none"
        stroke="#dfcba8"
        strokeWidth="3"
      />
    </svg>
  );
}

function App() {
  const [profile, setProfile] = useState<Profile | null>(null);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [ready, setReady] = useState(false);
  const [email, setEmail] = useState("");
  const [challenge, setChallenge] = useState("");
  const [code, setCode] = useState("");
  const [title, setTitle] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [synced, setSynced] = useState("");
  const [cooldown, setCooldown] = useState(0);
  const epoch = useRef(0);
  const refreshSequence = useRef(0);
  const acting = useRef(false);
  const active = useRef(true);
  const loginNonce = useRef("");
  const pendingTask = useRef<{ requestId: string; title: string } | null>(null);
  function clearSession() {
    epoch.current++;
    setProfile(null);
    setTasks([]);
    setTitle("");
    setSynced("");
    pendingTask.current = null;
  }
  async function refresh(initial = false) {
    const sequence = ++refreshSequence.current;
    const generation = epoch.current;
    try {
      const { profile: p, tasks: t } = await api<{
        profile: Profile;
        tasks: Task[];
      }>("/state");
      if (
        generation !== epoch.current ||
        sequence !== refreshSequence.current ||
        !active.current
      )
        return;
      setProfile(p);
      setTasks(t);
      setSynced(new Date().toLocaleTimeString("zh-CN"));
      setError("");
    } catch (e) {
      if (
        generation !== epoch.current ||
        sequence !== refreshSequence.current ||
        !active.current
      )
        return;
      if (e instanceof ApiError && e.status === 401) {
        clearSession();
        if (!initial) setError(message(e));
      } else setError(message(e));
    } finally {
      if (active.current) setReady(true);
    }
  }
  useEffect(() => {
    active.current = true;
    void refresh(true);
    return () => {
      active.current = false;
      epoch.current++;
    };
  }, []);
  useEffect(() => {
    if (!profile) return;
    const resume = () => {
      if (document.visibilityState === "visible" && !busy) void refresh();
    };
    const timer = window.setInterval(resume, 10000);
    window.addEventListener("online", resume);
    window.addEventListener("focus", resume);
    document.addEventListener("visibilitychange", resume);
    return () => {
      clearInterval(timer);
      window.removeEventListener("online", resume);
      window.removeEventListener("focus", resume);
      document.removeEventListener("visibilitychange", resume);
    };
  }, [profile?.accountId, busy]);
  useEffect(() => {
    if (!cooldown) return;
    const timer = setTimeout(() => setCooldown(cooldown - 1), 1000);
    return () => clearTimeout(timer);
  }, [cooldown]);
  async function action(work: () => Promise<void>) {
    if (acting.current) return;
    acting.current = true;
    epoch.current++;
    setBusy(true);
    setError("");
    try {
      await work();
    } catch (e) {
      if (e instanceof ApiError && e.code === "authentication_required")
        clearSession();
      if (e instanceof ApiError && e.code === "conflict") await refresh();
      setError(message(e));
    } finally {
      acting.current = false;
      setBusy(false);
    }
  }
  async function requestCode() {
    await action(async () => {
      const result = await api<{ challengeId: string }>("/auth/request-code", {
        email: email.trim(),
      });
      setChallenge(result.challengeId);
      loginNonce.current = nonce();
      setCode("");
      setCooldown(60);
    });
  }
  async function login() {
    await action(async () => {
      await api("/auth/verify-code", {
        challengeId: challenge,
        code,
        nonce: loginNonce.current,
      });
      setCode("");
      setChallenge("");
      loginNonce.current = "";
      await refresh();
    });
  }
  async function createTask() {
    await action(async () => {
      if (!pendingTask.current || pendingTask.current.title !== title.trim())
        pendingTask.current = {
          requestId: crypto.randomUUID(),
          title: title.trim(),
        };
      await api("/tasks", pendingTask.current);
      pendingTask.current = null;
      setTitle("");
      await refresh();
    });
  }
  async function logout(allSessions: boolean) {
    await action(async () => {
      // Invalidate in-flight refreshes before revocation so old results cannot repaint.
      epoch.current++;
      await api("/logout", { allSessions });
      clearSession();
      setCode("");
      setChallenge("");
      loginNonce.current = "";
    });
  }
  return (
    <main>
      <header>
        <a className="brand" href="/">
          栖伴<span>QIBAN</span>
        </a>
        <span className="edition">随身伙伴 · 联调版</span>
      </header>
      <section className="hero">
        <div className="intro">
          <p className="eyebrow">把小天地，带在身边</p>
          <h1>
            {profile ? (
              <>
                <span>换个屏幕，</span>
                <span>我还在。</span>
              </>
            ) : (
              <>
                <span>走到哪里，</span>
                <span>都有栖栖。</span>
              </>
            )}
          </h1>
          <p>
            {profile
              ? "想到的事，我们一起记住。"
              : "用同一个邮箱登录，接续同一个伙伴和待办。"}
          </p>
        </div>
        <Companion />
      </section>
      <div className="notice" role="status" aria-live="polite">
        {error ||
          (!ready
            ? "正在找回你的伙伴…"
            : profile
              ? `已同步 · ${synced} · 页面打开时每 10 秒更新`
              : "仅对受邀邮箱开放。验证码有效期为 10 分钟。")}
      </div>
      {!ready ? (
        <button onClick={() => void refresh(true)}>重新连接</button>
      ) : !profile ? (
        <section className="card login">
          <p className="eyebrow">第一次见，或好久不见</p>
          <h2>接回你的伙伴</h2>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void (challenge ? login() : requestCode());
            }}
          >
            <label htmlFor="email">邮箱</label>
            <input
              id="email"
              type="email"
              autoComplete="email"
              required
              maxLength={254}
              value={email}
              disabled={busy || !!challenge}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="你已配置的受邀邮箱"
            />
            {challenge && (
              <>
                <p className="hint">
                  如果邮箱在邀请名单内，验证码将发送到你的收件箱。
                </p>
                <label htmlFor="code">8 位验证码</label>
                <input
                  id="code"
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  pattern="[0-9]{8}"
                  maxLength={8}
                  required
                  value={code}
                  onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))}
                />
              </>
            )}
            <button
              className="primary"
              disabled={busy || (!challenge && cooldown > 0)}
            >
              {busy
                ? "请稍候…"
                : challenge
                  ? "与栖栖会合"
                  : cooldown
                    ? `${cooldown} 秒后可重发`
                    : "获取验证码"}
            </button>
            {challenge && (
              <div className="row">
                <button
                  type="button"
                  disabled={busy || cooldown > 0}
                  onClick={() => void requestCode()}
                >
                  {cooldown ? `${cooldown} 秒后重发` : "重新获取验证码"}
                </button>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => {
                    setChallenge("");
                    setCode("");
                    loginNonce.current = "";
                  }}
                >
                  更换邮箱
                </button>
              </div>
            )}
          </form>
        </section>
      ) : (
        <div className="dashboard">
          <section className="card partner">
            <p className="eyebrow">我们的连接</p>
            <h2>
              栖栖 <span className="tag">已接续</span>
            </h2>
            <p>
              电脑与手机打开同一链接、登录同一邮箱，就能看到这里的同一份待办。
            </p>
            <details>
              <summary>核对账号与伙伴标识</summary>
              <dl>
                <dt>账号 ID</dt>
                <dd data-testid="account-id">{profile.accountId}</dd>
                <dt>伙伴 ID</dt>
                <dd data-testid="companion-id">{profile.companionId}</dd>
              </dl>
            </details>
            <p className="hint">
              当前共享网页待办。原生桌面本地待办、聊天、记忆和外观尚未接入。
            </p>
            <div className="session-actions">
              <button disabled={busy} onClick={() => void logout(false)}>
                退出本设备
              </button>
              <button
                disabled={busy}
                onClick={() => {
                  if (window.confirm("退出这个账号在所有设备上的登录？"))
                    void logout(true);
                }}
              >
                退出所有设备
              </button>
            </div>
          </section>
          <PairingPanel key={profile.accountId} onExpired={clearSession} />
          <DocumentPanel key={profile.accountId} onExpired={clearSession} />
          <section className="card todos">
            <div className="section-head">
              <div>
                <p className="eyebrow">一点一点，慢慢来</p>
                <h2>
                  我们的待办{" "}
                  <span className="count">
                    {tasks.filter((t) => t.status !== "cancelled").length}
                  </span>
                </h2>
              </div>
              <button disabled={busy} onClick={() => void refresh()}>
                刷新
              </button>
            </div>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                void createTask();
              }}
            >
              <label htmlFor="task">想让栖栖记住什么？</label>
              <div className="compose">
                <input
                  id="task"
                  value={title}
                  maxLength={200}
                  required
                  disabled={busy}
                  onChange={(e) => setTitle(e.target.value)}
                  placeholder="例如：周末整理旅行清单"
                />
                <button className="primary" disabled={busy || !title.trim()}>
                  记下来
                </button>
              </div>
            </form>
            <p className="hint">这里只记录待办，暂不自动执行。</p>
            <ul className="task-list">
              {tasks.map((task) => (
                <li
                  key={task.id}
                  className={task.status === "cancelled" ? "cancelled" : ""}
                >
                  <span className="task-dot" />
                  <div>
                    <p>{task.title}</p>
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
                      aria-label={`取消 ${task.title}`}
                      onClick={() =>
                        void action(async () => {
                          await api(`/tasks/${task.id}/cancel`, {
                            revision: task.revision,
                          });
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
            {!tasks.length && (
              <div className="empty">
                还没有待办。
                <br />
                <span>把第一件小事交给我记着吧。</span>
              </div>
            )}
          </section>
        </div>
      )}
      <footer>栖伴 · 屏幕变了，陪伴继续。</footer>
    </main>
  );
}
createRoot(document.getElementById("root")!).render(<App />);
