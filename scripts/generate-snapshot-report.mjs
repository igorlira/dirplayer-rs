#!/usr/bin/env node
/**
 * Generate a multi-page HTML snapshot comparison report.
 * Produces an index page listing all test suites and one page per suite.
 * Usage: node scripts/generate-snapshot-report.mjs [snapshots-base] [output-dir]
 * Defaults: vm-rust/tests/snapshots   test-results/snapshot-report
 */

import * as fs from 'fs';
import * as path from 'path';
import { fileURLToPath } from 'url';
import { PNG } from 'pngjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const TEMPLATE_DIR = path.join(__dirname, 'snapshot-report');

const [,, snapshotsBase = 'vm-rust/tests/snapshots', outputDir = 'test-results/snapshot-report'] = process.argv;

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

function esc(str) {
  return String(str)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;')
    .replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

const snapshots = walkPngs(outDir)
  .map(fp => {
    const rel = path.relative(outDir, fp);
    const diffPath = path.join(diffDir, rel);
    return {
      rel,
      parts: rel.split(path.sep),
      out:  copyToReport(fp, 'out', rel),
      ref:  copyToReport(path.join(refDir, rel), 'ref', rel),
      diff: copyToReport(diffPath, 'diff', rel),
      diffRatio: computeDiffRatio(diffPath),
    };
  })
  .sort((a, b) => a.rel.localeCompare(b.rel));

// Group by suite/platform/test (first 3 path components)
const groups = new Map();
for (const s of snapshots) {
  const key = s.parts.slice(0, 3).join('/');
  if (!groups.has(key)) groups.set(key, []);
  groups.get(key).push(s);
}

// Sort groups by changed count desc, then name asc
const sortedGroups = [...groups.entries()].map(([key, snaps]) => {
  const changedCount = snaps.filter(s => s.diff !== null && s.ref !== null).length;
  return { key, snaps, changedCount };
}).sort((a, b) => b.changedCount - a.changedCount || a.key.localeCompare(b.key));

const css      = fs.readFileSync(path.join(TEMPLATE_DIR, 'report.css'), 'utf8');
const js       = fs.readFileSync(path.join(TEMPLATE_DIR, 'report.js'), 'utf8');
const template = fs.readFileSync(path.join(TEMPLATE_DIR, 'template.html'), 'utf8');

const filterToggle = `<label class="filter-toggle">
    <input type="checkbox" id="diffs-only">
    <span>Show only changed</span>
  </label>`;

function renderCard(s, imgPrefix = '') {
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
        <img class="img-ref" src="${esc(imgPrefix + s.ref)}" alt="Reference">
        <img class="img-out" src="${esc(imgPrefix + s.out)}" alt="Output">
        <div class="compare-line"></div>
        <span class="lbl-ref">Ref</span>
        <span class="lbl-out">Out</span>
        <input type="range" class="compare-range" min="0" max="100" value="50"
               aria-label="Slide to compare reference and output">
      </div>`;
    const diffView = changed
      ? `<div class="view-diff" hidden>
          <img class="img-diff" src="${esc(imgPrefix + s.diff)}" alt="Diff">
        </div>`
      : '';
    body = `<div class="view-compare">${compareView}</div>${diffView}`;
  } else if (s.out) {
    body = `<div class="single-img"><img src="${esc(imgPrefix + s.out)}" alt="Output"></div>`;
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

function buildPage({ title, nav, filter, passCount, diffCount, totalCount, body }) {
  return template
    .replace('{{TITLE}}',       title)
    .replace('{{NAV}}',         nav)
    .replace('{{CSS}}',         css)
    .replace('{{JS}}',          js)
    .replace('{{FILTER}}',      filter)
    .replace('{{PASS_COUNT}}',  String(passCount))
    .replace('{{DIFF_COUNT}}',  String(diffCount))
    .replace('{{TOTAL_COUNT}}', String(totalCount))
    .replace('{{BODY}}',        body);
}

fs.mkdirSync(path.join(outputDir, 'suites'), { recursive: true });

// ── Per-suite pages ──
for (const { key, snaps, changedCount } of sortedGroups) {
  const slug = key.replace(/\//g, '--');
  const [suite, platform, test] = key.split('/');
  const imgPrefix = '../';

  const changedSnaps = snaps
    .filter(s => s.diff !== null && s.ref !== null)
    .sort((a, b) => (b.diffRatio ?? 0) - (a.diffRatio ?? 0));
  const okSnaps = snaps
    .filter(s => s.diff === null || s.ref === null)
    .sort((a, b) => a.rel.localeCompare(b.rel));

  let body = '';
  if (changedSnaps.length > 0) {
    body += `<section class="snap-section">
  <h2 class="section-heading section-changed">Changed (${changedSnaps.length})</h2>
  <div class="cards-grid">
    ${changedSnaps.map(s => renderCard(s, imgPrefix)).join('\n    ')}
  </div>
</section>\n`;
  }
  if (okSnaps.length > 0) {
    body += `<section class="snap-section snap-section-ok">
  <h2 class="section-heading">OK (${okSnaps.length})</h2>
  <div class="cards-grid">
    ${okSnaps.map(s => renderCard(s, imgPrefix)).join('\n    ')}
  </div>
</section>`;
  }

  const totalCount = snaps.length;
  const html = buildPage({
    title: `${esc(suite)} / ${esc(platform)} / ${esc(test)}`,
    nav: `<a href="../index.html" class="back-link">← All suites</a>`,
    filter: filterToggle,
    passCount: totalCount - changedCount,
    diffCount: changedCount,
    totalCount,
    body,
  });

  fs.writeFileSync(path.join(outputDir, 'suites', `${slug}.html`), html);
}

// ── Index page ──
const totalSnapshots = snapshots.length;
const totalChanged   = snapshots.filter(s => s.diff !== null && s.ref !== null).length;
const changedSuites  = sortedGroups.filter(g => g.changedCount > 0).length;

const indexRows = sortedGroups.map(({ key, snaps, changedCount }) => {
  const slug = key.replace(/\//g, '--');
  const [suite, platform, test] = key.split('/');
  const totalCount = snaps.length;
  const changed = changedCount > 0;
  const statsHtml = changed
    ? `<span class="stat-changed">${changedCount} changed</span><span class="suite-total"> / ${totalCount} total</span>`
    : `<span class="suite-total">${totalCount} total</span>`;
  return `<a href="suites/${slug}.html" class="suite-row${changed ? ' suite-changed' : ''}">
  <span class="suite-crumbs">
    <span class="crumb">${esc(suite)}</span><span class="sep">/</span><span class="crumb">${esc(platform)}</span><span class="sep">/</span><span class="crumb crumb-test">${esc(test)}</span>
  </span>
  <span class="suite-stats">${statsHtml}</span>
  <span class="badge ${changed ? 'badge-fail' : 'badge-pass'}">${changed ? 'CHANGED' : 'OK'}</span>
</a>`;
}).join('\n');

const indexHtml = buildPage({
  title: 'Snapshot Report',
  nav: '',
  filter: filterToggle,
  // index summary counts suites, not individual snapshots
  passCount: sortedGroups.length - changedSuites,
  diffCount: changedSuites,
  totalCount: sortedGroups.length,
  body: `<div class="suite-list">\n${indexRows}\n</div>`,
});

fs.writeFileSync(path.join(outputDir, 'index.html'), indexHtml);

console.log(`Report written to ${outputDir} (${sortedGroups.length} suites, ${changedSuites} changed, ${totalSnapshots} snapshots)`);
