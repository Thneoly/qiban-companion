import { useEffect, useRef, useState } from "react";
import {
  decodeRemoteMemories,
  type RemoteMemoryItem,
} from "@companion/contracts";
import { api, ApiError, message } from "./api";

const KIND_LABELS: Record<RemoteMemoryItem["kind"], string> = {
  preference: "偏好",
  experience: "经验",
  task_fact: "任务事实",
};

/** View + correct + delete the account's mirrored memories. Creation stays
 * desktop-only (the local domain is the only creation entry); every change
 * here syncs to all devices — confirmations say so, and they never offer a
 * "new conversation" switch because the phone cannot see the desktop chat. */
export function MemoryPanel({ onExpired }: { onExpired: () => void }) {
  const [items, setItems] = useState<RemoteMemoryItem[]>([]),
    [note, setNote] = useState(""),
    [busy, setBusy] = useState(false),
    [unreachable, setUnreachable] = useState(false),
    // The entry whose correction dialog is open; the draft is captured when
    // the dialog opens so a background refresh cannot swap it mid-edit.
    [editing, setEditing] = useState<RemoteMemoryItem | null>(null),
    [draft, setDraft] = useState(""),
    [draftDate, setDraftDate] = useState(""),
    [confirming, setConfirming] = useState<RemoteMemoryItem | null>(null);
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
      const rows = decodeRemoteMemories(await api("/memories"));
      if (active.current && id === generation.current) {
        setUnreachable(false);
        setItems(rows);
        // A revision moved under an open dialog: close it rather than commit
        // against stale state — the same lockstep the desktop panels keep.
        setEditing((open) => {
          if (
            open &&
            rows.every(
              (r) => !(r.id === open.id && r.revision === open.revision),
            )
          ) {
            setNote("这条记忆已在其他设备更新，已关闭编辑并刷新。");
            return null;
          }
          return open;
        });
      }
    } catch (e) {
      if (active.current && id === generation.current) {
        if (e instanceof ApiError && (e.status === 0 || e.status === 503)) {
          setUnreachable(true);
        } else {
          setUnreachable(false);
          failure(e);
        }
      }
    }
  }

  useEffect(() => {
    active.current = true;
    void refresh();
    const tick = () => {
      if (!acting.current && document.visibilityState === "visible")
        void refresh();
    };
    const timer = setInterval(tick, 10000);
    const resume = () => {
      if (document.visibilityState === "visible") tick();
    };
    window.addEventListener("online", resume);
    window.addEventListener("focus", resume);
    document.addEventListener("visibilitychange", resume);
    return () => {
      active.current = false;
      generation.current++;
      clearInterval(timer);
      window.removeEventListener("online", resume);
      window.removeEventListener("focus", resume);
      document.removeEventListener("visibilitychange", resume);
    };
  }, []);

  function openEdit(item: RemoteMemoryItem) {
    setEditing(item);
    setDraft(item.body ?? "");
    setDraftDate(item.eventDate ?? "");
    setNote("");
  }

  async function submitEdit() {
    if (!editing || acting.current) return;
    const body = draft.trim();
    if (!body || [...body].length > 200) {
      setNote("记忆内容需要 1～200 个字符。");
      return;
    }
    acting.current = true;
    setBusy(true);
    setNote("");
    try {
      await api(`/memories/${editing.id}/update`, {
        revision: editing.revision,
        requestId: crypto.randomUUID(),
        kind: editing.kind,
        body,
        eventDate: /^\d{4}-\d{2}-\d{2}$/.test(draftDate) ? draftDate : null,
      });
      setEditing(null);
      setNote("已更正，将同步到所有设备。");
      await refresh();
    } catch (e) {
      failure(e);
      // A conflict means another device moved first; the refreshed list is
      // the honest view to retry against.
      setEditing(null);
      await refresh();
    } finally {
      acting.current = false;
      if (active.current) setBusy(false);
    }
  }

  async function submitDelete() {
    if (!confirming || acting.current) return;
    const target = confirming;
    acting.current = true;
    setBusy(true);
    setNote("");
    try {
      await api(`/memories/${target.id}/delete`, {
        revision: target.revision,
        requestId: crypto.randomUUID(),
      });
      setConfirming(null);
      setNote("已删除，将从所有设备移除。");
      await refresh();
    } catch (e) {
      failure(e);
      setConfirming(null);
      await refresh();
    } finally {
      acting.current = false;
      if (active.current) setBusy(false);
    }
  }

  return (
    <section className="card remote-memories">
      <h2>应用记忆</h2>
      <p className="hint">
        与电脑应用同步的记忆（最多 30 条）。在手机上更正或删除会同步到所有设备；电脑上引用它的对话会被清理，此操作不可撤销。
      </p>
      <button disabled={busy} onClick={() => void refresh()}>
        刷新记忆
      </button>
      {unreachable && (
        <p role="status" className="notice">
          无法连接服务，记忆状态未知；显示的是最后一次成功读取的结果。
        </p>
      )}
      {note && <p role="status">{note}</p>}
      {!items.length && !unreachable && (
        <p className="hint">此账号还没有同步的应用记忆。</p>
      )}
      {items.map((item) => (
        <article key={item.id}>
          <h4>
            {KIND_LABELS[item.kind]}
            {item.eventDate && <small> · {item.eventDate}</small>}
          </h4>
          {editing && editing.id === item.id ? (
            <div className="edit">
              <textarea
                value={draft}
                maxLength={200}
                onChange={(e) => setDraft(e.target.value)}
                aria-label="更正记忆内容"
              />
              <input
                type="date"
                value={draftDate}
                onChange={(e) => setDraftDate(e.target.value)}
                aria-label="关联日期（可选）"
              />
              <div>
                <button disabled={busy} onClick={() => void submitEdit()}>
                  确认更正
                </button>
                <button disabled={busy} onClick={() => setEditing(null)}>
                  返回，不修改
                </button>
              </div>
            </div>
          ) : (
            <>
              <p data-testid="remote-memory-body">{item.body}</p>
              {confirming && confirming.id === item.id ? (
                <div>
                  <p>
                    删除后这条记忆将从所有设备移除，电脑上引用它的对话会被清理；确认删除？
                  </p>
                  <div>
                    <button disabled={busy} onClick={() => void submitDelete()}>
                      确认删除
                    </button>
                    <button disabled={busy} onClick={() => setConfirming(null)}>
                      返回，不删除
                    </button>
                  </div>
                </div>
              ) : (
                <div>
                  <button disabled={busy} onClick={() => openEdit(item)}>
                    更正内容
                  </button>
                  <button
                    disabled={busy}
                    onClick={() => {
                      setConfirming(item);
                      setEditing(null);
                    }}
                  >
                    删除
                  </button>
                </div>
              )}
            </>
          )}
        </article>
      ))}
    </section>
  );
}
