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
  document.getElementById('popularity').innerHTML = '<h2>Top</h2><pre>'+JSON.stringify(top,null,2)+'</pre><h2>Bottom</h2><pre>'+JSON.stringify(bot,null,2)+'</pre>';
  const hist = await fetch('/api/stats/history').then(r=>r.json());
  // simple numeric table
  const c = document.getElementById('chart');
  const ctx = c.getContext('2d');
  ctx.fillStyle = getComputedStyle(document.body).getPropertyValue('--text-color');
  ctx.fillText('History points: ' + hist.length, 10, 20);
}
load();
