#!/usr/bin/env node
/**
 * Generate a self-contained HTML snapshot comparison report.
 * Usage: node scripts/generate-snapshot-report.mjs [snapshots-base] [output-dir]
 * Defaults: vm-rust/tests/snapshots   /tmp/diff-report
 */

import * as fs from 'fs';
import * as path from 'path';
import { fileURLToPath } from 'url';
import { PNG } from 'pngjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const TEMPLATE_DIR = path.join(__dirname, 'snapshot-report');

const [,, snapshotsBase = 'vm-rust/tests/snapshots', outputDir = '/tmp/diff-report'] = process.argv;

const outDir  = path.join(snapshotsBase, 'output');
const refDir  = path.join(snapshotsBase, 'reference');
const diffDir = path.join(snapshotsBase, 'diff');

function copyToReport(srcPath, subdir, rel) {
  const dest = path.join(outputDir, 'images', subdir, rel);
  try {
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.copyFileSync(srcPath, dest);
    return `images/${subdir}/${rel.split(path.sep).join('/')}`;
  } catch { return null; }
}

// Count red pixels ([255,0,0,255]) in a diff PNG — see compareSnapshots in e2e.spec.ts.
// Dimmed unchanged pixels can be at most [63,*,*,*] after the >>2 shift, so R===255 is unambiguous.
function computeDiffRatio(diffPath) {
  try {
    const png = PNG.sync.read(fs.readFileSync(diffPath));
    const total = png.width * png.height;
    let changed = 0;
    for (let i = 0; i < total * 4; i += 4) {
      if (png.data[i] === 255 && png.data[i + 1] === 0 && png.data[i + 2] === 0) changed++;
    }
    return changed / total;
  } catch { return null; }
}

function walkPngs(dir) {
  const result = [];
  function recurse(d) {
    if (!fs.existsSync(d)) return;
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const full = path.join(d, e.name);
      if (e.isDirectory()) recurse(full);
      else if (e.name.endsWith('.png')) result.push(full);
    }
  }
  recurse(dir);
  return result;
}

const snapshots = walkPngs(outDir)
  .map(fp => {
    const rel = path.relative(outDir, fp);
    const diffPath = path.join(diffDir, rel);
    return {
      rel,
      parts: rel.split(path.sep),
      out: copyToReport(fp, 'out', rel),
      ref: copyToReport(path.join(refDir, rel), 'ref', rel),
      diff: copyToReport(diffPath, 'diff', rel),
      diffRatio: computeDiffRatio(diffPath),
    };
  })
  .sort((a, b) => a.rel.localeCompare(b.rel));

const diffCount = snapshots.filter(s => s.diff !== null && s.ref !== null).length;
const passCount = snapshots.length - diffCount;

// Group by suite/platform/test (first 3 path components)
const groups = new Map();
for (const s of snapshots) {
  const key = s.parts.slice(0, 3).join('/');
  if (!groups.has(key)) groups.set(key, []);
  groups.get(key).push(s);
}

function esc(str) {
  return String(str)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;')
    .replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function renderCard(s) {
  const name = s.parts[s.parts.length - 1].replace(/\.png$/, '');
  // 'changed' requires both diff and ref — a diff without a ref is a stale artifact
  const changed = s.diff !== null && s.ref !== null;
  const isNew   = s.ref === null;
  const id = `card-${s.rel.replace(/[^a-z0-9]/gi, '-')}`;

  const badgeClass = changed ? 'badge-fail' : isNew ? 'badge-new' : 'badge-pass';
  const badgeText  = changed ? 'CHANGED'   : isNew ? 'NEW'       : 'OK';
  const pctHtml    = changed && s.diffRatio !== null
    ? `<span class="diff-pct">${(s.diffRatio * 100).toFixed(2)}%</span>`
    : '';
  const tabsHtml   = changed
    ? `<div class="view-tabs">
        <button class="view-tab active" data-view="compare">Compare</button>
        <button class="view-tab" data-view="diff">Diff</button>
      </div>`
    : '';

  let body;
  if (s.out && s.ref) {
    const compareView = `
      <div class="compare-imgs" data-compare>
        <img class="img-ref" src="${esc(s.ref)}" alt="Reference">
        <img class="img-out" src="${esc(s.out)}" alt="Output">
        <div class="compare-line"></div>
        <span class="lbl-ref">Ref</span>
        <span class="lbl-out">Out</span>
        <input type="range" class="compare-range" min="0" max="100" value="50"
               aria-label="Slide to compare reference and output">
      </div>`;
    const diffView = changed
      ? `<div class="view-diff" hidden>
          <img class="img-diff" src="${esc(s.diff)}" alt="Diff">
        </div>`
      : '';
    body = `<div class="view-compare">${compareView}</div>${diffView}`;
  } else if (s.out) {
    body = `<div class="single-img"><img src="${esc(s.out)}" alt="Output"></div>`;
  } else {
    body = `<div class="missing-img">No output</div>`;
  }

  return `<div class="card${changed ? ' card-changed' : ''}" id="${esc(id)}-wrap">
  <div class="card-header">
    <span class="card-name">${esc(name)}</span>
    ${pctHtml}
    ${tabsHtml}
    <span class="badge ${badgeClass}">${badgeText}</span>
  </div>
  ${body}
</div>`;
}

function renderGroup([key, snaps]) {
  const changed = snaps.some(s => s.diff !== null && s.ref !== null);
  const [suite, platform, test] = key.split('/');
  const anchorId = esc(key.replace(/\//g, '--'));

  return `<section class="group${changed ? ' group-changed' : ''}" id="${anchorId}">
  <div class="group-header">
    <h2 class="group-title">
      <a href="#${anchorId}" class="anchor">#</a>
      <span class="crumb">${esc(suite)}</span><span class="sep">/</span><span class="crumb">${esc(platform)}</span><span class="sep">/</span><span class="crumb crumb-test">${esc(test)}</span>
    </h2>
    <span class="badge ${changed ? 'badge-fail' : 'badge-pass'}">${changed ? 'CHANGED' : 'OK'}</span>
  </div>
  <div class="cards-grid">
    ${snaps.map(renderCard).join('\n    ')}
  </div>
</section>`;
}

const css      = fs.readFileSync(path.join(TEMPLATE_DIR, 'report.css'), 'utf8');
const js       = fs.readFileSync(path.join(TEMPLATE_DIR, 'report.js'), 'utf8');
const template = fs.readFileSync(path.join(TEMPLATE_DIR, 'template.html'), 'utf8');

const html = template
  .replace('{{CSS}}',         css)
  .replace('{{JS}}',          js)
  .replace('{{PASS_COUNT}}',  String(passCount))
  .replace('{{DIFF_COUNT}}',  String(diffCount))
  .replace('{{TOTAL_COUNT}}', String(snapshots.length))
  .replace('{{BODY}}',        [...groups.entries()].map(renderGroup).join('\n'));

fs.mkdirSync(outputDir, { recursive: true });
const outPath = path.join(outputDir, 'report.html');
fs.writeFileSync(outPath, html);
const kb = Math.round(fs.statSync(outPath).size / 1024);
console.log(`Report written to ${outPath} (${kb} KB, ${snapshots.length} snapshots, ${diffCount} changed)`);
