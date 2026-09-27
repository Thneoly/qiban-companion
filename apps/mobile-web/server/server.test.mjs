import { test } from "node:test";
import assert from "node:assert/strict";
import { request } from "node:http";
import { fileURLToPath } from "node:url";
import { createWebServer } from "./server.mjs";

const origin = "https://test.trycloudflare.com";
const token = `qbs_${"A".repeat(43)}`;
async function fixture(t, fetchImpl) {
  const server = createWebServer({
    origin,
    dist: fileURLToPath(new URL("../dist", import.meta.url)),
    fetchImpl,
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => {
    server.close();
    server.closeAllConnections();
  });
  return (path, { method = "GET", headers = {}, body } = {}) =>
    new Promise((resolve, reject) => {
      const req = request(
        {
          host: "127.0.0.1",
          port: server.address().port,
          path,
          method,
          headers: {
            Host: new URL(origin).host,
            "X-Forwarded-Proto": "https",
            ...(method === "POST"
              ? {
                  Origin: origin,
                  "X-Qiban-Request": "1",
                  "Content-Type": "application/json",
                }
              : {}),
            ...headers,
          },
        },
        (res) => {
          let text = "";
          res.setEncoding("utf8");
          res.on("data", (c) => (text += c));
          res.on("end", () =>
            resolve({ status: res.statusCode, headers: res.headers, text }),
          );
        },
      );
      req.on("error", reject);
      req.end(body);
    });
}
test("login hides token, uses bounded secure cookie; logout revokes and expires", async (t) => {
  const calls = [];
  const call = await fixture(t, async (url, options) => {
    calls.push([url, options]);
    return Response.json(
      url.endsWith("verify-code")
        ? { accessToken: token, expiresAt: Date.now() + 1800000 }
        : {},
    );
  });
  const login = await call("/api/auth/verify-code", {
    method: "POST",
    body: "{}",
  });
  assert.equal(login.status, 200);
  assert.equal(login.text, '{"authenticated":true}');
  const cookie = login.headers["set-cookie"][0];
  assert.match(cookie, /__Host-qiban_session=qbs_/);
  assert.match(
    cookie,
    /HttpOnly; SameSite=Strict; Max-Age=(?:17\d\d|1800); Secure/,
  );
  assert.equal(login.headers["cache-control"], "no-store");
  const logout = await call("/api/logout", {
    method: "POST",
    headers: { Cookie: cookie.split(";")[0] },
    body: '{"allSessions":true}',
  });
  assert.equal(logout.status, 200);
  assert.match(logout.headers["set-cookie"][0], /Max-Age=0/);
  assert.equal(calls[1][1].headers.Authorization, `Bearer ${token}`);
  assert.equal(calls[1][1].redirect, "error");
});
test("rejects wrong host/origin, insecure proxy, cross-site, unknown routes, bearer bypass and duplicate cookies", async (t) => {
  let calls = 0;
  const call = await fixture(t, () => {
    calls++;
    return Response.json({});
  });
  for (const options of [
    { headers: { Host: "evil.example" } },
    { headers: { Origin: "https://evil.example" } },
    { headers: { "X-Forwarded-Proto": "http" } },
    { headers: { "Sec-Fetch-Site": "cross-site" } },
    { method: "POST", headers: { "X-Qiban-Request": "" }, body: "{}" },
  ])
    assert.equal((await call("/api/tasks", options)).status, 403);
  assert.equal((await call("/v1/tasks")).status, 404);
  assert.equal((await call("/api/files")).status, 404);
  assert.equal((await call("/api/tasks?accountId=someone")).status, 404);
  assert.equal(
    (
      await call("/api/tasks", {
        headers: { Authorization: `Bearer ${token}` },
      })
    ).status,
    401,
  );
  assert.equal(
    (
      await call("/api/tasks", {
        headers: {
          Cookie: `__Host-qiban_session=${token}; __Host-qiban_session=${token}`,
        },
      })
    ).status,
    401,
  );
  assert.equal(calls, 0);
});
test("body limit, invalid JSON, upstream failure and session expiry fail closed", async (t) => {
  let mode = "expired";
  const call = await fixture(t, () => {
    if (mode === "offline") throw new Error("secret upstream error");
    return Response.json(
      { error: { code: "authentication_required" } },
      { status: 401 },
    );
  });
  assert.equal(
    (
      await call("/api/auth/request-code", {
        method: "POST",
        body: "x".repeat(9000),
      })
    ).status,
    413,
  );
  assert.equal(
    (await call("/api/auth/request-code", { method: "POST", body: "{" }))
      .status,
    400,
  );
  const response = await call("/api/me", {
    headers: { Cookie: `__Host-qiban_session=${token}` },
  });
  assert.equal(response.status, 401);
  assert.match(response.headers["set-cookie"][0], /Max-Age=0/);
  mode = "offline";
  const offline = await call("/api/auth/request-code", {
    method: "POST",
    body: "{}",
  });
  assert.equal(offline.status, 503);
  assert.doesNotMatch(offline.text, /secret/);
});
test("serves built app with CSP but never source, database or traversal paths", async (t) => {
  const call = await fixture(t, () => {
    throw new Error("unexpected");
  });
  const response = await call("/");
  assert.equal(response.status, 200);
  assert.match(
    response.headers["content-security-policy"],
    /frame-ancestors 'none'/,
  );
  for (const path of [
    "/server/server.mjs",
    "/.env",
    "/assets/../server/server.mjs",
    "/accounts.db",
  ])
    assert.equal((await call(path)).status, 404);
});
test("snapshot uses the same captured credential for identity and tasks; partial failure reveals neither", async (t) => {
  const calls = [];
  let expired = false;
  const call = await fixture(t, async (url, options) => {
    calls.push(options.headers.Authorization);
    if (url.endsWith("/me"))
      return Response.json({
        accountId: "account-a",
        companionId: "companion-a",
      });
    return expired
      ? Response.json(
          { error: { code: "authentication_required" } },
          { status: 401 },
        )
      : Response.json([{ id: "task-a" }]);
  });
  const headers = { Cookie: `__Host-qiban_session=${token}` };
  const first = await call("/api/state", { headers });
  assert.equal(first.status, 200);
  assert.deepEqual(JSON.parse(first.text), {
    profile: { accountId: "account-a", companionId: "companion-a" },
    tasks: [{ id: "task-a" }],
  });
  assert.deepEqual(calls, [`Bearer ${token}`, `Bearer ${token}`]);
  expired = true;
  const second = await call("/api/state", { headers });
  assert.equal(second.status, 401);
  assert.doesNotMatch(second.text, /account-a|companion-a|task-a/);
});

test("pairing gateway only permits explicit control routes and never exposes action admission", async (t) => {
  const calls = [];
  const call = await fixture(t, async (url, options) => {
    calls.push(url);
    assert.equal(options.headers.Authorization, `Bearer ${token}`);
    return Response.json([]);
  });
  const headers = { Cookie: `__Host-qiban_session=${token}` };
  assert.equal((await call("/api/pairings", { headers })).status, 200);
  for (const path of [
    "/api/pairings/preview",
    "/api/pairings/accept",
    "/api/pairings/00000000-0000-0000-0000-000000000000/revoke",
  ]) {
    assert.equal(
      (await call(path, { method: "POST", headers, body: "{}" })).status,
      200,
    );
    assert.equal(
      (
        await call(path, {
          method: "POST",
          headers: { ...headers, Origin: "https://evil.example" },
          body: "{}",
        })
      ).status,
      403,
    );
    assert.equal(
      (await call(path, { method: "POST", body: "{}" })).status,
      401,
    );
  }
  for (const path of [
    "/api/pairings/offer",
    "/api/actions/admit",
    "/api/execute",
  ]) {
    assert.equal(
      (await call(path, { method: "POST", headers, body: "{}" })).status,
      404,
    );
  }
  assert.equal(calls.length, 4);
});

test("document gateway permits confirmation and cancellation but denies submission, admission and receipts", async (t) => {
  const call = await fixture(t, async () => Response.json([]));
  const headers = { Cookie: `__Host-qiban_session=${token}` };
  const id = "00000000-0000-0000-0000-000000000000";
  assert.equal((await call("/api/documents", { headers })).status, 200);
  for (const op of ["confirm", "cancel"]) {
    assert.equal(
      (
        await call(`/api/documents/${id}/${op}`, {
          method: "POST",
          headers,
          body: "{}",
        })
      ).status,
      200,
    );
    assert.equal(
      (
        await call(`/api/documents/${id}/${op}`, {
          method: "POST",
          headers: { ...headers, Origin: "https://evil.example" },
          body: "{}",
        })
      ).status,
      403,
    );
  }
  for (const path of [
    "/api/documents",
    `/api/documents/${id}/admit`,
    `/api/documents/${id}/receipt`,
  ])
    assert.equal(
      (await call(path, { method: "POST", headers, body: "{}" })).status,
      404,
    );
});

test("pairing lockout stays distinct from email cooldown and does not clear login", async (t) => {
  const call = await fixture(t, async () =>
    Response.json({ error: { code: "pairing_rate_limited" } }, { status: 429 }),
  );
  const response = await call("/api/pairings/preview", {
    method: "POST",
    headers: { Cookie: `__Host-qiban_session=${token}` },
    body: JSON.stringify({ code: "012345" }),
  });
  assert.equal(response.status, 429);
  assert.deepEqual(JSON.parse(response.text), {
    error: { code: "pairing_rate_limited" },
  });
  assert.equal(response.headers["set-cookie"], undefined);
});
