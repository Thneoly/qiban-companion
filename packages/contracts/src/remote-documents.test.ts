import { describe, expect, it } from "vitest";
import { decodeDocument, decodeDocuments } from "./remote-documents";
const uuid = "ea80aff6-a661-49d2-8175-74ec3c67dc21";
const example = () => ({
  authorization: {
    pairingId: uuid,
    binding: {
      actionId: uuid,
      resourceId: uuid,
      resourceVersion: 1,
      parametersDigest: "a".repeat(64),
      pairRevision: 2,
      scope: "document_excerpt",
    },
    expiresAt: 1000,
    state: "awaiting_confirmation",
  },
  sourceName: "测试.txt",
  preview: "exact preview",
  artifactHash: "b".repeat(64),
  currentRole: "controller",
});
describe("remote document boundary", () => {
  it("projects only display and exact confirmation fields", () => {
    const input = { ...example(), token: "must not propagate" };
    expect(decodeDocument(input)).toEqual(example());
    expect(decodeDocuments([input])).toHaveLength(1);
  });
  it("rejects changed scope, unsupported versions and unknown outcomes", () => {
    for (const patch of [
      { scope: "shell" },
      { resourceVersion: 2 },
      { pairRevision: 0 },
      { parametersDigest: "invalid" },
    ]) {
      const input = example();
      Object.assign(input.authorization.binding, patch);
      expect(() => decodeDocument(input)).toThrow();
    }
    const input = example();
    input.authorization.state = "success";
    expect(() => decodeDocument(input)).toThrow();
    expect(() => decodeDocument(null)).toThrow();
    expect(() => decodeDocuments(Array(21).fill(example()))).toThrow();
  });
});
