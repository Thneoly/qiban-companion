import { describe, expect, it } from "vitest";
import {
  decodeRemoteMemoryItem,
  decodeRemoteMemories,
} from "./remote-memories";

const active = {
  id: "00000000-0000-4000-8000-000000000001",
  kind: "preference",
  body: "prefer tea",
  eventDate: "2026-01-02",
  revision: 1,
  deleted: false,
};
const tombstone = {
  id: "00000000-0000-4000-8000-000000000002",
  kind: "experience",
  body: null,
  eventDate: null,
  revision: 3,
  deleted: true,
};

describe("decodeRemoteMemoryItem", () => {
  it("accepts the active and tombstone wire shapes", () => {
    expect(decodeRemoteMemoryItem(active)).toEqual(active);
    expect(decodeRemoteMemoryItem(tombstone)).toEqual(tombstone);
    expect(decodeRemoteMemoryItem({ ...active, eventDate: null })).toEqual({
      ...active,
      eventDate: null,
    });
  });
  it("rejects malformed ids, kinds, bodies, dates and revisions", () => {
    for (const bad of [
      { ...active, id: "not-a-uuid" },
      { ...active, kind: "mood" },
      { ...active, revision: 0 },
      { ...active, revision: 1.5 },
      { ...active, deleted: "false" },
      { ...active, body: "" },
      { ...active, body: "x".repeat(201) },
      { ...active, eventDate: "2026-1-2" },
      // Tombstones must be content-free.
      { ...tombstone, body: "leftover" },
      { ...tombstone, eventDate: "2026-01-02" },
      // Active entries must carry a body.
      { ...active, body: null },
      null,
      42,
    ]) {
      expect(() => decodeRemoteMemoryItem(bad)).toThrow();
    }
    // Calendar semantics stay server-side: a well-shaped impossible date is
    // shape-valid here (the coordinator rejects it at write time).
    expect(
      decodeRemoteMemoryItem({ ...active, eventDate: "2026-13-01" }).eventDate,
    ).toBe("2026-13-01");
  });
});

describe("decodeRemoteMemories", () => {
  it("decodes lists and rejects non-arrays", () => {
    expect(decodeRemoteMemories([])).toEqual([]);
    expect(decodeRemoteMemories([active, tombstone])).toEqual([
      active,
      tombstone,
    ]);
    expect(() => decodeRemoteMemories({})).toThrow();
    expect(() => decodeRemoteMemories([active, { junk: true }])).toThrow();
  });
});
