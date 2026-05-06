// Copy public/ruffle/ → <destDir>/ruffle/ for a build target.
// Used by postbuild / postbuild-extension / postbuild-polyfill.
//
// If public/ruffle/ doesn't exist, print a warning and exit cleanly
// instead of failing the build — the user just hasn't run
// `npm run build-ruffle` yet, and skipping the copy is more useful
// than aborting the entire build.

import { existsSync, mkdirSync, readdirSync, rmSync, statSync, copyFileSync } from 'fs';
import { join } from 'path';

const SRC = 'public/ruffle';
const destArg = process.argv[2];
if (!destArg) {
    console.error('copy-ruffle.mjs: missing destination argument (e.g. build, dist-extension, dist-polyfill)');
    process.exit(1);
}
const DEST = `${destArg}/ruffle`;

if (!existsSync(SRC)) {
    console.warn(
        `copy-ruffle.mjs: ${SRC}/ does not exist — Flash content will not work in ${destArg}/. ` +
        `Run \`npm run build-ruffle\` first if you need it.`
    );
    process.exit(0);
}

function copyDir(src, dest) {
    mkdirSync(dest, { recursive: true });
    for (const f of readdirSync(src)) {
        const sp = join(src, f);
        const dp = join(dest, f);
        if (statSync(sp).isDirectory()) {
            copyDir(sp, dp);
        } else {
            copyFileSync(sp, dp);
        }
    }
}

if (existsSync(DEST)) {
    rmSync(DEST, { recursive: true });
}
copyDir(SRC, DEST);
console.log(`Ruffle cleaned and copied to ${DEST}/`);
