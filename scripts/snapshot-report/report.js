document.addEventListener('DOMContentLoaded', () => {
  // Wire up compare sliders (suite pages)
  document.querySelectorAll('.compare-imgs[data-compare]').forEach(imgs => {
    const range  = imgs.querySelector('.compare-range');
    const imgRef = imgs.querySelector('.img-ref');
    const line   = imgs.querySelector('.compare-line');

    function update(v) {
      const pct = Number(v);
      imgRef.style.clipPath = `inset(0 ${100 - pct}% 0 0)`;
      line.style.left = `${pct}%`;
    }

    range.addEventListener('input', e => update(e.target.value));
    update(range.value);
  });

  // View mode tabs (suite pages)
  document.querySelectorAll('.view-tabs').forEach(tabs => {
    tabs.addEventListener('click', e => {
      const btn = e.target.closest('.view-tab');
      if (!btn) return;
      const card = btn.closest('.card');
      const view = btn.dataset.view;
      tabs.querySelectorAll('.view-tab').forEach(t => t.classList.toggle('active', t === btn));
      card.querySelector('.view-compare').hidden = view !== 'compare';
      card.querySelector('.view-diff').hidden    = view !== 'diff';
    });
  });

  // "Show only changed" filter — works on both index and suite pages
  const toggle = document.getElementById('diffs-only');
  if (!toggle) return;
  toggle.addEventListener('change', () => {
    // Suite pages: hide/show the OK section
    document.querySelectorAll('.snap-section-ok').forEach(s => {
      s.style.display = toggle.checked ? 'none' : '';
    });
    // Index pages: hide/show OK suite rows
    document.querySelectorAll('.suite-row:not(.suite-changed)').forEach(r => {
      r.style.display = toggle.checked ? 'none' : '';
    });
  });
});
