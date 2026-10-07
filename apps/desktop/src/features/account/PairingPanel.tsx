import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  decodePairings,
  decodePairingOffer,
  pairingStatus,
  type Pairing,
  type PairingOffer,
} from "@companion/contracts";
import { errorMessage } from "../../lib/client";
export function PairingPanel({
  onAuthError,
}: {
  onAuthError: (e: unknown) => void;
}) {
  const [pairs, setPairs] = useState<Pairing[]>([]);
  const [offer, setOffer] = useState<PairingOffer | null>(null);
  const [name, setName] = useState("我的电脑");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const alive = useRef(false),
    acting = useRef(false),
    sequence = useRef(0);
  const authError = useRef(onAuthError);
  authError.current = onAuthError;
  function failed(e: unknown) {
    if (
      e &&
      typeof e === "object" &&
      "code" in e &&
      ["authentication_required", "logout_pending", "credentials"].includes(
        String(e.code),
      )
    )
      authError.current(e);
    setNote(errorMessage(e));
  }
  async function refresh() {
    const id = ++sequence.current;
    try {
      const rows = decodePairings(await invoke("account_pairings"));
      if (!alive.current || id !== sequence.current) return;
      setPairs(rows);
      setNote("");
      setOffer((old) =>
        old &&
        rows.some((p) => p.id === old.pairing.id && p.status === "pending")
          ? old
          : null,
      );
    } catch (e) {
      if (alive.current && id === sequence.current) failed(e);
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
      sequence.current++;
      clearInterval(timer);
      window.removeEventListener("online", tick);
      window.removeEventListener("focus", tick);
    };
  }, []);
  async function run(work: () => Promise<void>) {
    if (acting.current) return;
    acting.current = true;
    sequence.current++;
    setBusy(true);
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
    <section className="pairing-panel" aria-labelledby="pairing-title">
      <h3 id="pairing-title">设备配对与权限</h3>
      <p className="helper">
        仅用于文档摘录协作，每个动作仍需单独确认。电脑选定并分享预览后，可在手机确认保存。配对不授予任意文件、聊天或记忆访问。
      </p>
      <p className="helper">
        配对绑定本次登录；退出或会话过期后需重新配对。设备名称由用户填写，请核对两端信息。
      </p>
      {note && <p role="status">{note}</p>}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void run(async () => {
            setOffer(null);
            const value = decodePairingOffer(
              await invoke("account_pairing_offer", { name }),
            );
            if (alive.current) {
              setOffer(value);
              await refresh();
            }
          });
        }}
      >
        <label htmlFor="pair-device-name">这台电脑的名称</label>
        <input
          id="pair-device-name"
          required
          maxLength={40}
          value={name}
          disabled={busy}
          onChange={(e) => setName(e.target.value)}
        />
        <div className="account-actions">
          <button disabled={busy}>生成五分钟配对码</button>
          <button type="button" disabled={busy} onClick={() => void refresh()}>
            刷新配对
          </button>
        </div>
      </form>
      {offer && (
        <div className="account-notice">
          <p>在手机的「连接电脑」输入这六位数字，再核对设备并确认。</p>
          <code className="pairing-code" data-testid="pairing-code">
            {offer.code}
          </code>
          <p>
            有效至 {new Date(offer.pairing.expiresAt).toLocaleTimeString()}
            。重新生成会使旧码失效。
          </p>
        </div>
      )}
      <ul className="account-tasks">
        {pairs
          .filter((p) => ["pending", "active"].includes(p.status))
          .map((p) => (
            <li key={p.id}>
              <div>
                <strong>
                  {p.desktopName} → {p.controllerName || "等待手机"}
                </strong>
                <small>{pairingStatus[p.status]} · 文档摘录</small>
                <small>电脑标识：{p.desktopId}</small>
              </div>
              <button
                disabled={busy}
                aria-label={`撤销配对 ${p.desktopName}`}
                onClick={() => {
                  if (
                    window.confirm(
                      "撤销配对会阻止新动作；已准入的动作需核对停止结果，不能保证即时回滚。确认撤销？",
                    )
                  )
                    void run(async () => {
                      setOffer(null);
                      await invoke("account_pairing_revoke", {
                        id: p.id,
                        revision: p.revision,
                      });
                      await refresh();
                    });
                }}
              >
                撤销配对
              </button>
            </li>
          ))}
      </ul>
      {pairs.some((p) => !["pending", "active"].includes(p.status)) && (
        <details className="finished-pairings">
          <summary>
            已结束的配对（
            {pairs.filter((p) => !["pending", "active"].includes(p.status)).length}
            ）
          </summary>
          <p className="helper">
            撤销或失效的关系不再可用，仅作留痕；重新生成配对码会新增记录，不会恢复旧关系。
          </p>
          <ul className="account-tasks">
            {pairs
              .filter((p) => !["pending", "active"].includes(p.status))
              .map((p) => (
                <li key={p.id}>
                  <div>
                    <strong>
                      {p.desktopName} → {p.controllerName || "等待手机"}
                    </strong>
                    <small>{pairingStatus[p.status]} · 文档摘录</small>
                    <small>电脑标识：{p.desktopId}</small>
                  </div>
                </li>
              ))}
          </ul>
        </details>
      )}
    </section>
  );
}
