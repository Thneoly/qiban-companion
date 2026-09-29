import { describe, expect, it } from "vitest";
import {
  decodeDocument,
  decodeDocuments,
  deriveDocumentPhase,
  relativeTime,
  type DocumentAction,
} from "./remote-documents";
import type { Pairing } from "./pairing";
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
      scope: "document_excerpt" as const,
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

const NOW = 10_000_000;
const EXPIRES = NOW + 120_000;
function doc(
  state: string,
  currentRole: DocumentAction["currentRole"] = "controller",
): DocumentAction {
  return {
    ...example(),
    authorization: {
      ...example().authorization,
      pairingId: uuid,
      expiresAt: EXPIRES,
      state,
    },
    currentRole,
  };
}
function pairing(online: boolean | null): Pairing {
  return {
    id: uuid,
    desktopId: uuid,
    desktopName: "客厅电脑",
    controllerId: uuid,
    controllerName: "我的手机",
    scope: "document_excerpt",
    revision: 2,
    status: "active",
    expiresAt: EXPIRES,
    currentRole: "controller",
    desktopOnline: online,
    desktopLastHeartbeatAt: online === null ? null : NOW - 4000,
    desktopCapabilities: ["document_excerpt"],
  };
}

describe("document phase derivation", () => {
  const states = [
    "awaiting_confirmation",
    "confirmed",
    "admitted",
    "cancel_requested",
    "completed",
    "failed",
    "cancelled",
    "unknown",
  ] as const;
  it("derives every state under online/offline/null presence without crashing", () => {
    for (const state of states) {
      for (const online of [true, false, null] as const) {
        for (const role of ["controller", "desktop"] as const) {
          const phase = deriveDocumentPhase(
            doc(state, role),
            online === null ? null : pairing(online),
            NOW,
          );
          expect(phase.key).toBeTruthy();
          expect(phase.label.length).toBeGreaterThan(0);
          expect(phase.hint.length).toBeGreaterThan(0);
        }
      }
    }
  });
  it("splits confirmed into waiting-desktop hints and admitted into executing labels by presence", () => {
    const confirmedOnline = deriveDocumentPhase(doc("confirmed"), pairing(true), NOW);
    expect(confirmedOnline.key).toBe("waiting_desktop");
    expect(confirmedOnline.label).toContain("已确认，等待电脑保存（剩余");
    expect(confirmedOnline.hint).toBe("电脑在线，一般数秒内开始；确认不等于已执行。");
    expect(deriveDocumentPhase(doc("confirmed"), pairing(false), NOW).hint).toContain(
      "电脑当前离线（最后联系 4 秒前）",
    );
    expect(deriveDocumentPhase(doc("confirmed"), pairing(null), NOW).hint).toContain(
      "电脑在线状态未知",
    );
    expect(deriveDocumentPhase(doc("confirmed"), undefined, NOW).key).toBe(
      "waiting_desktop",
    );
    const executing = deriveDocumentPhase(doc("admitted"), pairing(true), NOW);
    expect(executing.key).toBe("executing");
    expect(executing.label).toBe("执行中（推断）");
    expect(deriveDocumentPhase(doc("admitted"), pairing(false), NOW).label).toBe(
      "已开始，电脑离线",
    );
    expect(deriveDocumentPhase(doc("admitted"), pairing(null), NOW).label).toBe(
      "已准入，等待保存回执",
    );
  });
  it("labels executing as inferred only when the desktop lease says online", () => {
    for (const state of states) {
      for (const online of [true, false, null] as const) {
        const label = deriveDocumentPhase(
          doc(state),
          online === null ? null : pairing(online),
          NOW,
        ).label;
        const mustBeInferred = state === "admitted" && online === true;
        expect(label.includes("推断")).toBe(mustBeInferred);
      }
    }
  });
  it("formats the countdown window and clamps at expiry without inventing a terminal state", () => {
    const fresh = deriveDocumentPhase(doc("confirmed"), pairing(true), NOW);
    expect(fresh.label).toBe("已确认，等待电脑保存（剩余 2:00）");
    const last = deriveDocumentPhase(doc("confirmed"), pairing(true), EXPIRES - 1);
    expect(last.label).toBe("已确认，等待电脑保存（剩余 0:00）");
    const past = deriveDocumentPhase(doc("confirmed"), pairing(true), EXPIRES);
    expect(past.label).toBe("已确认，等待电脑保存（已到有效期）");
    expect(past.hint).toContain("以服务端刷新核对为准");
    expect(past.key).toBe("waiting_desktop");
    const awaiting = deriveDocumentPhase(doc("awaiting_confirmation"), null, NOW);
    expect(awaiting.label).toBe("等待手机确认（剩余 2:00）");
  });
  it("keeps the e2e-asserted labels as substrings and ignores presence on terminal states", () => {
    expect(deriveDocumentPhase(doc("awaiting_confirmation"), null, NOW).label).toContain(
      "等待手机确认",
    );
    for (const online of [true, false, null] as const) {
      expect(
        deriveDocumentPhase(doc("cancelled"), online === null ? null : pairing(online), NOW)
          .label,
      ).toBe("已取消");
      expect(
        deriveDocumentPhase(doc("completed"), online === null ? null : pairing(online), NOW)
          .label,
      ).toBe("已保存并核验");
      expect(
        deriveDocumentPhase(doc("failed"), online === null ? null : pairing(online), NOW)
          .label,
      ).toBe("未保存成功");
    }
    const completed = deriveDocumentPhase(doc("completed"), pairing(true), NOW);
    expect(completed.hint).toContain("服务端接收回报时核对");
  });
  it("treats a mismatched pairing row as no presence instead of decorating", () => {
    const other = { ...pairing(true), id: "0b0b0b0b-1111-2222-3333-444444444444" };
    expect(deriveDocumentPhase(doc("admitted"), other, NOW).label).toBe(
      "已准入，等待保存回执",
    );
    expect(deriveDocumentPhase(doc("unknown"), other, NOW).label).toBe(
      "结果未知，需要核对",
    );
  });
  it("derives the unknown-recovery guidance from presence", () => {
    expect(deriveDocumentPhase(doc("unknown"), pairing(true), NOW).label).toBe(
      "结果未知，待自动核对",
    );
    expect(
      deriveDocumentPhase(doc("unknown"), pairing(false), NOW).hint,
    ).toContain("无需重新确认");
    expect(deriveDocumentPhase(doc("cancel_requested"), pairing(false), NOW).hint).toContain(
      "恢复联网后会核对停止结果",
    );
  });
  it("keeps the awaiting-confirmation hint independent of presence", () => {
    const online = deriveDocumentPhase(doc("awaiting_confirmation"), pairing(true), NOW);
    const offline = deriveDocumentPhase(doc("awaiting_confirmation"), pairing(false), NOW);
    const absent = deriveDocumentPhase(doc("awaiting_confirmation"), null, NOW);
    expect(online.hint).toBe(offline.hint);
    expect(online.hint).toBe(absent.hint);
    expect(online.hint).not.toContain("最后联系");
    expect(online.hint).not.toContain("在线");
    expect(online.hint).not.toContain("离线");
  });
  it("falls back to 从未 when the desktop never sent a heartbeat", () => {
    const silent = { ...pairing(false), desktopLastHeartbeatAt: null };
    expect(deriveDocumentPhase(doc("confirmed"), silent, NOW).hint).toContain(
      "最后联系 从未）",
    );
    expect(deriveDocumentPhase(doc("unknown"), silent, NOW).hint).toContain(
      "最后联系 从未）",
    );
  });
});

describe("relativeTime", () => {
  it("formats null, seconds, minutes and future stamps (display only)", () => {
    expect(relativeTime(null, NOW)).toBe("");
    expect(relativeTime(NOW - 59_000, NOW)).toBe("59 秒前");
    expect(relativeTime(NOW - 60_000, NOW)).toBe("1 分钟前");
    expect(relativeTime(NOW - 119_000, NOW)).toBe("2 分钟前");
    expect(relativeTime(NOW + 30_000, NOW)).toBe("0 秒前");
  });
});
