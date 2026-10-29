import { getPref, setPref } from './prefs.js';

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

// theme handling
function applyTheme(t) {
  if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t;
  else delete document.documentElement.dataset.theme;
}
const themeSel = document.getElementById('theme');
themeSel.value = getPref('theme', 'system');
applyTheme(themeSel.value);
themeSel.addEventListener('change', () => { setPref('theme', themeSel.value); applyTheme(themeSel.value); });

// background
const bgFile = document.getElementById('bgFile');
const bgPreview = document.getElementById('bgPreview');
bgFile.addEventListener('change', async (e) => {
  const f = e.target.files[0];
  if (!f) return;
  const buf = await f.arrayBuffer();
  // store in IndexedDB simple: use localStorage dataurl
  const reader = new FileReader();
  reader.onload = () => {
    localStorage.setItem('bg', reader.result);
    document.body.classList.add('has-bg');
    document.body.style.backgroundImage = `url(${reader.result})`;
    bgPreview.innerHTML = `<img src="${reader.result}" style="max-width:200px"><button id="rm">X</button>`;
  };
  reader.readAsDataURL(f);
});
const bg = localStorage.getItem('bg');
if (bg) { document.body.classList.add('has-bg'); document.body.style.backgroundImage = `url(${bg})`; bgPreview.innerHTML = `<img src="${bg}" style="max-width:200px">`; }
document.getElementById('removeBg').addEventListener('click', () => {
  localStorage.removeItem('bg');
  document.body.classList.remove('has-bg');
  document.body.style.backgroundImage = '';
  bgPreview.innerHTML = '';
});

// results per page
const rppDiv = document.getElementById('rpp');
for (const cat of ['PAGES','IMAGES','VIDEOS','NEWS']) {
  const def = (cat==='IMAGES'||cat==='VIDEOS')?18:10;
  const inp = document.createElement('input');
  inp.type = 'number';
  inp.value = getPref('rpp_'+cat, def);
  inp.addEventListener('change', () => setPref('rpp_'+cat, inp.value));
  const lbl = document.createElement('label');
  lbl.textContent = cat + ' ';
  lbl.appendChild(inp);
  rppDiv.appendChild(lbl);
}

// login
document.getElementById('loginForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const username = document.getElementById('username').value;
  const password = document.getElementById('password').value;
  const res = await fetch('/api/admin/login', { method:'POST', headers:{'content-type':'application/json'}, body: JSON.stringify({username,password}) });
  if (res.ok) {
    document.getElementById('loginSection').style.display = 'none';
    document.getElementById('adminSection').style.display = 'block';
    loadSettings();
  } else { alert('login failed'); }
});

async function loadSettings() {
  const res = await fetch('/api/admin/settings');
  const data = await res.json();
  const form = document.getElementById('settingsForm');
  form.innerHTML = '';
  for (const [k,v] of Object.entries(data)) {
    const lbl = document.createElement('label');
    lbl.textContent = k;
    const inp = document.createElement('input');
    inp.value = v;
    inp.name = k;
    lbl.appendChild(inp);
    form.appendChild(lbl);
    form.appendChild(document.createElement('br'));
  }
  const btn = document.createElement('button');
  btn.textContent = 'Save Settings';
  btn.type = 'button';
  btn.addEventListener('click', saveSettings);
  form.appendChild(btn);
  // seeds
  const sr = await fetch('/api/admin/seeds');
  const seeds = await sr.json();
  document.getElementById('seeds').value = seeds.join('\n');
}

async function saveSettings() {
  const form = document.getElementById('settingsForm');
  const inputs = form.querySelectorAll('input');
  const obj = {};
  inputs.forEach(i => { if (i.name) obj[i.name] = isNaN(i.value)? i.value : Number(i.value); });
  await fetch('/api/admin/settings', { method:'PUT', headers:{'content-type':'application/json'}, body: JSON.stringify(obj) });
  alert('saved');
}

document.getElementById('saveSeeds').addEventListener('click', async () => {
  const seeds = document.getElementById('seeds').value.split('\n').map(s=>s.trim()).filter(Boolean);
  await fetch('/api/admin/seeds', { method:'PUT', headers:{'content-type':'application/json'}, body: JSON.stringify({seeds}) });
  alert('saved');
});

async function refreshStatus() {
  const res = await fetch('/api/admin/crawl/status');
  const data = await res.json();
  document.getElementById('crawlStatus').textContent = JSON.stringify(data, null, 2);
}
document.getElementById('crawlStart').addEventListener('click', async () => { await fetch('/api/admin/crawl/start', {method:'POST'}); refreshStatus(); });
document.getElementById('crawlStop').addEventListener('click', async () => { await fetch('/api/admin/crawl/stop', {method:'POST'}); refreshStatus(); });
document.getElementById('crawlRestart').addEventListener('click', async () => { await fetch('/api/admin/crawl/restart', {method:'POST'}); refreshStatus(); });
setInterval(refreshStatus, 3000);
