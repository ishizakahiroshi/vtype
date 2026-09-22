// Disposable spike server. Node standard library only. Bind 127.0.0.1.
// requestTimeout is 0 so a 10 minute WebSocket is not cut by Node's 5 minute default.
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_PAYLOAD = 256 * 1024;

const port = Number(process.argv[2]);
const resultsPath = process.argv[3];
if (!Number.isInteger(port) || port < 1 || port > 65535 || !resultsPath) {
  console.error("usage: node server.mjs <port> <results.jsonl>");
  process.exit(1);
}

function append(payload) {
  const line = JSON.stringify({ t: new Date().toISOString(), payload }) + "\n";
  fs.appendFileSync(resultsPath, line);
}

function sendFrame(socket, opcode, payload) {
  if (!socket.writable || socket.destroyed) return;
  const data = Buffer.isBuffer(payload) ? payload : Buffer.from(payload);
  let header;
  if (data.length < 126) {
    header = Buffer.alloc(2);
    header[0] = 0x80 | opcode;
    header[1] = data.length;
  } else if (data.length < 65536) {
    header = Buffer.alloc(4);
    header[0] = 0x80 | opcode;
    header[1] = 126;
    header.writeUInt16BE(data.length, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x80 | opcode;
    header[1] = 127;
    header.writeUInt32BE(0, 2);
    header.writeUInt32BE(data.length, 6);
  }
  socket.write(Buffer.concat([header, data]));
}

function attachWebSocket(socket) {
  let buf = Buffer.alloc(0);
  let fragments = [];
  let fragOpcode = 0;

  socket.setTimeout(0);
  socket.setNoDelay(true);

  socket.on("data", (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    while (buf.length >= 2) {
      const b0 = buf[0];
      const b1 = buf[1];
      const fin = (b0 & 0x80) !== 0;
      const opcode = b0 & 0x0f;
      const masked = (b1 & 0x80) !== 0;
      let len = b1 & 0x7f;
      let offset = 2;
      if (len === 126) {
        if (buf.length < 4) return;
        len = buf.readUInt16BE(2);
        offset = 4;
      } else if (len === 127) {
        if (buf.length < 10) return;
        const hi = buf.readUInt32BE(2);
        const lo = buf.readUInt32BE(6);
        if (hi !== 0 || lo > MAX_PAYLOAD) {
          socket.destroy();
          return;
        }
        len = lo;
        offset = 10;
      }
      if (len > MAX_PAYLOAD) {
        socket.destroy();
        return;
      }
      const maskLen = masked ? 4 : 0;
      if (buf.length < offset + maskLen + len) return;
      const mask = masked ? buf.subarray(offset, offset + 4) : null;
      offset += maskLen;
      const payload = Buffer.from(buf.subarray(offset, offset + len));
      if (mask) {
        for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i & 3];
      }
      buf = Buffer.from(buf.subarray(offset + len));

      if (opcode === 0x8) {
        sendFrame(socket, 0x8, payload.subarray(0, Math.min(2, payload.length)));
        socket.end();
        return;
      }
      if (opcode === 0x9) {
        sendFrame(socket, 0xa, payload);
        continue;
      }
      if (opcode === 0xa) continue;
      if (opcode === 0x1 || opcode === 0x2 || opcode === 0x0) {
        if (opcode !== 0x0) {
          fragOpcode = opcode;
          fragments = [];
        }
        fragments.push(payload);
        if (fin) {
          const msg = Buffer.concat(fragments);
          fragments = [];
          if (fragOpcode === 0x1) {
            const text = msg.toString("utf8");
            let parsed;
            try {
              parsed = JSON.parse(text);
            } catch {
              parsed = { type: "invalid-json", raw: text.slice(0, 500) };
            }
            append(parsed);
          }
        }
      }
    }
  });

  socket.on("error", (err) => {
    try {
      append({ type: "ws-error", message: String(err && err.message ? err.message : err) });
    } catch {
      /* ignore a full disk; the process should stay up for the rest of the run */
    }
  });
}

const pagePath = path.join(__dirname, "page.html");

const server = http.createServer((req, res) => {
  const host = req.socket.localAddress || "";
  if (host !== "127.0.0.1" && host !== "::ffff:127.0.0.1") {
    res.writeHead(403);
    res.end();
    return;
  }
  let pathname = "/";
  try {
    pathname = new URL(req.url || "/", "http://127.0.0.1").pathname;
  } catch {
    pathname = "/";
  }
  if (req.method === "GET" && pathname === "/") {
    const html = fs.readFileSync(pagePath, "utf8").replaceAll("__PORT__", String(port));
    res.writeHead(200, {
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "no-store",
    });
    res.end(html);
    return;
  }
  res.writeHead(404);
  res.end();
});

server.requestTimeout = 0;
server.timeout = 0;

server.on("upgrade", (req, socket) => {
  const remote = socket.remoteAddress || "";
  if (remote !== "127.0.0.1" && remote !== "::ffff:127.0.0.1") {
    socket.destroy();
    return;
  }
  let pathname = "";
  try {
    pathname = new URL(req.url || "/", "http://127.0.0.1").pathname;
  } catch {
    pathname = "";
  }
  if (pathname !== "/ws") {
    socket.destroy();
    return;
  }
  const keyHeader = req.headers["sec-websocket-key"];
  const key = Array.isArray(keyHeader) ? keyHeader[0] : keyHeader;
  if (!key) {
    socket.destroy();
    return;
  }
  const accept = crypto.createHash("sha1").update(String(key) + GUID).digest("base64");
  socket.on("error", () => {});
  // Attach the frame reader before 101 so a fast client cannot race the listener.
  attachWebSocket(socket);
  socket.write(
    "HTTP/1.1 101 Switching Protocols\r\n" +
      "Upgrade: websocket\r\n" +
      "Connection: Upgrade\r\n" +
      `Sec-WebSocket-Accept: ${accept}\r\n` +
      "\r\n"
  );
  append({ type: "ws-open" });
});

server.on("error", (err) => {
  console.error(String(err && err.message ? err.message : err));
  process.exit(1);
});

server.listen(port, "127.0.0.1", () => {
  const addr = server.address();
  console.log(`listening ${addr.address}:${addr.port}`);
});
