/**
 * Fixture bodies for extension CORS e2e.
 *
 * fast.dcr / slow.dcr are synthetic bytes (Port protocol only).
 * shapes-*.dcr is the corpus movie `D8_5_00001_shapes_1.dcr` — a real Director
 * file we already play in the VM suite, served from a remote-looking host so
 * load_movie_from_file → fetch_net_task goes through the SW.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

export const PORT = Number(process.env.EXTENSION_E2E_PORT || 9323);

export const PLAYER_HOST = "player.dirplayer.test";
export const CDN_HOST = "cdn.dirplayer.test";

export const PLAYER_ORIGIN = `http://${PLAYER_HOST}:${PORT}`;
export const CDN_ORIGIN = `http://${CDN_HOST}:${PORT}`;

export const FAST_PATH = "/fast.dcr";
export const SLOW_PATH = "/slow.dcr";

export const SHAPES_FILE_NAME = "D8_5_00001_shapes_1.dcr";
export const SHAPES_RELATIVE = `dcr_dirplayer_test_movies/${SHAPES_FILE_NAME}`;
export const SHAPES_FILE = path.join(REPO_ROOT, "public", ...SHAPES_RELATIVE.split("/"));
export const SHAPES_FAST_PATH = "/shapes-fast.dcr";
export const SHAPES_SLOW_PATH = "/shapes-slow.dcr";
export const EMBED_FAST_PATH = "/embed-fast.html";
export const EMBED_SLOW_PATH = "/embed-slow.html";

export const SLOW_CHUNK_COUNT = 4;
export const SLOW_CHUNK_DELAY_MS = 200;

export const FAST_PAYLOAD = Buffer.from(`DIRPLAYER-E2E-FAST\n${"x".repeat(4096)}`);

export const SLOW_PAYLOAD = (() => {
  const buf = Buffer.alloc(64 * 1024);
  for (let i = 0; i < buf.length; i++) buf[i] = (i * 13) & 0xff;
  return buf;
})();

export const SHAPES_PAYLOAD = fs.existsSync(SHAPES_FILE)
  ? fs.readFileSync(SHAPES_FILE)
  : null;

export const HOST_RESOLVER_RULES =
  `MAP ${PLAYER_HOST} 127.0.0.1,MAP ${CDN_HOST} 127.0.0.1`;
