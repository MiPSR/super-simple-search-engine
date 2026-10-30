function colorBrand(text) {
  const el = document.getElementById('brand');
  el.innerHTML = '';
  for (const ch of text) {
    const span = document.createElement('span');
    span.className = 'letter';
    span.textContent = ch;
    const hue = crypto.getRandomValues(new Uint32Array(1))[0] % 360;
    span.style.color = `oklch(0.5 0.05 ${hue})`;
    span.style.webkitTextStroke = `0.5px var(--text-color)`;
    el.appendChild(span);
  }
}
colorBrand('SSSE');

async function load() {
  const res = await fetch('/api/stats');
  const d = await res.json();
  document.getElementById('current').innerHTML = `<pre>${JSON.stringify(d,null,2)}</pre>`;

  const top = await fetch('/api/stats/top').then(r=>r.json());
  const bot = await fetch('/api/stats/bottom').then(r=>r.json());
  let html = '<h2>Top domains</h2><ul>';
  for (const x of top) html += `<li>${x.domain}: ${x.popularity}</li>`;
  html += '</ul><h2>Bottom domains</h2><ul>';
  for (const x of bot) html += `<li>${x.domain}: ${x.popularity}</li>`;
  html += '</ul>';
  document.getElementById('popularity').innerHTML = html;

  const hist = await fetch('/api/stats/history').then(r=>r.json());
  if (hist.length > 0 && window.Chart) {
    const ctx = document.getElementById('chart').getContext('2d');
    new Chart(ctx, {
      type: 'line',
      data: {
        labels: hist.map(h => new Date(h.completed_at*1000).toLocaleDateString()),
        datasets: [
          { label: 'Pages', data: hist.map(h=>h.pages), borderColor: 'oklch(0.6 0.1 120)', hidden: false },
          { label: 'Domains', data: hist.map(h=>h.domains), borderColor: 'oklch(0.6 0.1 240)', hidden: true },
          { label: 'Images', data: hist.map(h=>h.images), borderColor: 'oklch(0.6 0.1 30)', hidden: true },
          { label: 'Videos', data: hist.map(h=>h.videos), borderColor: 'oklch(0.6 0.1 300)', hidden: true },
          { label: 'News', data: hist.map(h=>h.news), borderColor: 'oklch(0.6 0.1 180)', hidden: true },
          { label: 'Refused', data: hist.map(h=>h.refused), borderColor: 'oklch(0.6 0.1 0)', hidden: true },
          { label: 'Duration (ms)', data: hist.map(h=>h.duration_ms), borderColor: 'oklch(0.6 0.1 60)', hidden: true },
        ]
      },
      options: { responsive: true, plugins: { legend: { position: 'top' } } }
    });
  }
}
load();
EOF
echo done