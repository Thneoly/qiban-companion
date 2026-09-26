import { describe, expect, it } from "vitest";
import { decodeAccountSnapshot } from "./account";
const profile = {
  accountId: "f239880f-7073-4b19-ae78-cc961611a3a8",
  companionId: "3fd30ea7-fb31-4678-852e-d2584d93a110",
};
describe("native account boundary", () => {
  it("projects identity without allowing credentials or arbitrary native fields into UI", () => {
    const s = decodeAccountSnapshot({
      port: 4318,
      status: "authenticated",
      profile: { ...profile, token: "must-not-return" },
      tasks: [],
      accessToken: "must-not-return",
    });
    expect(s).toEqual({
      port: 4318,
      status: "authenticated",
      profile,
      tasks: [],
    });
  });
  it("rejects data attached to signed-out/pending snapshots and invalid identity/ports", () => {
    for (const v of [
      { port: 4318, status: "signed_out", profile, tasks: [] },
      { port: 4318, status: "logout_pending", profile, tasks: [] },
      { port: 80, status: "signed_out", profile: null, tasks: [] },
      {
        port: 4318,
        status: "authenticated",
        profile: { ...profile, accountId: "x" },
        tasks: [],
      },
    ])
      expect(() => decodeAccountSnapshot(v)).toThrow();
  });
});
