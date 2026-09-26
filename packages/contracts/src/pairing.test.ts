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
  };
  expect(decodePairing({ ...p, codeHash: "secret", token: "secret" })).toEqual(
    p,
  );
  expect(() => decodePairing({ ...p, scope: "shell" })).toThrow();
  expect(() => decodePairing({ ...p, status: "active" })).toThrow();
  expect(() => decodePairing({ ...p, revision: 0 })).toThrow();
  expect(() => decodePairingOffer({ pairing: p, code: "too-short" })).toThrow();
});
