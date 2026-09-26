import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { mkdir, readFile, writeFile, appendFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { createServer } from "node:net";
import { createWebServer } from "../server/server.mjs";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const cache = fileURLToPath(
  new URL("../../../.cache/mobile-web/", import.meta.url),
);
// Pinned official Windows release; verify the downloaded executable before running it.
const version = "2026.9.1";
const digest =
  "2837888cc0f5d58f15b6dc478376de90b4d3ba5241c7947455d1e0a0df429712";
const executable = `${cache}cloudflared-${version}.exe`;
let tunnel;
let server;
let stopping = false;
function stop() {
  stopping = true;
  tunnel?.kill();
  server?.close();
  server?.closeAllConnections();
}
for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, stop);
process.on("exit", stop);
try {
  const upstream = process.env.QIBAN_COORDINATOR_URL ?? "http://127.0.0.1:4318";
  const backend = new URL(upstream);
  if (
    backend.origin !== upstream ||
    backend.protocol !== "http:" ||
    backend.hostname !== "127.0.0.1"
  )
    throw new Error("QIBAN_COORDINATOR_URL must be a loopback HTTP origin.");
  try {
    const health = await fetch(`${upstream}/healthz`, {
      redirect: "error",
      signal: AbortSignal.timeout(3000),
    });
    if (!health.ok || (await health.json()).status !== "ok") throw new Error();
  } catch {
    throw new Error(
      "Start the coordinator first: npm run coordinator. For a custom port, set QIBAN_COORDINATOR_URL.",
    );
  }
  await readFile(new URL("../dist/index.html", import.meta.url));
  if (process.platform !== "win32" || process.arch !== "x64")
    throw new Error(
      "This HTTPS helper supports Windows x64. Use your own HTTPS reverse proxy on other platforms; see the guide.",
    );
  const port = Number(process.env.QIBAN_WEB_PORT ?? 4320);
  await new Promise((accept, reject) => {
    const probe = createServer();
    probe.once("error", reject);
    probe.listen(port, "127.0.0.1", () => probe.close(accept));
  });
  await mkdir(cache, { recursive: true });
  let bytes;
  try {
    bytes = await readFile(executable);
  } catch {
    console.log(
      `Downloading official cloudflared ${version} into .cache/mobile-web (about 55 MB)...`,
    );
    const response = await fetch(
      `https://github.com/cloudflare/cloudflared/releases/download/${version}/cloudflared-windows-amd64.exe`,
      { signal: AbortSignal.timeout(120000) },
    );
    if (!response.ok)
      throw new Error(
        "cloudflared download failed. Check GitHub connectivity and retry.",
      );
    bytes = Buffer.from(await response.arrayBuffer());
    if (createHash("sha256").update(bytes).digest("hex") !== digest)
      throw new Error(
        "cloudflared checksum mismatch; executable was not saved or started.",
      );
    await writeFile(executable, bytes, { flag: "wx" });
  }
  if (createHash("sha256").update(bytes).digest("hex") !== digest)
    throw new Error(
      "Cached cloudflared checksum mismatch. Remove only the cached executable and retry.",
    );
  // Isolated empty config avoids inheriting the user's named-tunnel settings.
  const config = `${cache}quick-tunnel.yml`;
  await writeFile(config, "{}\n");
  console.log(
    "Starting a temporary HTTPS link. No mail is sent until you request a code in the page.",
  );
  tunnel = spawn(
    executable,
    [
      "tunnel",
      "--config",
      config,
      "--no-autoupdate",
      "--protocol",
      "http2",
      "--url",
      `http://127.0.0.1:${port}`,
    ],
    { cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] },
  );
  const logFile = `${cache}tunnel.log`;
  await writeFile(logFile, "");
  const origin = await new Promise((accept, reject) => {
    let output = "";
    let link;
    const timeout = setTimeout(
      () =>
        reject(
          new Error(
            output.includes("ip=198.18.") || output.includes("ip=198.19.")
              ? "Tunnel DNS returned a proxy fake IP (198.18/15). Exclude *.argotunnel.com from fake-IP DNS or disable TUN, allow TCP 7844, then retry. Details: .cache/mobile-web/tunnel.log"
              : "Tunnel did not connect. Check outbound TCP 7844, DNS and proxy settings. Diagnostics: .cache/mobile-web/tunnel.log",
          ),
        ),
      45000,
    );
    const inspect = (chunk) => {
      void appendFile(logFile, chunk).catch(() => {});
      output = (output + chunk.toString()).slice(-16000);
      link ??= output.match(/https:\/\/[a-z0-9-]+\.trycloudflare\.com/)?.[0];
      if (link && output.includes("Registered tunnel connection")) {
        clearTimeout(timeout);
        accept(link);
      }
    };
    tunnel.stdout.on("data", inspect);
    tunnel.stderr.on("data", inspect);
    tunnel.once("error", () => {
      clearTimeout(timeout);
      reject(new Error("Unable to start cloudflared."));
    });
    tunnel.once("exit", () => {
      clearTimeout(timeout);
      reject(new Error("Tunnel exited before creating a link."));
    });
  });
  if (stopping) throw new Error("Stopped.");
  server = createWebServer({
    origin,
    upstream,
    dist: fileURLToPath(new URL("../dist", import.meta.url)),
  });
  await new Promise((accept, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", accept);
  });
  tunnel.once("exit", () => {
    if (!stopping) {
      console.error("HTTPS tunnel stopped. Restart npm run mobile:https.");
      process.exitCode = 1;
      stop();
    }
  });
  console.log(
    `\nOpen on BOTH computer and phone: ${origin}\nKeep this terminal and the coordinator running. Ctrl+C stops sharing.\nThe URL changes after restart. Use the same invited email; request separate codes at least 60 seconds apart.`,
  );
  await writeFile(`${cache}last-url.txt`, `${origin}\n`);
} catch (error) {
  console.error(
    `Mobile HTTPS: ${error instanceof Error ? error.message : "Startup failed."}`,
  );
  process.exitCode = 1;
  stop();
}
