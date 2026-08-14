// Parser for Shockwave Projector (SPR.exe) launch commands.
//
// Archived Shockwave entries are published as a projector command line
// rather than a bare movie URL, because the movie is usually a
// leech-protection wrapper that is inert without the arguments the
// launcher supplied. Flashpoint front-ends expose the whole command as a
// `data-launch-command` attribute:
//
//   "…/wrapper_silentbaystudios.dcr"
//     --do "member('gameUrl').text = '…/agent_freeride.dcr'"
//     --bugfixShockwave3DBadDriverList
//
// This module turns such a string into the calls dirplayer needs. Plain ESM
// with no imports, so it is bundled into both the polyfill and the extension
// player (which share `EmbedPlayer`) and can be exercised directly by node.
//
// See docs/github_wiki/Projector-Launch-Commands.md for the full flag
// reference and the semantics each one maps onto.
// Flags that take a single value and map onto a LeechProtectionRemovalHelp
// Lingo handler of the same name. Value type matters: the integer ones must
// NOT be quoted when we synthesise the Lingo call.
var LPRH_STRING = [
  'setTheMoviePath',
  'setTheMovieName',
  'setTheEnvironment_shockMachineVersion',
  'setThePlatform',
  'setTheRunMode',
  'setTheEnvironment_productBuildVersion',
  'setTheProductVersion',
  'setTheEnvironment_osVersion',
];
var LPRH_INT = [
  'setTheEnvironment_shockMachine',
  'setTheMachineType',
  'forceTheExitLock',
  'forceTheSafePlayer',
];
// Flags that take no value.
var LPRH_BARE = [
  'disableGoToNetMovie',
  'disableGoToNetPage',
  'bugfixShockwave3DBadDriverList',
];

// Projector-only flags with no browser equivalent. Recognised so they are
// consumed rather than mistaken for a positional argument, and reported so a
// curator can see they were ignored.
var IGNORED_WITH_VALUE = ['preload', 'newScriptName', 'newScriptText', 'newScriptType', 'trace', 'traceLoad', 'traceLogFile'];
var IGNORED_BARE = ['versionStrict', 'noDirectX7'];

function lower(s) { return String(s).toLowerCase(); }
function findFlag(list, name) {
  var target = lower(name);
  for (var i = 0; i < list.length; i++) if (lower(list[i]) === target) return list[i];
  return null;
}

/// Split a command line into tokens, honouring double-quoted runs. Quotes
/// are stripped from the returned token but recorded, because the FIRST
/// token is the movie only when it is a bare/quoted path rather than a flag.
function tokenize(cmd) {
  var tokens = [];
  var i = 0;
  while (i < cmd.length) {
    while (i < cmd.length && /\s/.test(cmd[i])) i++;
    if (i >= cmd.length) break;
    if (cmd[i] === '"') {
      // Scan char-by-char rather than indexOf, so a BACKSLASH-ESCAPED quote
      // stays part of the token. Flashpoint payloads dodge nested quotes by
      // writing `'`, but a hand-written command may legitimately contain
      // `\"`, and truncating there would silently lose most of the payload.
      var buf = '';
      i++;
      while (i < cmd.length) {
        if (cmd[i] === '\\' && cmd[i + 1] === '"') { buf += '"'; i += 2; continue; }
        if (cmd[i] === '"') { i++; break; }
        buf += cmd[i++];
      }
      tokens.push({ value: buf, quoted: true });
    } else {
      var start = i;
      while (i < cmd.length && !/\s/.test(cmd[i])) i++;
      tokens.push({ value: cmd.slice(start, i), quoted: false });
    }
  }
  return tokens;
}

