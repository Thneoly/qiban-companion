import { useEffect, useRef, useState } from "react";
import {
  capabilityLabels,
  decodePairings,
  decodePairing,
  pairingStatus,
  type Pairing,
} from "@companion/contracts";
import { api, ApiError, message } from "./api";
export function PairingPanel({ onExpired }: { onExpired: () => void }) {
  const [pairs, setPairs] = useState<Pairing[]>([]),
    [preview, setPreview] = useState<Pairing | null>(null);
  const [code, setCode] = useState(""),
    [name, setName] = useState("我的手机"),
    [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [unreachable, setUnreachable] = useState(false);
  const alive = useRef(false),
    acting = useRef(false),
    seq = useRef(0),
    expired = useRef(onExpired);
  expired.current = onExpired;
  function failed(e: unknown) {
    if (e instanceof ApiError && e.status === 401) expired.current();
    if (
      e instanceof ApiError &&
      ["pairing_denied", "pairing_conflict", "pairing_rate_limited"].includes(
        e.code,
      )
    )
      setPreview(null);
    setNote(message(e));
  }
  async function refresh() {
    const id = ++seq.current;
    try {
      const rows = decodePairings(await api("/pairings"));
      if (alive.current && id === seq.current) {
        setUnreachable(false);
        setPairs(rows);
        setPreview((p) =>
          p && rows.some((r) => r.id === p.id && r.status === "pending")
            ? p
            : null,
        );
      }
    } catch (e) {
      if (alive.current && id === seq.current) {
        // 服务不可达 ≠ 电脑离线：保留最后一轮成功数据并明确提示状态未知。
        if (e instanceof ApiError && (e.status === 0 || e.status === 503)) {
          setUnreachable(true);
        } else {
          setUnreachable(false);
          failed(e);
        }
      }
    }
  }
  useEffect(() => {
    alive.current = true;
    void refresh();
    const tick = () => {
      if (!acting.current && document.visibilityState === "visible")
        void refresh();
    };
    const timer = setInterval(tick, 10000);
    const visible = () => {
      if (document.visibilityState === "visible") tick();
    };
    window.addEventListener("online", tick);
    window.addEventListener("focus", tick);
    document.addEventListener("visibilitychange", visible);
    return () => {
      alive.current = false;
      seq.current++;
      clearInterval(timer);
      window.removeEventListener("online", tick);
      window.removeEventListener("focus", tick);
      document.removeEventListener("visibilitychange", visible);
    };
  }, []);
  async function run(work: () => Promise<void>) {
    if (acting.current) return;
    acting.current = true;
    seq.current++;
    setBusy(true);
    setNote("");
    try {
      await work();
    } catch (e) {
      if (alive.current) failed(e);
    } finally {
      acting.current = false;
      if (alive.current) setBusy(false);
    }
  }
  return (
    <section className="card pairing" aria-labelledby="mobile-pair-title">
      <h2 id="mobile-pair-title">连接电脑</h2>
      <p className="hint">
        先在电脑的「账号与共享待办」生成六位数字配对码。只接受同账号的另一登录会话。
      </p>
      <p className="hint">
        文档摘录协作 ·
        每次动作另行确认。支持确认电脑已分享的摘录；配对不授予任意文件、聊天或记忆访问。退出或会话过期后需重新配对。
      </p>
      {unreachable && (
        <p role="status" className="notice">
          无法连接服务，电脑在线状态未知；显示的是最后一次成功读取的结果。
        </p>
      )}
      {note && <p role="status">{note}</p>}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void run(async () => {
            const p = decodePairing(await api("/pairings/preview", { code }));
            if (alive.current) setPreview(p);
          });
        }}
      >
        <label htmlFor="pair-code">电脑配对码</label>
        <input
          id="pair-code"
          value={code}
          disabled={busy}
          required
          maxLength={12}
          inputMode="numeric"
          pattern="[0-9]{6}"
          placeholder="输入六位数字"
          autoComplete="off"
          onChange={(e) => {
            setCode(e.target.value.replace(/\s/g, "").slice(0, 6));
            setPreview(null);
          }}
        />
        <p className="hint">五分钟有效。累计输错五次后需等待五分钟再试。</p>
        <label htmlFor="phone-name">这台手机的名称</label>
        <input
          id="phone-name"
          value={name}
          disabled={busy}
          required
          maxLength={40}
          onChange={(e) => setName(e.target.value)}
        />
        <div className="session-actions">
          <button disabled={busy}>核对电脑与权限</button>
          <button type="button" disabled={busy} onClick={() => void refresh()}>
            刷新配对
          </button>
        </div>
      </form>
      {preview && (
        <div className="notice">
          <strong>{preview.desktopName}</strong>
          <p>电脑标识：{preview.desktopId}</p>
          <p>权限：文档摘录（每次动作另行确认）</p>
          <p>配对码有效至 {new Date(preview.expiresAt).toLocaleTimeString()}</p>
          <button
            disabled={busy}
            onClick={() =>
              void run(async () => {
                decodePairing(
                  await api("/pairings/accept", {
                    code,
                    pairingId: preview.id,
                    name,
                  }),
                );
                if (!alive.current) return;
                setCode("");
                setPreview(null);
                await refresh();
                setNote("配对成功。电脑分享摘录后，可在下方确认保存。");
              })
            }
          >
            确认配对这台电脑
          </button>
        </div>
      )}
      <ul className="task-list">
        {pairs.map((p) => (
          <li key={p.id}>
            <div>
              <p>
                {p.desktopName} → {p.controllerName || "等待手机"}
              </p>
              <small>{pairingStatus[p.status]}</small>
              {p.desktopOnline !== null &&
                ["pending", "active"].includes(p.status) && (
                  <small className="presence">
                    {p.desktopOnline
                      ? `电脑在线 · 最后联系 ${relative(p.desktopLastHeartbeatAt)} · 可执行：${capabilities(p)}（每次动作仍需确认）`
                      : `电脑离线 · 最后联系 ${relative(p.desktopLastHeartbeatAt) || "从未"}`}
                  </small>
                )}
            </div>
            {["pending", "active"].includes(p.status) && (
              <button
                disabled={busy}
                aria-label={`撤销配对 ${p.desktopName}`}
                onClick={() => {
                  if (
                    window.confirm(
                      "撤销后阻止新动作，已准入动作仍需核对停止结果。确认撤销？",
                    )
                  )
                    void run(async () => {
                      await api(`/pairings/${p.id}/revoke`, {
                        revision: p.revision,
                      });
                      if (alive.current) {
                        setPreview(null);
                        setCode("");
                        await refresh();
                        setNote("已撤销配对。");
                      }
                    });
                }}
              >
                撤销
              </button>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}

function relative(at: number | null): string {
  if (at === null) return "";
  // Phone clock skew only affects this display text; the online boolean is
  // decided by the server clock, never by this computation.
  const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));
  return seconds < 60 ? `${seconds} 秒前` : `${Math.round(seconds / 60)} 分钟前`;
}
function capabilities(p: Pairing): string {
  if (p.desktopCapabilities.length === 0) return "无";
  return p.desktopCapabilities
    .map((slug) => capabilityLabels[slug] ?? slug)
    .join("、");
}
