import { it, expect } from "vitest";
import { decodePairing, decodePairingOffer } from "./pairing";
it("rejects privilege escalation and projects only public pairing metadata", () => {
  const p = {
    id: crypto.randomUUID(),
    desktopId: crypto.randomUUID(),
    desktopName: "电脑",
    controllerId: null,
    controllerName: null,
    scope: "document_excerpt",
    status: "pending",
    revision: 1,
    expiresAt: 1000,
    currentRole: "desktop",
    desktopOnline: true,
    desktopLastHeartbeatAt: 950,
    desktopCapabilities: ["document_excerpt"],
  };
  expect(decodePairing({ ...p, codeHash: "secret", token: "secret" })).toEqual(
    p,
  );
  // Missing presence fields (an older coordinator) degrade to unknown, never
  // a decode failure: null/null/[] so the UI can show 状态未知 instead.
  const { desktopOnline: _o, desktopLastHeartbeatAt: _t, desktopCapabilities: _c, ...legacy } = p;
  expect(decodePairing(legacy)).toEqual({
    ...legacy,
    desktopOnline: null,
    desktopLastHeartbeatAt: null,
    desktopCapabilities: [],
  });
  // Present-but-invalid values still throw.
  expect(() => decodePairing({ ...p, desktopOnline: "yes" })).toThrow();
  expect(() => decodePairing({ ...p, desktopLastHeartbeatAt: -1 })).toThrow();
  expect(() => decodePairing({ ...p, desktopCapabilities: Array(9).fill("document_excerpt") })).toThrow();
  expect(() => decodePairing({ ...p, desktopCapabilities: ["Not A Slug"] })).toThrow();
  // Open set: a future unknown slug decodes and surfaces raw.
  expect(decodePairing({ ...p, desktopCapabilities: ["future_scope"] }).desktopCapabilities).toEqual([
    "future_scope",
  ]);
  expect(() => decodePairing({ ...p, scope: "shell" })).toThrow();
  expect(() => decodePairing({ ...p, status: "active" })).toThrow();
  expect(() => decodePairing({ ...p, revision: 0 })).toThrow();
  expect(decodePairingOffer({ pairing: p, code: "012345" }).code).toBe(
    "012345",
  );
  for (const code of [
    "too-short",
    "a".repeat(32),
    "12345",
    "1234567",
    "１２３４５６",
    123456,
  ]) {
    expect(() => decodePairingOffer({ pairing: p, code })).toThrow();
  }
});
