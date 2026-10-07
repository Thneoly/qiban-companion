import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  decodeDocuments,
  decodeDocument,
  decodePairings,
  documentStates,
  type DocumentAction,
  type Pairing,
  type ExecutionTask,
  decodeExecutionTask,
} from "@companion/contracts";
import { errorMessage } from "../../lib/client";
export function RemoteDocuments({
  onAuthError,
}: {
  onAuthError: (e: unknown) => void;
}) {
  const [pairs, setPairs] = useState<Pairing[]>([]),
    [pair, setPair] = useState(""),
    [docs, setDocs] = useState<DocumentAction[]>([]),
    [prepared, setPrepared] = useState<ExecutionTask | null>(null);
  const [file, setFile] = useState<File | null>(null),
    [note, setNote] = useState(""),
    [busy, setBusy] = useState(false),
    // actionId of the task this panel just shared; its hint is true only
    // while that task is still awaiting the phone's confirmation.
    [shared, setShared] = useState<string | null>(null);
  const request = useRef(crypto.randomUUID()),
    alive = useRef(false),
    acting = useRef(false),
    seq = useRef(0),
    auth = useRef(onAuthError);
  auth.current = onAuthError;
  function fail(e: unknown) {
    if (
      e &&
      typeof e === "object" &&
      "code" in e &&
      ["authentication_required", "logout_pending", "credentials"].includes(
        String(e.code),
      )
    )
      auth.current(e);
    setNote(errorMessage(e));
  }
  async function refresh() {
    const id = ++seq.current;
    try {
      const rows = decodeDocuments(await invoke("remote_document_sync"));
      const links = decodePairings(await invoke("account_pairings"));
      if (alive.current && id === seq.current) {
        setDocs(rows);
        setPairs(
          links.filter(
            (p) => p.status === "active" && p.currentRole === "desktop",
          ),
        );
        // The "awaiting phone" hint retires with the fact it describes: once
        // a fresh list shows the task advanced (or gone), the hint is stale.
        setShared((current) => {
          const row = rows.find(
            (d) => d.authorization.binding.actionId === current,
          );
          return row?.authorization.state === "awaiting_confirmation"
            ? current
            : null;
        });
      }
    } catch (e) {
      if (alive.current && id === seq.current) fail(e);
    }
  }
  useEffect(() => {
    alive.current = true;
    void refresh();
    const timer = setInterval(() => {
      if (!acting.current && document.visibilityState === "visible")
        void refresh();
    }, 5000);
    return () => {
      alive.current = false;
      seq.current++;
      clearInterval(timer);
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
      if (alive.current) fail(e);
    } finally {
      acting.current = false;
      if (alive.current) setBusy(false);
    }
  }
  return (
    <section className="remote-documents">
      <h3>让手机确认一份文档</h3>
      <p className="helper">
        先在电脑生成本地预览。分享时仅发送文件名和摘录到账号服务；手机确认后，在本机
        remote-documents 下保存新草稿，不覆盖原文件。
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void run(async () => {
            if (!file || !pair || file.size > 256 * 1024)
              throw Error("请选择已配对手机和最多 256 KB 的 UTF-8 文档");
            const text = new TextDecoder("utf-8", { fatal: true }).decode(
              await file.arrayBuffer(),
            );
            const result = await invoke<{ task: unknown }>(
              "remote_document_prepare",
              {
                pairId: pair,
                requestId: request.current,
                sourceName: file.name,
                text,
              },
            );
            if (alive.current) setPrepared(decodeExecutionTask(result.task));
          });
        }}
      >
        <label htmlFor="remote-peer">接收确认的手机</label>
        <select
          id="remote-peer"
          required
          disabled={busy}
          value={pair}
          onChange={(e) => {
            setPair(e.target.value);
            setPrepared(null);
            setShared(null);
            request.current = crypto.randomUUID();
          }}
        >
          <option value="">请选择配对关系</option>
          {pairs.map((p) => (
            <option key={p.id} value={p.id}>
              {p.controllerName}（{p.id.slice(0, 8)}）
            </option>
          ))}
        </select>
        <label htmlFor="remote-source">选择要分享摘录的文档</label>
        <input
          id="remote-source"
          type="file"
          accept=".txt,.md"
          disabled={busy}
          onChange={(e) => {
            setFile(e.target.files?.[0] || null);
            setPrepared(null);
            setShared(null);
            request.current = crypto.randomUUID();
          }}
        />
        <button disabled={busy || !file || !pair}>生成本地预览</button>
      </form>
      {prepared && (
        <div>
          <pre aria-label="待分享摘录预览">{prepared.preview}</pre>
          <p>
            保存文件：{prepared.artifactName}
            。只分享上面的摘录，不发送完整原文。
          </p>
          <button
            disabled={busy}
            onClick={() =>
              void run(async () => {
                const doc = decodeDocument(
                  await invoke("remote_document_share", { id: prepared.id }),
                );
                if (alive.current) {
                  setPrepared(null);
                  setShared(doc.authorization.binding.actionId);
                  await refresh();
                }
              })
            }
          >
            分享预览并等待手机确认
          </button>
        </div>
      )}
      <button disabled={busy} onClick={() => void refresh()}>
        刷新文档协作
      </button>
      {note && <p role="status">{note}</p>}
      {shared && (
        <p role="status">
          预览已分享，等待手机确认。关闭面板不停止已经授权的保存。
        </p>
      )}
      {docs.map((d) => (
        <article key={d.authorization.binding.actionId}>
          <strong>{d.sourceName}</strong>
          <p>{documentStates[d.authorization.state]}</p>
          <details>
            <summary>摘录与文件名</summary>
            <pre>{d.preview}</pre>
            <code>{d.authorization.binding.actionId}.md</code>
          </details>
        </article>
      ))}
    </section>
  );
}
