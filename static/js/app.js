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

function randomHeading() {
  const h = document.getElementById('searchHeading');
  if (!h) return;
  h.innerHTML = '';
  for (const ch of 'SEARCH') {
    const span = document.createElement('span');
    span.className = 'letter';
    span.textContent = ch;
    const hue = crypto.getRandomValues(new Uint32Array(1))[0] % 360;
    span.style.color = `oklch(0.5 0.05 ${hue})`;
    span.style.webkitTextStroke = `0.5px var(--text-color)`;
    h.appendChild(span);
  }
}
randomHeading();

let currentCategory = 'PAGES';
let currentPage = 1;
let currentQuery = '';

const homeForm = document.getElementById('homeForm');
const searchForm = document.getElementById('searchForm');
const qInput = document.getElementById('q');
const homeQ = document.getElementById('homeQ');

function doSearch(q) {
  currentQuery = q;
  document.getElementById('home').style.display = 'none';
  document.getElementById('results').style.display = 'block';
  searchForm.style.display = 'flex';
  currentPage = 1;
  loadResults();
}

homeForm.addEventListener('submit', (e) => { e.preventDefault(); doSearch(homeQ.value); });
searchForm.addEventListener('submit', (e) => { e.preventDefault(); doSearch(qInput.value); });

document.querySelectorAll('.tabs button').forEach(btn => {
  btn.addEventListener('click', () => {
    currentCategory = btn.dataset.cat;
    document.querySelectorAll('.tabs button').forEach(b => b.classList.toggle('active', b===btn));
    currentPage = 1;
    loadResults();
  });
});
document.querySelector('.tabs button').classList.add('active');

async function loadResults() {
  const perPage = getPref('rpp_' + currentCategory, defaultRpp(currentCategory));
  const url = `/api/search?q=${encodeURIComponent(currentQuery)}&category=${currentCategory}&page=${currentPage}&per_page=${perPage}`;
  const res = await fetch(url);
  const data = await res.json();
  const list = document.getElementById('resultList');
  list.innerHTML = '';
  if (currentCategory === 'IMAGES' || currentCategory === 'VIDEOS') {
    list.className = 'grid';
  } else {
    list.className = '';
  }
  for (const r of data.results) {
    const div = document.createElement('div');
    div.className = 'card';
    if (currentCategory === 'IMAGES' || currentCategory === 'VIDEOS') {
      const img = document.createElement('img');
      img.src = currentCategory === 'IMAGES' ? r.url : (r.extra?.thumbnail || '');
      img.alt = r.description || r.title || '';
      img.onerror = () => { img.style.display='none'; };
      div.appendChild(img);
    }
    const title = document.createElement('div');
    title.textContent = r.title || r.url;
    div.appendChild(title);
    const urlEl = document.createElement('a');
    urlEl.href = r.url;
    urlEl.textContent = r.url;
    urlEl.target = '_blank';
    div.appendChild(urlEl);
    if (r.published_at) {
      const d = document.createElement('div');
      d.textContent = new Date(r.published_at*1000).toLocaleDateString();
      div.appendChild(d);
    }
    list.appendChild(div);
  }
  // pagination
  const pag = document.getElementById('pagination');
  pag.innerHTML = '';
  const totalPages = Math.ceil(data.total / data.per_page) || 1;
  for (let i=1; i<=Math.min(totalPages,20); i++) {
    const b = document.createElement('button');
    b.textContent = i;
    if (i === currentPage) b.disabled = true;
    b.addEventListener('click', () => { currentPage = i; loadResults(); });
    pag.appendChild(b);
  }
}

function defaultRpp(cat) {
  if (cat === 'IMAGES' || cat === 'VIDEOS') return 18;
  return 10;
}
