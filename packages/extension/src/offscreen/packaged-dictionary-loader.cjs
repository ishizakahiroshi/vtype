// Stands in for kuromoji's BrowserDictionaryLoader in the build (build.mjs swaps it in).
//
// kuromoji's own browser loader fetches the dictionary with XMLHttpRequest from wherever it is
// told. This one can only read files inside the extension package: every request goes through
// chrome.runtime.getURL, which always yields a chrome-extension:// URL of this extension. That is
// what scripts/validate-extension.ps1 checks for, so "vtype sends nothing of its own" stays true
// of the shipped bytes. The files are gzip; the browser's DecompressionStream unpacks them.
//
// CommonJS on purpose: kuromoji requires its loader and calls it with `new`.

"use strict";

var DictionaryLoader = require("kuromoji/src/loader/DictionaryLoader");

function PackagedDictionaryLoader(dicPath) {
  DictionaryLoader.apply(this, [dicPath]);
}

PackagedDictionaryLoader.prototype = Object.create(DictionaryLoader.prototype);

PackagedDictionaryLoader.prototype.loadArrayBuffer = function (path, callback) {
  if (/^[a-z][a-z0-9+.-]*:/i.test(path) || path.indexOf("..") >= 0) {
    callback(new Error("the dictionary must be read from inside the extension"), null);
    return;
  }
  fetch(chrome.runtime.getURL(path))
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

module.exports = PackagedDictionaryLoader;
