/**
 * Fake-plugin polyfill so Shockwave-detection scripts on legacy pages
 * find a "Shockwave for Director" entry in `navigator.plugins` and a
 * matching `application/x-director` MIME type. Adapted from Ruffle's
 * `web/packages/core/src/plugin-polyfill.ts`, trimmed to one plugin
 * and one MIME type.
 *
 * The detection patterns this satisfies:
 *
 *   for (const p of navigator.plugins)
 *     if (p.description.includes("Shockwave") &&
 *         p.description.includes("Director")) ...
 *
 *   navigator.mimeTypes["application/x-director"]?.enabledPlugin
 *
 * Modern browsers ignore the alternative VBScript / `CreateObject(
 * "SWCtl.SWCtl.10.1.1")` ActiveX path entirely, so polyfilling the
 * `navigator.plugins` array is sufficient for the JS-only path.
 *
 * No-op if a real Shockwave plugin already exists or another emulator
 * has already polyfilled the array.
 */

const DIRECTOR_MIMETYPE = "application/x-director";
const PLUGIN_NAME = "Shockwave for Director";
// Description MUST contain both "Shockwave" AND "Director" — that's what
// every legacy detection script greps for.
const PLUGIN_DESCRIPTION = "Adobe Shockwave for Director";
const PLUGIN_FILENAME = "dirplayer.js";

class DirPlayerMimeTypeArray implements MimeTypeArray {
    readonly #mimeTypes: MimeType[];
    readonly #namedMimeTypes: Record<string, MimeType>;

    constructor(mimeTypes?: MimeTypeArray) {
        this.#mimeTypes = [];
        this.#namedMimeTypes = {};
        if (mimeTypes) {
            for (let i = 0; i < mimeTypes.length; i++) {
                this.install(mimeTypes[i]!);
            }
        }
    }

    install(mimeType: MimeType): void {
        const wrapper = new DirPlayerMimeType(mimeType);
        const index = this.#mimeTypes.length;
        this.#mimeTypes.push(wrapper);
        this.#namedMimeTypes[mimeType.type] = wrapper;
        Object.defineProperty(this, wrapper.type, {
            configurable: true,
            enumerable: false,
            value: wrapper,
        });
        this[index] = wrapper;
    }

    item(index: number): MimeType {
        return this.#mimeTypes[index >>> 0]!;
    }

    namedItem(name: string): MimeType {
        return this.#namedMimeTypes[name]!;
    }

    get length(): number {
        return this.#mimeTypes.length;
    }

    [index: number]: MimeType;
    [name: string]: unknown;

    [Symbol.iterator](): ArrayIterator<MimeType> {
        return this.#mimeTypes[Symbol.iterator]() as ArrayIterator<MimeType>;
    }

    get [Symbol.toStringTag](): string {
        return "MimeTypeArray";
    }
}

class DirPlayerMimeType implements MimeType {
    readonly #mimeType: MimeType;
    constructor(mimeType: MimeType) { this.#mimeType = mimeType; }
    get type(): string { return this.#mimeType.type; }
    get description(): string { return this.#mimeType.description; }
    get suffixes(): string { return this.#mimeType.suffixes; }
    get enabledPlugin(): Plugin { return this.#mimeType.enabledPlugin; }
    get [Symbol.toStringTag](): string { return "MimeType"; }
}

class DirPlayerPlugin extends DirPlayerMimeTypeArray implements Plugin {
    readonly #name: string;
    readonly #description: string;
    readonly #filename: string;

    constructor(name: string, description: string, filename: string) {
        super();
        this.#name = name;
        this.#description = description;
        this.#filename = filename;
    }

    get name(): string { return this.#name; }
    get description(): string { return this.#description; }
    get filename(): string { return this.#filename; }

    override get [Symbol.toStringTag](): string { return "Plugin"; }
}

class DirPlayerPluginArray implements PluginArray {
    readonly #plugins: Plugin[];
    readonly #namedPlugins: Record<string, Plugin>;

    constructor(plugins: PluginArray) {
        this.#plugins = [];
        this.#namedPlugins = {};
        for (let i = 0; i < plugins.length; i++) {
            this.install(plugins[i]!);
        }
    }

    install(plugin: Plugin): void {
        const index = this.#plugins.length;
        this.#plugins.push(plugin);
        this.#namedPlugins[plugin.name] = plugin;
        Object.defineProperty(this, plugin.name, {
            configurable: true,
            enumerable: false,
            value: plugin,
        });
        this[index] = plugin;
    }

    item(index: number): Plugin {
        return this.#plugins[index >>> 0]!;
    }

    namedItem(name: string): Plugin {
        return this.#namedPlugins[name]!;
    }

    refresh(): void { /* no-op */ }

    [index: number]: Plugin;
    [name: string]: unknown;

    [Symbol.iterator](): ArrayIterator<Plugin> {
        return this.#plugins[Symbol.iterator]() as ArrayIterator<Plugin>;
    }

    get [Symbol.toStringTag](): string { return "PluginArray"; }

    get length(): number { return this.#plugins.length; }
}

declare global {
    interface PluginArray {
        install?: (plugin: Plugin) => void;
    }
    interface MimeTypeArray {
        install?: (mimeType: MimeType) => void;
    }
}

export const SHOCKWAVE_PLUGIN = new DirPlayerPlugin(
    PLUGIN_NAME,
    PLUGIN_DESCRIPTION,
    PLUGIN_FILENAME,
);

SHOCKWAVE_PLUGIN.install({
    type: DIRECTOR_MIMETYPE,
    description: PLUGIN_DESCRIPTION,
    suffixes: "dir,dxr,dcr",
    enabledPlugin: SHOCKWAVE_PLUGIN,
});

/**
 * Install the fake Shockwave plugin into `navigator.plugins` and its
 * MIME types into `navigator.mimeTypes`. Idempotent: bails when a
 * Shockwave-named plugin already exists (real plugin OR a previous
 * polyfill run).
 */
export function installShockwavePlugin(): void {
    // Skip when a real plugin or another emulator already advertises
    // Shockwave for Director — don't replace working detection.
    if (
        navigator.plugins.namedItem(PLUGIN_NAME) ||
        navigator.plugins.namedItem("Shockwave Plugin") ||
        navigator.mimeTypes.namedItem(DIRECTOR_MIMETYPE)
    ) {
        return;
    }

    if (!("install" in navigator.plugins) || !navigator.plugins["install"]) {
        Object.defineProperty(window, "PluginArray", {
            value: DirPlayerPluginArray,
        });
        Object.defineProperty(navigator, "plugins", {
            value: new DirPlayerPluginArray(navigator.plugins),
            writable: false,
        });
    }
    navigator.plugins.install!(SHOCKWAVE_PLUGIN);

    if (
        SHOCKWAVE_PLUGIN.length > 0 &&
        (!("install" in navigator.mimeTypes) || !navigator.mimeTypes["install"])
    ) {
        Object.defineProperty(window, "MimeTypeArray", {
            value: DirPlayerMimeTypeArray,
        });
        Object.defineProperty(window, "MimeType", {
            value: DirPlayerMimeType,
        });
        Object.defineProperty(navigator, "mimeTypes", {
            value: new DirPlayerMimeTypeArray(navigator.mimeTypes),
            writable: false,
        });
    }

    for (let i = 0; i < SHOCKWAVE_PLUGIN.length; i += 1) {
        navigator.mimeTypes.install!(SHOCKWAVE_PLUGIN[i]!);
    }
}
