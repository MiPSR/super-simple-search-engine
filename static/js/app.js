import { colorBrand, resultsPerPage } from './prefs.js';
colorBrand();
colorBrand('SEARCH', 'searchHeading');
let pending;
const homeForm = document.getElementById('homeForm');
const searchForm = document.getElementById('searchForm');
const queryInput = document.getElementById('q');
const status = document.getElementById('searchStatus');
const list = document.getElementById('resultList');
const categories = ['PAGES', 'IMAGES', 'VIDEOS', 'NEWS'];
function state() {
  const p = new URLSearchParams(location.search);
  return { query: p.get('q') || '', category: categories.includes(p.get('category')) ? p.get('category') : 'PAGES', page: Math.max(1, Number(p.get('page')) || 1) };
}
function navigate(changes) {
  const next = { ...state(), ...changes };
  const params = new URLSearchParams({ q: next.query, category: next.category, page: next.page });
  history.pushState(null, '', `/?${params}`);
  render();
}
homeForm.addEventListener('submit', e => { e.preventDefault(); navigate({ query: document.getElementById('homeQ').value, page: 1 }); });
searchForm.addEventListener('submit', e => { e.preventDefault(); navigate({ query: queryInput.value, page: 1 }); });
document.querySelectorAll('.tabs button').forEach(button => button.addEventListener('click', () => navigate({ category: button.dataset.cat, page: 1 })));
window.addEventListener('popstate', render);
function safeURL(value) {
  try { const url = new URL(value); return url.protocol === 'https:' && !url.username && !url.password ? url : null; } catch { return null; }
}
function image(src, alt, className = '') {
  const img = document.createElement('img');
  img.src = src; img.alt = alt; img.loading = 'lazy'; img.referrerPolicy = 'no-referrer'; img.className = className;
  img.addEventListener('error', () => img.hidden = true);
  return img;
}
function preview(url, label) {
  const dialog = document.createElement('dialog');
  dialog.setAttribute('aria-label', label || 'Image preview');
  const close = document.createElement('button'); close.textContent = 'Close preview';
  close.addEventListener('click', () => dialog.close());
  const original = document.createElement('a'); original.href = url; original.textContent = 'Open original image'; original.target = '_blank'; original.rel = 'noopener noreferrer';
  dialog.append(close, image(url, label), original);
  dialog.addEventListener('close', () => dialog.remove());
  document.body.append(dialog); dialog.showModal();
}
async function render() {
  if (pending) pending.abort();
  const controller = new AbortController(); pending = controller;
  const { query, category, page } = state();
  const isSearch = new URLSearchParams(location.search).has('q');
  document.getElementById('home').hidden = isSearch;
  document.getElementById('results').hidden = !isSearch;
  searchForm.hidden = !isSearch;
  queryInput.value = query;
  document.title = isSearch ? `${query || 'Search'} — SSSE` : 'Super Simple Search Engine';
  if (!isSearch) return;
  document.querySelectorAll('.tabs button').forEach(button => {
    const active = button.dataset.cat === category;
    button.classList.toggle('active', active); button.setAttribute('aria-pressed', String(active));
  });
  list.replaceChildren(); document.getElementById('pagination').replaceChildren();
  status.textContent = 'Searching…'; list.setAttribute('aria-busy', 'true');
  try {
    const params = new URLSearchParams({ q: query, category, page, per_page: resultsPerPage(category) });
    const response = await fetch(`/api/search?${params}`, { signal: controller.signal });
    const data = await response.json();
    if (!response.ok) throw new Error(data.error || 'Search failed');
    if (controller.signal.aborted) return;
    list.className = category === 'IMAGES' || category === 'VIDEOS' ? 'grid' : '';
    for (const result of data.results) {
      const url = safeURL(result.url); if (!url) continue;
      const card = document.createElement('article'); card.className = 'card';
      if (category === 'IMAGES' || category === 'VIDEOS') {
        const source = category === 'IMAGES' ? url.href : safeURL(result.extra?.thumbnail)?.href;
        if (source) {
          const thumb = image(source, result.description || result.title || 'Preview');
          if (category === 'IMAGES') {
            const button = document.createElement('button'); button.className = 'image-preview'; button.setAttribute('aria-label', `Preview ${result.title || 'image'}`);
            button.append(thumb); button.addEventListener('click', () => preview(url.href, thumb.alt)); card.append(button);
          } else card.append(thumb);
        }
      }
      const heading = document.createElement('h2');
      const link = document.createElement('a'); link.href = url.href; link.textContent = result.title || url.href; link.target = '_blank'; link.rel = 'noopener noreferrer';
      heading.append(image(`${url.origin}/favicon.ico`, '', 'favicon'), link); card.append(heading);
      const address = document.createElement('div'); address.className = 'result-url'; address.textContent = url.href; card.append(address);
      if (category === 'IMAGES' && result.description) { const alt = document.createElement('p'); alt.textContent = result.description; card.append(alt); }
      if (result.published_at) { const date = document.createElement('time'); date.dateTime = new Date(result.published_at * 1000).toISOString(); date.textContent = new Date(result.published_at * 1000).toLocaleDateString(); card.append(date); }
      list.append(card);
    }
    status.textContent = data.total ? `${data.total.toLocaleString()} results · Page ${page}` : query.trim() ? 'No matching results.' : 'Enter a search query.';
    const totalPages = Math.ceil(data.total / data.per_page);
    const pagination = document.getElementById('pagination');
    const pages = new Set([1, page - 2, page - 1, page, page + 1, page + 2, totalPages]);
    if (page > 1) addPage('Previous', page - 1);
    let previous = 0;
    for (const i of [...pages].filter(i => i > 0 && i <= totalPages).sort((a, b) => a - b)) {
      if (previous && i - previous > 1) pagination.append(document.createTextNode(' … '));
      addPage(String(i), i); previous = i;
    }
    if (page < totalPages) addPage('Next', page + 1);
    function addPage(label, target) {
      const button = document.createElement('button'); button.textContent = label;
      if (target === page) { button.disabled = true; button.setAttribute('aria-current', 'page'); }
      button.addEventListener('click', () => { navigate({ page: target }); queryInput.focus(); }); pagination.append(button);
    }
  } catch (error) { if (error.name !== 'AbortError') status.textContent = error.message; }
  finally { if (!controller.signal.aborted) list.removeAttribute('aria-busy'); }
}
render();
