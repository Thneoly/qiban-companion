import { useEffect, useRef, useState } from "react";
import {
  decodeDocuments,
  decodePairings,
  deriveDocumentPhase,
  isDeletableDocumentState,
  type DocumentAction,
  type Pairing,
} from "@companion/contracts";
import { api, ApiError, message } from "./api";
export function DocumentPanel({ onExpired }: { onExpired: () => void }) {
  const [docs, setDocs] = useState<DocumentAction[]>([]),
    [pairings, setPairings] = useState<Pairing[]>([]),
    [presenceOk, setPresenceOk] = useState(false),
    [note, setNote] = useState(""),
    [busy, setBusy] = useState(false),
    [unreachable, setUnreachable] = useState(false),
    [clockNow, setClockNow] = useState(() => Date.now());
  const active = useRef(false),
    acting = useRef(false),
    docGeneration = useRef(0),
    pairGeneration = useRef(0),
    expired = useRef(onExpired);
  expired.current = onExpired;
  function failure(e: unknown) {
    if (e instanceof ApiError && e.status === 401) expired.current();
    // A 409 here means the task moved on (often the window expired between
    // render and click) — the coordinator reports it as pairing_conflict.
    if (
      e instanceof ApiError &&
      ["conflict", "pairing_conflict"].includes(e.code)
    ) {
      setNote("该任务状态已变化（可能已到有效期），已重新获取列表。");
      return;
    }
    setNote(message(e));
  }
  // The presence lease decorates labels only while the last pairings read
  // succeeded — a failed read degrades to "unknown", never a stale claim.
  async function refreshPresence() {
    const id = ++pairGeneration.current;
    try {
      const rows = decodePairings(await api("/pairings"));
      if (active.current && id === pairGeneration.current) {
        setPairings(rows);
        setPresenceOk(true);
      }
    } catch (e) {
      if (active.current && id === pairGeneration.current) {
        if (e instanceof ApiError && e.status === 401) expired.current();
        setPresenceOk(false);
      }
    }
  }
  async function refresh() {
    const id = ++docGeneration.current;
    try {
      const v = decodeDocuments(await api("/documents"));
      if (active.current && id === docGeneration.current) {
        setUnreachable(false);
        setDocs(v);
      }
    } catch (e) {
      if (active.current && id === docGeneration.current) {
        // 服务不可达 ≠ 电脑离线：保留最后一轮成功数据并明确提示状态未知。
        if (e instanceof ApiError && (e.status === 0 || e.status === 503)) {
          setUnreachable(true);
        } else {
          setUnreachable(false);
          failure(e);
        }
      }
    }
    void refreshPresence();
  }
  useEffect(() => {
    active.current = true;
    void refresh();
    const documentsTick = () => {
      if (!acting.current && document.visibilityState === "visible")
        void refresh();
    };
    const presenceTick = () => {
      if (!acting.current && document.visibilityState === "visible")
        void refreshPresence();
    };
    const documentTimer = setInterval(documentsTick, 5000);
    // Presence effectively refreshes with the 5s documents cycle (refresh()
    // pulls both); this 10s timer is only a floor, still inside the
    // documented worst case (lease 15s + poll 10s).
    const presenceTimer = setInterval(presenceTick, 10000);
    const resume = () => {
      if (document.visibilityState === "visible") {
        documentsTick();
        presenceTick();
      }
    };
    window.addEventListener("online", resume);
    window.addEventListener("focus", resume);
    document.addEventListener("visibilitychange", resume);
    return () => {
      active.current = false;
      docGeneration.current++;
      pairGeneration.current++;
      clearInterval(documentTimer);
      clearInterval(presenceTimer);
      window.removeEventListener("online", resume);
      window.removeEventListener("focus", resume);
      document.removeEventListener("visibilitychange", resume);
    };
  }, []);
  // Live clock while any phase can still move: countdowns for
  // awaiting/confirmed AND the last-contact stamps on offline hints of
  // admitted/cancel_requested/unknown (otherwise the stamp freezes while
  // PairingPanel keeps counting for the same pairing).
  useEffect(() => {
    if (
      !docs.some(
        (d) => !isDeletableDocumentState(d.authorization.state),
      )
    )
      return;
    const timer = setInterval(() => setClockNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [docs]);
  function phase(d: DocumentAction) {
    return deriveDocumentPhase(
      d,
      presenceOk
        ? (pairings.find((p) => p.id === d.authorization.pairingId) ?? null)
        : null,
      clockNow,
    );
  }
  async function run(
    d: DocumentAction,
    operation: "confirm" | "cancel" | "delete",
  ) {
    if (acting.current) return;
    acting.current = true;
    docGeneration.current++;
    setBusy(true);
    setNote("");
    try {
      await api(
        `/documents/${d.authorization.binding.actionId}/${operation}`,
        operation === "confirm" ? d.authorization.binding : {},
      );
      if (active.current) await refresh();
    } catch (e) {
      if (active.current) {
        // A retried delete after a lost response: the record is already
        // gone and the server has no 404 semantics for documents.
        if (
          operation === "delete" &&
          e instanceof ApiError &&
          e.code === "pairing_denied"
        ) {
          setNote("该记录已不存在（可能已在其他设备删除），已刷新列表。");
        } else {
          failure(e);
        }
        await refresh();
      }
    } finally {
      acting.current = false;
      if (active.current) setBusy(false);
    }
  }
  // Save a local copy of a completed result — verify the preview against the
  // reported artifact hash on the phone first, so the copy is phone-verified,
  // not merely server-verified.
  async function saveCopy(d: DocumentAction) {
    if (acting.current) return;
    acting.current = true;
    docGeneration.current++;
    setBusy(true);
    setNote("");
    try {
      const digest = Array.from(
        new Uint8Array(
          await crypto.subtle.digest(
            "SHA-256",
            new TextEncoder().encode(d.preview),
          ),
        ),
        (b) => b.toString(16).padStart(2, "0"),
      ).join("");
      if (digest !== d.artifactHash) {
        setNote("副本内容与回报哈希不一致，已取消保存，请刷新后再试。");
        return;
      }
      const url = URL.createObjectURL(
        new Blob([d.preview], { type: "text/markdown;charset=utf-8" }),
      );
      const link = document.createElement("a");
      link.href = url;
      link.download = `${d.sourceName.replace(/\.[^.]+$/, "")}-${d.authorization.binding.actionId.slice(0, 8)}.md`;
      link.click();
      URL.revokeObjectURL(url);
      setNote("副本已按回报哈希核验一致，保存到本机。");
    } finally {
      acting.current = false;
      if (active.current) setBusy(false);
    }
  }
  function renderDoc(d: DocumentAction) {
    const p = phase(d);
    return (
      <article key={d.authorization.binding.actionId}>
        <h4>{d.sourceName}</h4>
        <p data-testid="remote-document-status">{p.label}</p>
        <p className="hint" data-testid="remote-document-phase-hint">
          {p.hint}
        </p>
        <details>
          <summary>
            {d.authorization.state === "completed"
              ? "核对保存结果"
              : "查看具体摘录与保存信息"}
          </summary>
          <pre>{d.preview}</pre>
          <p>
            保存至电脑应用数据的 remote-documents/账号目录/document-drafts/
            下，文件名 {d.authorization.binding.actionId}.md；不覆盖原文。
          </p>
          {d.authorization.state === "completed" && (
            <p>
              产物哈希 {d.artifactHash.slice(0, 12)}…。电脑回报完成时，服务端已核对该哈希与这份预览的摘要一致；上方预览即本次保存的内容。
            </p>
          )}
        </details>
        {d.currentRole === "controller" &&
          d.authorization.state === "awaiting_confirmation" && (
            <button
              disabled={busy}
              onClick={() => {
                if (
                  window.confirm(
                    `确认让电脑将 ${d.sourceName} 的这份摘录保存为新草稿？`,
                  )
                )
                  void run(d, "confirm");
              }}
            >
              确认电脑保存这份摘录
            </button>
          )}
        {["awaiting_confirmation", "confirmed", "admitted"].includes(
          d.authorization.state,
        ) && (
          <button disabled={busy} onClick={() => void run(d, "cancel")}>
            取消或请求停止
          </button>
        )}
        {["waiting_desktop", "executing", "cancel_requested", "unknown"]
          .includes(p.key) && (
          <button
            disabled={busy}
            aria-label={`刷新此任务 ${d.sourceName}`}
            onClick={() => void refresh()}
          >
            刷新此任务
          </button>
        )}
        {d.authorization.state === "completed" && (
          <button disabled={busy} onClick={() => void saveCopy(d)}>
            保存副本到手机
          </button>
        )}
        {isDeletableDocumentState(d.authorization.state) && (
          <button
            disabled={busy}
            onClick={() => {
              if (
                window.confirm(
                  "删除后此记录从手机与服务端消失且无法撤销，电脑不再执行或补报，已保存的草稿文件与已开始的写入不受影响；确认删除？",
                )
              )
                void run(d, "delete");
            }}
          >
            删除记录
          </button>
        )}
      </article>
    );
  }
  const running = docs.filter(
    (d) => !isDeletableDocumentState(d.authorization.state),
  );
  const finished = docs.filter((d) =>
    isDeletableDocumentState(d.authorization.state),
  );
  return (
    <section className="card remote-documents">
      <h2>文档确认与结果</h2>
      <p className="hint">
        由电脑选定文档并分享摘录。确认后电脑保存此份草稿；从分享起最多五分钟内可执行，电脑需保持运行和联网。
      </p>
      <button disabled={busy} onClick={() => void refresh()}>
        刷新文档状态
      </button>
      {unreachable && (
        <p role="status" className="notice">
          无法连接服务，任务状态未知；显示的是最后一次成功读取的结果。
        </p>
      )}
      {note && <p role="status">{note}</p>}
      {!docs.length && <p className="hint">暂时没有分享给此设备的文档。</p>}
      {!!running.length && (
        <>
          <h3>进行中的任务</h3>
          {running.map(renderDoc)}
        </>
      )}
      {!!finished.length && (
        <>
          <h3>已结束的任务</h3>
          <p className="hint">
            已结束的任务不再变化。可把已保存并核验的结果另存副本到本机：副本内容与电脑文件一致（手机已按回报哈希核验），保存在手机本地，与电脑文件互不影响。删除只清除手机与服务端的任务记录，不删除电脑上的文件，也无法撤销；结果未知的任务需电脑核对结束后才能清理。
          </p>
          {finished.map(renderDoc)}
        </>
      )}
    </section>
  );
}
