// Serves packages/extension/testbed/ on http://127.0.0.1:8787/ (loopback only).
// Content scripts only run on http(s) pages, so the testbed cannot be opened as file://.
//   node packages/extension/testbed/serve.mjs            (port 8787)
//   node packages/extension/testbed/serve.mjs 9000       (another port)

import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { dirname, extname, join, normalize, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const port = Number(process.argv[2] ?? 8787);
const types = { ".html": "text/html; charset=utf-8", ".css": "text/css", ".js": "text/javascript" };

createServer(async (req, res) => {
  const url = new URL(req.url ?? "/", "http://127.0.0.1");
  const rel = normalize(decodeURIComponent(url.pathname === "/" ? "/hostile.html" : url.pathname));
  const file = join(root, rel);
  if (!file.startsWith(root + sep)) {
    res.writeHead(403).end();
    return;
  }
  try {
    const body = await readFile(file);
    res.writeHead(200, { "content-type": types[extname(file)] ?? "application/octet-stream" }).end(body);
  } catch {
    res.writeHead(404).end("not found");
  }
}).listen(port, "127.0.0.1", () => {
  console.log(`vtype testbed: http://127.0.0.1:${port}/hostile.html`);
});
