import { useEffect, useState } from "react";

/**
 * Product announcements from the repository's announcements.json —
 * publishing is a commit, not a release. Raw on main is authoritative and
 * instant but often unreachable from CN networks; jsDelivr mirrors the same
 * file with CDN reachability at the cost of cache lag, so it is the
 * fallback. Read state lives in localStorage; a failed fetch is silent
 * (offline looks like no news). The file is authored content from the
 * project, rendered as TEXT only — no HTML from the feed is ever injected.
 */
const FEED_URLS = [
  "https://raw.githubusercontent.com/Thneoly/qiban-companion/main/announcements.json",
  "https://cdn.jsdelivr.net/gh/Thneoly/qiban-companion@main/announcements.json",
];
const READ_KEY = "qiban-read-announcements";

type Item = { id: string; date: string; title: string; body: string };
const isItem = (value: unknown): value is Item => {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.id === "string" &&
    v.id.length > 0 &&
    v.id.length <= 80 &&
    typeof v.date === "string" &&
    v.date.length <= 10 &&
    typeof v.title === "string" &&
    v.title.length > 0 &&
    v.title.length <= 80 &&
    typeof v.body === "string" &&
    v.body.length <= 1000
  );
};

function readSet(): Set<string> {
  try {
    const raw = localStorage.getItem(READ_KEY);
    const parsed = raw ? (JSON.parse(raw) as unknown) : [];
    return new Set(Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

export function Announcements() {
  const [items, setItems] = useState<Item[]>([]);
  const [read, setRead] = useState<Set<string>>(readSet);
  const [open, setOpen] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let disposed = false;
    // Wait out the startup burst, same as the update check.
    const timer = setTimeout(() => {
      const attempt = (index: number) => {
        const url = FEED_URLS[index];
        if (url === undefined) {
          if (!disposed) setFailed(true);
          return;
        }
        fetch(url, { cache: "no-store" })
          .then((response) => (response.ok ? response.json() : Promise.reject(new Error("status"))))
          .then((value: unknown) => {
            if (disposed) return;
            const list = Array.isArray((value as { items?: unknown })?.items)
              ? ((value as { items: unknown[] }).items.filter(isItem) as Item[])
              : [];
            setItems(list.slice(0, 20));
          })
          .catch(() => attempt(index + 1));
      };
      attempt(0);
    }, 6000);
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, []);

  const unread = items.filter((item) => !read.has(item.id));
  if (!items.length) return null;

  function markAllRead() {
    const next = new Set(items.map((item) => item.id));
    setRead(next);
    try {
      localStorage.setItem(READ_KEY, JSON.stringify([...next]));
    } catch {
      /* read-state is cosmetic; losing it only re-shows the badge */
    }
    setOpen(false);
  }

  return (
    <section className="model-settings" aria-label="公告">
      <div className="section-heading">
        <h2>公告{unread.length > 0 && <span className="count">{unread.length} 条新</span>}</h2>
        <button type="button" onClick={() => (open ? markAllRead() : setOpen(true))}>
          {open ? "全部标为已读" : "查看"}
        </button>
      </div>
      {open ? (
        <ul className="account-tasks">
          {items.map((item) => (
            <li key={item.id}>
              <div>
                <strong>{item.title}</strong>
                <small>{item.date}</small>
                <p className="workspace-hint">{item.body}</p>
              </div>
            </li>
          ))}
        </ul>
      ) : (
        unread.length > 0 &&
          unread[0] && (
            <p className="helper">
              最新：{unread[0].title}（{unread[0].date}）
              {failed ? "；公告加载可能不完整，网络恢复后下次启动重试" : ""}
            </p>
          )
      )}
    </section>
  );
}
