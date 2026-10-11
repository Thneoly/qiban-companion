/** Mirrors the coordinator's MemorySnapshotItem (camelCase wire shape). The
 * phone list only carries active entries; tombstones appear in the desktop
 * sync snapshot, never on this peer. */
export interface RemoteMemoryItem {
  id: string;
  kind: "preference" | "experience" | "task_fact";
  body: string | null;
  eventDate: string | null;
  revision: number;
  deleted: boolean;
}

const KINDS = ["preference", "experience", "task_fact"] as const;
const UUID =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const EVENT_DATE = /^\d{4}-\d{2}-\d{2}$/;

export function decodeRemoteMemoryItem(value: unknown): RemoteMemoryItem {
  const v = value as RemoteMemoryItem;
  if (
    typeof v?.id !== "string" ||
    !UUID.test(v.id) ||
    !(KINDS as readonly string[]).includes(v.kind) ||
    !Number.isSafeInteger(v.revision) ||
    v.revision < 1 ||
    typeof v.deleted !== "boolean" ||
    (v.deleted
      ? v.body !== null || v.eventDate !== null
      : typeof v.body !== "string" ||
        v.body.length < 1 ||
        v.body.length > 200 ||
        (v.eventDate !== null &&
          (typeof v.eventDate !== "string" || !EVENT_DATE.test(v.eventDate))))
  ) {
    throw new Error("invalid memory item");
  }
  return v;
}

export function decodeRemoteMemories(value: unknown): RemoteMemoryItem[] {
  if (!Array.isArray(value)) throw new Error("invalid memory list");
  return value.map(decodeRemoteMemoryItem);
}
