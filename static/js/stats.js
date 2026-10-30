import { colorBrand } from './prefs.js';
colorBrand('SSSE');

async function load() {
  const res = await fetch('/api/stats');
  const d = await res.json();
  const summary = document.createElement('pre');
  summary.textContent = JSON.stringify(d, null, 2);
  document.getElementById('current').replaceChildren(summary);

  const top = await fetch('/api/stats/top').then(r=>r.json());
  const bot = await fetch('/api/stats/bottom').then(r=>r.json());
  const popularity = document.getElementById('popularity');
  for (const [title, rows] of [['Top domains', top], ['Bottom domains', bot]]) {
    const h = document.createElement('h2'); h.textContent = title;
    const ul = document.createElement('ul');
    for (const row of rows) { const li = document.createElement('li'); li.textContent = `${row.domain}: ${row.popularity}`; ul.append(li); }
    popularity.append(h, ul);
  }

  const hist = await fetch('/api/stats/history').then(r=>r.json());
  const table = document.getElementById('history');
  for (const row of hist) {
    const tr = document.createElement('tr');
    for (const value of [new Date(row.completed_at * 1000).toLocaleString(), row.pages, row.domains, row.images, row.videos, row.news, row.refused, row.processed, row.duration_ms]) {
      const td = document.createElement('td'); td.textContent = value; tr.append(td);
    }
    table.append(tr);
  }
  if (hist.length > 0 && window.Chart) {
    const ctx = document.getElementById('chart').getContext('2d');
    const chart = new Chart(ctx, {
      type: 'line',
      data: {
        labels: hist.map(h => h.completed_at * 1000),
        datasets: [
          { label: 'Pages', data: hist.map(h=>h.pages), borderColor: '#448844', hidden: false },
          { label: 'Domains', data: hist.map(h=>h.domains), borderColor: '#5577bb', hidden: true },
          { label: 'Images', data: hist.map(h=>h.images), borderColor: '#bb6633', hidden: true },
          { label: 'Videos', data: hist.map(h=>h.videos), borderColor: '#9955aa', hidden: true },
          { label: 'News', data: hist.map(h=>h.news), borderColor: '#338888', hidden: true },
          { label: 'Refused', data: hist.map(h=>h.refused), borderColor: '#aa4455', hidden: true },
          { label: 'Duration (ms)', data: hist.map(h=>h.duration_ms), borderColor: '#997722', hidden: true, yAxisID: 'duration' },
        ]
      },
      options: {
        animation: !matchMedia('(prefers-reduced-motion: reduce)').matches,
        responsive: true,
        scales: {
          x: { type: 'linear', title: { display: true, text: 'Successful crawl completion' }, ticks: { callback: value => new Date(value).toLocaleDateString() } },
          y: { title: { display: true, text: 'Count' }, beginAtZero: true },
          duration: { type: 'linear', position: 'right', title: { display: true, text: 'Duration (ms)' }, grid: { drawOnChartArea: false } }
        },
        plugins: { legend: { display: false }, tooltip: { callbacks: { title: items => new Date(items[0].parsed.x).toLocaleString() } } }
      }
    });
    chart.data.datasets.forEach((series, index) => {
      const label = document.createElement('label');
      const input = document.createElement('input'); input.type = 'checkbox'; input.checked = !series.hidden;
      input.addEventListener('change', () => { chart.setDatasetVisibility(index, input.checked); chart.update(); });
      label.append(input, document.createTextNode(` ${series.label}`)); document.getElementById('series').append(label);
    });
  }
}
load().catch(error => { document.getElementById('current').textContent = `Unable to load statistics: ${error.message}`; });
