import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { resolve, extname, sep } from "node:path";

const tokenPattern = /^qbs_[A-Za-z0-9_-]{43}$/;
const routes = new Set([
  "POST /api/auth/request-code",
  "POST /api/auth/verify-code",
  "GET /api/me",
  "GET /api/state",
  "GET /api/tasks",
  "POST /api/tasks",
  "POST /api/logout",
]);
const knownErrors = new Set([
  "invalid_request",
  "authentication_required",
  "invalid_code",
  "conflict",
  "capacity",
  "rate_limited",
  "busy",
  "authentication_unavailable",
  "storage_unavailable",
  "not_found",
]);

// The public origin is explicit. Neither Host nor forwarded headers select a backend.
export function createWebServer({
  origin,
  upstream = "http://127.0.0.1:4318",
  dist,
  fetchImpl = fetch,
}) {
  const publicUrl = new URL(origin);
  const backend = new URL(upstream);
  if (
    publicUrl.origin !== origin ||
    publicUrl.username ||
    publicUrl.password ||
    !(
      publicUrl.protocol === "https:" ||
      (publicUrl.protocol === "http:" && publicUrl.hostname === "127.0.0.1")
    ) ||
    backend.origin !== upstream ||
    backend.protocol !== "http:" ||
    backend.hostname !== "127.0.0.1"
  ) {
    throw new Error(
      "Use an exact HTTPS public origin and a loopback HTTP coordinator origin.",
    );
  }
  const secure = publicUrl.protocol === "https:";
  const cookieName = secure ? "__Host-qiban_session" : "qiban_local_session";
  const cookie = (token, maxAge) =>
    `${cookieName}=${token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=${maxAge}${secure ? "; Secure" : ""}`;
  const root = resolve(dist);
  let active = 0;
  let windowStart = Date.now();
  let requests = 0;
  const server = createServer(async (req, res) => {
    res.setHeader("Cache-Control", "no-store");
    res.setHeader("X-Content-Type-Options", "nosniff");
    res.setHeader("Referrer-Policy", "no-referrer");
    res.setHeader("X-Frame-Options", "DENY");
    res.setHeader(
      "Content-Security-Policy",
      "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
    );
    res.setHeader(
      "Permissions-Policy",
      "camera=(), microphone=(), geolocation=()",
    );
    if (secure) res.setHeader("Strict-Transport-Security", "max-age=86400");
    const send = (status, value) => {
      res.writeHead(status, {
        "Content-Type": "application/json; charset=utf-8",
      });
      res.end(JSON.stringify(value));
    };
    const fail = (status, code) => send(status, { error: { code } });
    if (req.headers.host !== publicUrl.host) return fail(403, "invalid_origin");
    if (secure && req.headers["x-forwarded-proto"] !== "https")
      return fail(403, "https_required");
    if (
      req.headers["sec-fetch-site"] === "cross-site" ||
      (req.headers.origin && req.headers.origin !== origin)
    )
      return fail(403, "invalid_origin");
    const path = req.url;
    if (path?.startsWith("/api/")) {
      if (
        !routes.has(`${req.method} ${path}`) &&
        !(
          req.method === "POST" &&
          /^\/api\/tasks\/[0-9a-f-]{36}\/cancel$/.test(path)
        )
      )
        return fail(404, "not_found");
      if (
        req.method === "POST" &&
        (req.headers.origin !== origin ||
          req.headers["x-qiban-request"] !== "1" ||
          req.headers["content-type"] !== "application/json")
      )
        return fail(403, "invalid_origin");
      if (Date.now() - windowStart >= 60_000) {
        windowStart = Date.now();
        requests = 0;
      }
      // Aggregate limits are intentional for this invitation-only integration instance;
      // forwarded IPs are not trusted as a security identity.
      if (++requests > 600 || active >= 16) return fail(429, "busy");
      active++;
      try {
        const anonymous = path.startsWith("/api/auth/");
        const cookies = (req.headers.cookie ?? "")
          .split(";")
          .map((c) => c.trim())
          .filter((c) => c.startsWith(`${cookieName}=`));
        const token =
          cookies.length === 1 ? cookies[0].slice(cookieName.length + 1) : "";
        if (!anonymous && !tokenPattern.test(token))
          return fail(401, "authentication_required");
        let body;
        if (req.method === "POST") {
          const chunks = [];
          let size = 0;
          for await (const chunk of req) {
            size += chunk.length;
            if (size > 8192) return fail(413, "invalid_request");
            chunks.push(chunk);
          }
          body = Buffer.concat(chunks).toString("utf8");
          try {
            JSON.parse(body);
          } catch {
            return fail(400, "invalid_request");
          }
        }
        const call = (suffix) =>
          fetchImpl(`${upstream}/v1/${suffix}`, {
            method: req.method,
            redirect: "error",
            signal: AbortSignal.timeout(12_000),
            headers: {
              "Content-Type": "application/json",
              ...(!anonymous ? { Authorization: `Bearer ${token}` } : {}),
            },
            body,
          });
        let response = await call(path === "/api/state" ? "me" : path.slice(5));
        let value = await response.json();
        if (response.ok && path === "/api/state") {
          // Capture one cookie for both reads. A login in another tab must never
          // pair account A's identity with account B's task list.
          const profile = value;
          response = await call("tasks");
          value = await response.json();
          if (response.ok) value = { profile, tasks: value };
        }
        if (!response.ok) {
          if (response.status === 401 && !anonymous)
            res.setHeader("Set-Cookie", cookie("", 0));
          return fail(
            response.status >= 400 && response.status < 600
              ? response.status
              : 502,
            knownErrors.has(value?.error?.code)
              ? value.error.code
              : "service_unavailable",
          );
        }
        if (path === "/api/auth/verify-code") {
          if (
            !tokenPattern.test(value.accessToken) ||
            !Number.isSafeInteger(value.expiresAt)
          )
            return fail(502, "service_unavailable");
          const maxAge = Math.max(
            0,
            Math.min(86400, Math.floor((value.expiresAt - Date.now()) / 1000)),
          );
          res.setHeader("Set-Cookie", cookie(value.accessToken, maxAge));
          return send(200, { authenticated: true });
        }
        if (path === "/api/logout") res.setHeader("Set-Cookie", cookie("", 0));
        return send(response.status, value);
      } catch {
        return fail(503, "service_unavailable");
      } finally {
        active--;
      }
    }
    if (
      req.method !== "GET" ||
      (path !== "/" && !/^\/assets\/[A-Za-z0-9_.-]+$/.test(path ?? ""))
    )
      return fail(404, "not_found");
    const file = resolve(root, path === "/" ? "index.html" : `.${path}`);
    if (!file.startsWith(root + sep)) return fail(404, "not_found");
    try {
      const bytes = await readFile(file);
      res.writeHead(200, {
        "Content-Type":
          {
            ".html": "text/html; charset=utf-8",
            ".js": "text/javascript; charset=utf-8",
            ".css": "text/css; charset=utf-8",
            ".svg": "image/svg+xml",
          }[extname(file)] ?? "application/octet-stream",
      });
      res.end(bytes);
    } catch {
      fail(404, "not_found");
    }
  });
  server.requestTimeout = 15_000;
  server.headersTimeout = 10_000;
  server.maxHeadersCount = 40;
  server.maxConnections = 64;
  return server;
}
