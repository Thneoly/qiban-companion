import { fileURLToPath } from "node:url";
import { createWebServer } from "./server.mjs";
const port = Number(process.env.QIBAN_WEB_PORT ?? 4320);
const origin = process.env.QIBAN_WEB_ORIGIN ?? `http://127.0.0.1:${port}`;
const server = createWebServer({
  origin,
  upstream: process.env.QIBAN_COORDINATOR_URL,
  dist: fileURLToPath(new URL("../dist", import.meta.url)),
});
server.on("error", () => {
  console.error(
    "Web port unavailable. Stop the existing Web process or select another port.",
  );
  process.exitCode = 1;
});
server.listen(port, "127.0.0.1", () =>
  console.log(`Qiban Web: ${origin} (loopback upstream only)`),
);
for (const signal of ["SIGINT", "SIGTERM"])
  process.on(signal, () => {
    server.close();
    server.closeAllConnections();
  });
