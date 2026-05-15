// Self-contained, no-imports, IIFE-wrapped Shockwave plugin polyfill.
// Imported as `?raw` text by the extension's isolated-world content
// script and injected as a `<script>` element so it runs in the page's
// main world — modifying `navigator.plugins` / `navigator.mimeTypes`
// where the page's detection scripts can see it.
(function () {
  'use strict';
  if (window.__dirplayerShockwavePluginInstalled__) return;
  window.__dirplayerShockwavePluginInstalled__ = true;

  var DIRECTOR_MIMETYPE = "application/x-director";
  var PLUGIN_NAME = "Shockwave for Director";
  var PLUGIN_DESCRIPTION = "Adobe Shockwave for Director 11.0.3.141";
  var PLUGIN_FILENAME = "dirplayer.js";

  // Bail when a real Shockwave plugin or another emulator already
  // advertises the entry, so we don't replace working detection.
  try {
    if (
      navigator.plugins.namedItem(PLUGIN_NAME) ||
      navigator.plugins.namedItem("Shockwave Plugin") ||
      navigator.mimeTypes.namedItem(DIRECTOR_MIMETYPE)
    ) return;
  } catch (e) { /* ignore */ }

  function MimeArr(srcArr) {
    this._items = [];
    this._named = Object.create(null);
    if (srcArr) {
      for (var i = 0; i < srcArr.length; i++) this.install(srcArr[i]);
    }
  }
  MimeArr.prototype.install = function (mimeType) {
    var idx = this._items.length;
    this._items.push(mimeType);
    this._named[mimeType.type] = mimeType;
    Object.defineProperty(this, mimeType.type, {
      configurable: true, enumerable: false, value: mimeType,
    });
    this[idx] = mimeType;
  };
  MimeArr.prototype.item = function (index) {
    return this._items[index >>> 0];
  };
  MimeArr.prototype.namedItem = function (name) {
    return this._named[name];
  };
  Object.defineProperty(MimeArr.prototype, 'length', {
    get: function () { return this._items.length; },
  });
  MimeArr.prototype[Symbol.iterator] = function () {
    return this._items[Symbol.iterator]();
  };
  Object.defineProperty(MimeArr.prototype, Symbol.toStringTag, {
    get: function () { return 'MimeTypeArray'; },
  });

  function PluginObj(name, description, filename) {
    MimeArr.call(this);
    this._name = name;
    this._description = description;
    this._filename = filename;
  }
  PluginObj.prototype = Object.create(MimeArr.prototype);
  PluginObj.prototype.constructor = PluginObj;
  Object.defineProperty(PluginObj.prototype, 'name', {
    get: function () { return this._name; },
  });
  Object.defineProperty(PluginObj.prototype, 'description', {
    get: function () { return this._description; },
  });
  Object.defineProperty(PluginObj.prototype, 'filename', {
    get: function () { return this._filename; },
  });
  Object.defineProperty(PluginObj.prototype, Symbol.toStringTag, {
    get: function () { return 'Plugin'; },
  });

  function PluginArr(srcArr) {
    this._items = [];
    this._named = Object.create(null);
    for (var i = 0; i < srcArr.length; i++) this.install(srcArr[i]);
  }
  PluginArr.prototype.install = function (plugin) {
    var idx = this._items.length;
    this._items.push(plugin);
    this._named[plugin.name] = plugin;
    Object.defineProperty(this, plugin.name, {
      configurable: true, enumerable: false, value: plugin,
    });
    this[idx] = plugin;
  };
  PluginArr.prototype.item = function (index) {
    return this._items[index >>> 0];
  };
  PluginArr.prototype.namedItem = function (name) {
    return this._named[name];
  };
  PluginArr.prototype.refresh = function () { /* no-op */ };
  Object.defineProperty(PluginArr.prototype, 'length', {
    get: function () { return this._items.length; },
  });
  PluginArr.prototype[Symbol.iterator] = function () {
    return this._items[Symbol.iterator]();
  };
  Object.defineProperty(PluginArr.prototype, Symbol.toStringTag, {
    get: function () { return 'PluginArray'; },
  });

  var plugin = new PluginObj(PLUGIN_NAME, PLUGIN_DESCRIPTION, PLUGIN_FILENAME);
  plugin.install({
    type: DIRECTOR_MIMETYPE,
    description: PLUGIN_DESCRIPTION,
    suffixes: 'dir,dxr,dcr',
    enabledPlugin: plugin,
  });

  // Wrap navigator.plugins
  if (!('install' in navigator.plugins)) {
    try {
      Object.defineProperty(navigator, 'plugins', {
        configurable: true,
        value: new PluginArr(navigator.plugins),
      });
    } catch (e) { return; /* native getter is non-configurable */ }
  }
  navigator.plugins.install(plugin);

  // Wrap navigator.mimeTypes and install plugin's MIME types
  if (plugin.length > 0 && !('install' in navigator.mimeTypes)) {
    try {
      Object.defineProperty(navigator, 'mimeTypes', {
        configurable: true,
        value: new MimeArr(navigator.mimeTypes),
      });
    } catch (e) { /* ignore */ }
  }
  if ('install' in navigator.mimeTypes) {
    for (var i = 0; i < plugin.length; i += 1) {
      navigator.mimeTypes.install(plugin[i]);
    }
  }
})();
