import { useEffect, useRef, useState } from "react";
import {
  decodeDocuments,
  documentStates,
  type DocumentAction,
} from "@companion/contracts";
import { api, ApiError, message } from "./api";
export function DocumentPanel({ onExpired }: { onExpired: () => void }) {
  const [docs, setDocs] = useState<DocumentAction[]>([]),
    [note, setNote] = useState(""),
    [busy, setBusy] = useState(false);
  const active = useRef(false),
    acting = useRef(false),
    generation = useRef(0),
    expired = useRef(onExpired);
  expired.current = onExpired;
  function failure(e: unknown) {
    if (e instanceof ApiError && e.status === 401) expired.current();
    setNote(message(e));
  }
  async function refresh() {
    const id = ++generation.current;
    try {
      const v = decodeDocuments(await api("/documents"));
      if (active.current && id === generation.current) setDocs(v);
    } catch (e) {
      if (active.current && id === generation.current) failure(e);
    }
  }
  useEffect(() => {
    active.current = true;
    void refresh();
    const timer = setInterval(() => {
      if (!acting.current && document.visibilityState === "visible")
        void refresh();
    }, 5000);
    return () => {
      active.current = false;
      generation.current++;
      clearInterval(timer);
    };
  }, []);
  async function run(d: DocumentAction, operation: "confirm" | "cancel") {
    if (acting.current) return;
    acting.current = true;
    generation.current++;
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
        failure(e);
        await refresh();
      }
    } finally {
      acting.current = false;
      if (active.current) setBusy(false);
    }
  }
  return (
    <section className="card remote-documents">
      <h2>文档确认与结果</h2>
      <p className="hint">
        由电脑选定文档并分享摘录。确认后电脑保存此份草稿；从分享起最多五分钟内可执行，电脑需保持运行和联网。
      </p>
      <button disabled={busy} onClick={() => void refresh()}>
        刷新文档状态
      </button>
      {note && <p role="status">{note}</p>}
      {!docs.length && <p className="hint">暂时没有分享给此设备的文档。</p>}
      {docs.map((d) => (
        <article key={d.authorization.binding.actionId}>
          <h3>{d.sourceName}</h3>
          <p data-testid="remote-document-status">
            {documentStates[d.authorization.state]}
          </p>
          <details>
            <summary>查看具体摘录与保存信息</summary>
            <pre>{d.preview}</pre>
            <p>
              保存至电脑应用数据的 remote-documents 下，文件名{" "}
              {d.authorization.binding.actionId}.md；不覆盖原文。
            </p>
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
          {!["completed", "failed", "cancelled"].includes(
            d.authorization.state,
          ) && (
            <button disabled={busy} onClick={() => void run(d, "cancel")}>
              取消或请求停止
            </button>
          )}
          {d.authorization.state === "completed" && (
            <p className="hint">
              电脑已报告草稿内容与摘要一致。上方摘录即本次保存内容；未提供手机下载电脑文件入口。
            </p>
          )}
        </article>
      ))}
    </section>
  );
}
