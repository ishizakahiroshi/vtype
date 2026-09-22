// The desktop speech page's stand-in for kuromoji's BrowserDictionaryLoader (build.mjs swaps it
// in for src/speech/ only; the extension keeps src/offscreen/packaged-dictionary-loader.cjs).
//
// The page is served by the desktop app on 127.0.0.1 and has no chrome.runtime, so the
// dictionary is read from the page's own origin: only a relative path, resolved against the
// page's URL (which carries the per-launch token), is accepted. The files are gzip; the browser's
// DecompressionStream unpacks them.
//
// CommonJS on purpose: kuromoji requires its loader and calls it with `new`.

"use strict";

var DictionaryLoader = require("kuromoji/src/loader/DictionaryLoader");

function DesktopDictionaryLoader(dicPath) {
  DictionaryLoader.apply(this, [dicPath]);
}

DesktopDictionaryLoader.prototype = Object.create(DictionaryLoader.prototype);

DesktopDictionaryLoader.prototype.loadArrayBuffer = function (path, callback) {
  if (/^[a-z][a-z0-9+.-]*:/i.test(path) || path.indexOf("..") >= 0 || path.charAt(0) === "/") {
    callback(new Error("the dictionary must be read from the page's own directory"), null);
    return;
  }
  fetch(new URL(path, location.href).toString())
    .then(function (response) {
      if (!response.ok) throw new Error("dictionary file missing: " + path);
      return new Response(response.body.pipeThrough(new DecompressionStream("gzip"))).arrayBuffer();
    })
    .then(
      function (buffer) {
        callback(null, buffer);
      },
      function (err) {
        callback(err, null);
      },
    );
};

module.exports = DesktopDictionaryLoader;
