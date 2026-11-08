// Loopback relay that lets DSH-action use OpenCode Go as its model provider.
//
// OpenCode Go rejects every inference request that lacks an x-opencode-session
// header (400 MissingSessionID). DSH-action's controller-side credential proxy
// forwards a fixed header allowlist and deliberately drops every other header,
// including the hub header DSH itself sends. This relay runs on the GitHub
// Actions runner, receives the credential proxy's requests, adds a stable
// per-job session id, and forwards them to OpenCode Go over HTTPS.
//
// The real API key never reaches this process: the Action's credential proxy
// injects it into the Authorization / x-api-key headers of each request, and
// this relay only forwards those two headers to the trusted upstream.
//
// Environment:
//   DSH_OPENCODE_RELAY_PORT    listen port on 127.0.0.1 (default 8787)
//   DSH_OPENCODE_SESSION_ID    session id to send (default: random UUID)
//   DSH_OPENCODE_UPSTREAM      upstream Messages URL (default: OpenCode Go)

import { createServer } from "node:http";
import { randomUUID } from "node:crypto";
import { Readable } from "node:stream";

const HOST = "127.0.0.1";
const PORT = Number(process.env.DSH_OPENCODE_RELAY_PORT ?? "8787");
const SESSION_ID = process.env.DSH_OPENCODE_SESSION_ID ?? randomUUID();
const UPSTREAM =
  process.env.DSH_OPENCODE_UPSTREAM ??
  "https://opencode.ai/inference/go/anthropic/v1/messages";
const MESSAGES_PATH = "/anthropic/v1/messages";
const MAX_REQUEST_BYTES = 64 * 1024 * 1024;

function reply(res, status, payload) {
  res.writeHead(status, {
    "content-type": "application/json",
    "cache-control": "no-store",
  });
  res.end(JSON.stringify(payload));
}

async function readBody(req) {
  const chunks = [];
  let bytes = 0;
  for await (const chunk of req) {
    bytes += chunk.byteLength;
    if (bytes > MAX_REQUEST_BYTES) throw new Error("request too large");
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

function forwardHeaders(req) {
  const headers = {
    accept: req.headers.accept ?? "application/json",
    "content-type": req.headers["content-type"] ?? "application/json",
    "user-agent": "dsh-opencode-relay/1",
    "x-opencode-session": SESSION_ID,
  };
  // The Action proxy replaces the worker's placeholder with the real provider
  // credential in these two headers. Forward whichever forms it supplied.
  for (const name of ["authorization", "x-api-key", "anthropic-version", "anthropic-beta"]) {
    const value = req.headers[name];
    if (typeof value === "string" && value !== "") headers[name] = value;
  }
  return headers;
}

const server = createServer(async (req, res) => {
  if (req.method === "GET" && req.url === "/health") {
    reply(res, 200, { status: "ok" });
    return;
  }
  if (req.method !== "POST" || req.url !== MESSAGES_PATH) {
    reply(res, 404, { type: "error", error: { type: "relay_not_found" } });
    return;
  }

  let body;
  try {
    body = await readBody(req);
  } catch {
    reply(res, 413, { type: "error", error: { type: "relay_request_too_large" } });
    return;
  }

  const controller = new AbortController();
  res.once("close", () => controller.abort());

  try {
    const upstream = await fetch(UPSTREAM, {
      method: "POST",
      headers: forwardHeaders(req),
      body,
      redirect: "error",
      signal: controller.signal,
    });
    res.writeHead(upstream.status, {
      "content-type": upstream.headers.get("content-type") ?? "application/json",
      "cache-control": "no-store",
    });
    if (upstream.body === null) {
      res.end();
      return;
    }
    const stream = Readable.fromWeb(upstream.body);
    stream.on("error", () => res.destroy());
    res.once("close", () => stream.destroy());
    stream.pipe(res);
  } catch {
    if (!res.headersSent) {
      reply(res, 502, { type: "error", error: { type: "relay_upstream_failed" } });
    } else {
      res.destroy();
    }
  }
});

server.on("error", (error) => {
  console.error(`OpenCode Go relay failed: ${error.message}`);
  process.exit(1);
});

server.listen(PORT, HOST, () => {
  console.error(`OpenCode Go relay listening on http://${HOST}:${PORT}${MESSAGES_PATH}`);
});
