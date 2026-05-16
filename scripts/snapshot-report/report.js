document.addEventListener('DOMContentLoaded', () => {
  // Wire up compare sliders
  document.querySelectorAll('.compare-imgs[data-compare]').forEach(imgs => {
    const range  = imgs.querySelector('.compare-range');
    const imgOut = imgs.querySelector('.img-out');
    const line   = imgs.querySelector('.compare-line');

    function update(v) {
      const pct = Number(v);
      imgOut.style.clipPath = `inset(0 ${100 - pct}% 0 0)`;
      line.style.left = `${pct}%`;
    }

    range.addEventListener('input', e => update(e.target.value));
    update(range.value);
  });

  // View mode tabs (Compare / Diff)
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

  // "Show only changed" filter
  const toggle = document.getElementById('diffs-only');
  toggle.addEventListener('change', () => {
    document.querySelectorAll('.group').forEach(g => {
      if (toggle.checked) {
        g.classList.toggle('hide', !g.classList.contains('group-changed'));
      } else {
        g.classList.remove('hide');
      }
    });
  });
});
