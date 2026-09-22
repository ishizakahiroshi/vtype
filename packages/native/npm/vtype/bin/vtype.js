#!/usr/bin/env node
"use strict";

// `npx @ishizakahiroshi/vtype …` / `vtype …` after `npm i -g`: starts the vtype binary built for
// this OS and CPU, which npm installed as one of the optional dependencies, with the same
// arguments. No postinstall step: npm picks the right optional dependency by its `os` and `cpu`.

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const BINARIES = {
  "win32-x64": "vtype.exe",
  "darwin-arm64": "vtype",
  "darwin-x64": "vtype",
  "linux-x64": "vtype",
};

function binaryPath() {
  const key = `${process.platform}-${process.arch}`;
  const file = BINARIES[key];
  if (!file) {
    throw new Error(`there is no vtype build for ${key} (Windows x64, macOS, Linux x64 only)`);
  }
  const pkg = `@ishizakahiroshi/vtype-${key}`;
  const candidates = [];
  try {
    candidates.push(path.join(path.dirname(require.resolve(`${pkg}/package.json`)), "bin", file));
  } catch {
    // not installed; see the next place
  }
  // In the source repository the platform packages sit next to this one.
  candidates.push(path.join(__dirname, "..", "..", `vtype-${key}`, "bin", file));
  const found = candidates.find((p) => fs.existsSync(p));
  if (!found) {
    throw new Error(`${pkg} is not installed; reinstall without --no-optional / --omit=optional`);
  }
  return found;
}

let bin;
try {
  bin = binaryPath();
} catch (e) {
  console.error(`vtype: ${e.message}`);
  process.exit(1);
}
const result = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`vtype: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
