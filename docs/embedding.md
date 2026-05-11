# Embedding DirPlayer

DirPlayer can be embedded in a web page using standard HTML `<embed>` or `<object>` elements, the same markup originally used for Shockwave. The polyfill script detects these elements and replaces them with the DirPlayer runtime.

## Quick start

Add the polyfill script to your page, then use `<embed>` or `<object>` as you normally would:

```html
<script src="/path/to/dirplayer-polyfill.js"></script>

<embed
  src="/movies/game.dcr"
  type="application/x-director"
  width="640"
  height="480"
/>
```

The polyfill auto-initializes on page load. It detects Director embeds by MIME type (`application/x-director`, `application/x-shockwave-director`) and file extension (`.dcr`, `.dxr`, `.dir`).

## Polyfill script attributes

These attributes go on the `<script>` tag that loads the polyfill:

| Attribute | Description |
|-----------|-------------|
| `data-require-click` | Show a "Click to Play" overlay before loading the movie. Useful for pages where audio autoplay is blocked. |
| `data-disable-flash` | Skip Ruffle entirely. Flash cast members will be invisible but Lingo Flash calls won't throw errors. |
| `data-manual-init` | Suppress auto-initialization. Call `window.DirPlayer.init()` yourself when ready. |
| `data-ruffle-url` | Override the URL of the Ruffle bundle (`ruffle/dirplayer_ruffle.js`). Defaults to a sibling `ruffle/` directory next to the polyfill script. |

```html
<!-- Require a click before playing, and disable Flash support -->
<script
  src="/polyfill/dirplayer-polyfill.js"
  data-require-click
  data-disable-flash
></script>
```

## `<embed>` element

```html
<embed
  src="/movies/game.dcr"
  type="application/x-director"
  width="640"
  height="480"
  sw1="paramValue"
  sw2="anotherValue"
  data-enable-gestures
/>
```

### `<embed>` attribute reference

| Attribute | Description |
|-----------|-------------|
| `src` | URL of the Director movie. Also accepted as `data-src` (useful when the browser would otherwise try to load the plugin). |
| `type` | MIME type. Use `application/x-director` or `application/x-shockwave-director`. |
| `width`, `height` | Dimensions of the player. Numeric values are interpreted as pixels. |
| `sw1` … `sw30` | External parameters passed to the movie (Director's classic `externalParamValue()` API). Numbered consecutively from 1; the loop stops at the first missing number. |
| `data-sw-<name>` | Named external parameter. For example, `data-sw-mode="game"` makes `externalParamValue("mode")` return `"game"`. These are merged with `sw1`…`sw30`. |
| `data-enable-gestures` | Enable pan/zoom gestures (see [Gestures](#gestures)). |

## `<object>` element

```html
<object
  classid="clsid:166B1BCA-3F9C-11CF-8075-444553540000"
  type="application/x-director"
  width="640"
  height="480"
  data-enable-gestures
>
  <param name="src" value="/movies/game.dcr" />
  <param name="sw1" value="paramValue" />
  <param name="sw2" value="anotherValue" />
  <param name="enableGestures" value="true" />
  <!-- Fallback for non-plugin browsers -->
  <embed src="/movies/game.dcr" type="application/x-director"
         width="640" height="480" />
</object>
```

Detection fires on:
- `classid="clsid:166B1BCA-3F9C-11CF-8075-444553540000"` (Shockwave Director)
- `classid="clsid:7FD1D18D-7787-11D2-B3F7-00600832B7C6"` (Director 7+)
- `type="application/x-director"` or `type="application/x-shockwave-director"`
- A `<param name="src">` value with a `.dcr`, `.dxr`, or `.dir` extension

### `<param>` reference

| `name` | Description |
|--------|-------------|
| `src` | URL of the movie (case-insensitive). |
| `sw1` … `sw30` | External parameters (same semantics as `<embed>`). |
| `enableGestures` | Set to `"true"` to enable pan/zoom gestures. |

You can also put `data-sw-<name>` attributes directly on the `<object>` element to pass named external params, just like with `<embed>`.

## Nested `<embed>` inside `<object>`

The classic cross-browser pattern nests an `<embed>` inside an `<object>`. DirPlayer handles this correctly:

- When a matching `<embed>` is found inside an `<object>`, the entire `<object>` is replaced (not just the `<embed>`).
- Width and height from the `<object>` take precedence over those on the inner `<embed>`.
- `data-enable-gestures` on the `<object>` is inherited by the inner `<embed>`.

## Dynamic elements

The polyfill installs a `MutationObserver` after initialization. Any `<embed>` or `<object>` added to the page after load — including elements nested inside dynamically added container nodes — is detected and replaced automatically.

## Accessing params from Lingo

External params appear in Director's standard API:

```lingo
-- Number of params passed by the host page
put externalParamCount()

-- Get the name of param at 1-based index
put externalParamName(1)   -- "sw1", or custom name

-- Get value by name (case-insensitive) or 1-based index
put externalParamValue("sw1")
put externalParamValue(1)
```

Named params (`data-sw-*` / `<param>` with arbitrary names) are accessible by their exact names. The `sw1`…`sw30` shorthand params use `"sw1"`, `"sw2"`, … as their keys.

### Reserved param names

DirPlayer reads these params itself; they are also visible to Lingo via `externalParamValue()`:

| Param | Description |
|-------|-------------|
| `_moviePath` | Overrides the movie label used for `gotoMovie` and similar navigation. |
| `_runMode` | Affects how `gotoMovie` resolves relative movie paths. |
| `multiuser_websocket_path` | Path suffix appended to the WebSocket URL for Multiuser connections (see [Multiuser](multiuser.md)). |
| `multiuser_websocket_ssl` | Force SSL for Multiuser WebSocket connections (`"true"` / `"1"`, see [Multiuser](multiuser.md)). |

## Gestures

When `data-enable-gestures` is present (or `enableGestures="true"` in a `<param>`), the player gains pan and zoom capabilities:

| Interaction | Action |
|-------------|--------|
| Trackpad pinch | Zoom in/out, anchored at the pinch centroid |
| Trackpad two-finger drag | Pan the stage |
| Middle mouse button drag | Pan the stage |
| Pan toggle button (overlay) | Lock into single-finger/mouse pan mode |
| Minimap (overlay) | Shows current viewport position; drag to navigate |

The pan/zoom state is maintained independently of the movie's own coordinate system. The stage is auto-centered at its natural size until the user deliberately pans or zooms, after which the position is sticky.

## Flash configuration

To configure the Flash (Ruffle) subsystem before loading, call `window.DirPlayer.configureFlash()` before auto-init fires, or use `data-manual-init` and call it before `window.DirPlayer.init()`:

```html
<script src="/polyfill/dirplayer-polyfill.js" data-manual-init></script>
<script>
  window.DirPlayer.configureFlash({
    disableFlash: false,
    socketProxy: [
      { host: "game.example.com", port: 1626, proxyUrl: "wss://proxy.example.com/mus" }
    ],
    fetchRewriteRules: [
      {
        pathPrefix: "/gateway/",
        targetHost: "api.example.com",
        targetPort: "443",
        targetProtocol: "https:"
      }
    ]
  });
  window.DirPlayer.init();
</script>
```

See [Multiuser](multiuser.md) for the `socketProxy` details.
