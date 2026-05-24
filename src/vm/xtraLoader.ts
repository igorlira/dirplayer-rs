import { loadExternalXtraFromUrl } from 'dirplayer-js-api';
import { register_external_xtra_plugin } from 'vm-rust';

/**
 * Fetch, instantiate, and register a list of external xtra plugin WASM modules.
 * Must be called before `load_movie_file`.
 *
 * Each URL points to a .wasm file built with the dirplayer-xtra SDK.
 * The function is intentionally forgiving — a failed plugin logs a warning
 * but does not abort the others.
 */
export async function loadXtraPlugins(urls: string[]): Promise<void> {
  await Promise.all(
    urls.map(async (url) => {
      try {
        const name = await loadExternalXtraFromUrl(url);
        register_external_xtra_plugin(name);
        console.log(`[dirplayer] External xtra registered: ${name} (${url})`);
      } catch (e) {
        console.warn(`[dirplayer] Failed to load external xtra from ${url}:`, e);
      }
    })
  );
}