/// Lingo has ONE string delimiter, `"`. Launch commands write payloads with
/// `'` because the whole argument is itself double-quoted and nesting would
/// need escaping; the projector undoes that before evaluating. Only rewrite
/// when the payload has no `"` of its own — otherwise it is already
/// Lingo-quoted and an apostrophe in it is a literal apostrophe.
function normalizeQuotes(code) {
  if (code.indexOf('"') !== -1 || code.indexOf("'") === -1) return code;
  return code.replace(/'/g, '"');
}

function lingoString(value) {
  // Director has no escape syntax for a quote inside a literal; drop any.
  return '"' + String(value).replace(/"/g, '') + '"';
}

/**
 * Parse an SPR launch command.
 *
 * @param {string} cmd
 * @returns {{movieUrl: string|null, externalParams: Object, startupDo: string,
 *            startupDoBefore: string, startupGo: number, ignored: string[]}}
 */
export function parseLaunchCommand(cmd) {
  var result = {
    movieUrl: null,
    externalParams: {},
    startupDo: '',
    startupDoBefore: '',
    startupGo: 0,
    ignored: [],
  };
  if (!cmd || typeof cmd !== 'string') return result;

  // Some entries prefix the projector executable itself.
  var stripped = cmd.replace(/^\s*\S*SPR[A-Z]*\.exe\s+/i, '');
  var tokens = tokenize(stripped);
  // LPRH settings must run BEFORE the movie's own code, in command order,
  // and before any --do payload — that is the order the projector applies
  // them in.
  var lprhLines = [];
  var doLines = [];

  // `i` and `next` are declared OUTSIDE the loop on purpose: `next` consumes
  // the following token by advancing the cursor, so it has to close over the
  // loop variable. Declaring it inside the loop is what `no-loop-func` warns
  // about — one closure per iteration, all capturing the same `var`.
  var i;
  var next = function () { return i + 1 < tokens.length ? tokens[++i].value : ''; };

  for (i = 0; i < tokens.length; i++) {
    var tok = tokens[i];
    var raw = tok.value;

    if (raw.indexOf('--') !== 0) {
      // First positional token is the movie to load.
      if (result.movieUrl === null) result.movieUrl = raw;
      continue;
    }

    var flag = raw.slice(2);

    if (lower(flag) === 'do') { doLines.push(normalizeQuotes(next())); continue; }
    if (lower(flag) === 'dobefore') {
      var before = normalizeQuotes(next());
      result.startupDoBefore = result.startupDoBefore
        ? result.startupDoBefore + '\n' + before
        : before;
      continue;
    }
    if (lower(flag) === 'go') {
      var frame = parseInt(next(), 10);
      if (!isNaN(frame)) result.startupGo = frame;
      continue;
    }
    if (lower(flag) === 'setexternalparam') {
      var key = next();
      var value = next();
      if (key) result.externalParams[key] = value;
      continue;
    }

    var strFlag = findFlag(LPRH_STRING, flag);
    if (strFlag) { lprhLines.push(strFlag + '(' + lingoString(next()) + ')'); continue; }

    var intFlag = findFlag(LPRH_INT, flag);
    if (intFlag) {
      var n = parseInt(next(), 10);
      lprhLines.push(intFlag + '(' + (isNaN(n) ? 0 : n) + ')');
      continue;
    }

    var bareFlag = findFlag(LPRH_BARE, flag);
    if (bareFlag) { lprhLines.push(bareFlag + '()'); continue; }

    if (findFlag(IGNORED_WITH_VALUE, flag)) { next(); result.ignored.push(flag); continue; }
    if (findFlag(IGNORED_BARE, flag)) { result.ignored.push(flag); continue; }

    // Unknown flag: consume a following value if it is not itself a flag, so
    // a stray value is never mistaken for the movie URL.
    if (i + 1 < tokens.length && tokens[i + 1].value.indexOf('--') !== 0) i++;
    result.ignored.push(flag);
  }

  // The LPRH calls and the --do payload go through the same `--do` slot:
  // both are Lingo evaluated after the movie loads and before prepareMovie,
  // which is when the projector applies them.
  result.startupDo = lprhLines.concat(doLines).filter(Boolean).join('\n');
  return result;
}

/**
 * Apply a parsed command to the player. `api` is the vm-rust module (or any
 * object exposing the same functions), passed in so this file stays free of
 * imports and can be injected as a plain script.
 *
 * Call BEFORE loading the movie: the startup payloads are consumed by the
 * movie-init sequence.
 *
 * @returns {string|null} the movie URL to load, if the command named one.
 */
export function applyLaunchCommand(cmd, api) {
  var parsed = typeof cmd === 'string' ? parseLaunchCommand(cmd) : cmd;
  if (!api) return parsed.movieUrl;

  if (Object.keys(parsed.externalParams).length && api.set_external_params) {
    api.set_external_params(parsed.externalParams);
  }
  if (parsed.startupDoBefore && api.set_startup_do_before) {
    api.set_startup_do_before(parsed.startupDoBefore);
  }
  if (parsed.startupDo && api.set_startup_do) {
    api.set_startup_do(parsed.startupDo);
  }
  if (parsed.startupGo && api.set_startup_go) {
    api.set_startup_go(parsed.startupGo);
  }
  if (parsed.ignored.length) {
    console.warn('[DirPlayer] launch command: ignored projector-only flags:', parsed.ignored.join(', '));
  }
  return parsed.movieUrl;
}

/** Find a `data-launch-command` on the page, as archive front-ends emit. */
export function findLaunchCommand(root) {
  var el = (root || document).querySelector('[data-launch-command]');
  return el ? el.getAttribute('data-launch-command') : null;
}

/**
 * Find a `data-launch-command` in a fetched HTML *string* — for the dev
 * "Fetch & Load" flow, which pulls an archive entry page through the CORS
 * proxy and never puts it in the DOM.
 *
 * Regex rather than DOMParser so this stays usable outside a browser (node
 * tests, Electron main). The attribute value is HTML-escaped on the wire —
 * 9o3o writes `data-launch-command="&quot;…dcr&quot; --do &quot;…&quot;"` —
 * so entities must be decoded or every quoted token is lost.
 */
export function findLaunchCommandInHtml(html) {
  if (!html) return null;
  var m = /data-launch-command\s*=\s*"([^"]*)"/i.exec(html)
       || /data-launch-command\s*=\s*'([^']*)'/i.exec(html);
  if (!m) return null;
  return decodeHtmlEntities(m[1]).trim() || null;
}

