import { useEffect, useRef, useState } from "react";
import {
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
  const alive = useRef(false),
    acting = useRef(false),
    seq = useRef(0),
    expired = useRef(onExpired);
  expired.current = onExpired;
  function failed(e: unknown) {
    if (e instanceof ApiError && e.status === 401) expired.current();
    setNote(message(e));
  }
  async function refresh() {
    const id = ++seq.current;
    try {
      const rows = decodePairings(await api("/pairings"));
      if (alive.current && id === seq.current) {
        setPairs(rows);
        setPreview((p) =>
          p && rows.some((r) => r.id === p.id && r.status === "pending")
            ? p
            : null,
        );
      }
    } catch (e) {
      if (alive.current && id === seq.current) failed(e);
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
    window.addEventListener("online", tick);
    window.addEventListener("focus", tick);
    return () => {
      alive.current = false;
      seq.current++;
      clearInterval(timer);
      window.removeEventListener("online", tick);
      window.removeEventListener("focus", tick);
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
        先在电脑的「账号与共享待办」生成配对码。只接受同账号的另一登录会话。
      </p>
      <p className="hint">
        文档摘录协作 ·
        每次动作另行确认。执行尚未开放；配对不授予文件、聊天或记忆访问。退出或会话过期后需重新配对。
      </p>
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
          maxLength={32}
          pattern="[0-9a-f]{32}"
          autoComplete="off"
          onChange={(e) => {
            setCode(e.target.value.trim().toLowerCase());
            setPreview(null);
          }}
        />
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
                setNote("配对成功。手机执行尚未开放。");
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