/**
 * The archive's legacy content server, from a `data-legacy-server` attribute
 * (e.g. `https://infinity.unstable.life/Flashpoint/Legacy/htdocs`).
 *
 * A launch command names the movie by its ORIGINAL url — the site it was
 * published on, which is usually long dead. The archive serves the bytes from
 * this server instead, under a scheme-stripped mirror of that url.
 */
export function findLegacyServerInHtml(html) {
  if (!html) return null;
  var m = /data-legacy-server\s*=\s*"([^"]*)"/i.exec(html)
       || /data-legacy-server\s*=\s*'([^']*)'/i.exec(html);
  if (!m) return null;
  return decodeHtmlEntities(m[1]).trim().replace(/\/+$/, '') || null;
}

/**
 * Map an original url onto the archive's legacy server:
 *
 *   http://www.miniclip.com/games/x/y.dcr
 *   -> <legacyServer>/www.miniclip.com/games/x/y.dcr
 *
 * Returns the url unchanged when there is no legacy server, when it is already
 * pointing at one, or when it isn't an absolute http(s) url.
 */
export function toLegacyUrl(url, legacyServer) {
  if (!url || !legacyServer) return url;
  if (url.indexOf(legacyServer) === 0) return url;
  var m = /^https?:\/\/(.+)$/i.exec(url);
  if (!m) return url;
  return legacyServer + '/' + m[1];
}

function decodeHtmlEntities(s) {
  return String(s)
    .replace(/&quot;/gi, '"')
    .replace(/&#0*34;/g, '"')
    .replace(/&apos;/gi, "'")
    .replace(/&#0*39;/g, "'")
    .replace(/&lt;/gi, '<')
    .replace(/&gt;/gi, '>')
    .replace(/&nbsp;/gi, ' ')
    // `&amp;` LAST: decoding it earlier would let a literal `&amp;quot;`
    // become a quote and corrupt the command.
    .replace(/&amp;/gi, '&');
}
