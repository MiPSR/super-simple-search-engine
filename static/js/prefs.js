const defaults = { PAGES: 10, IMAGES: 18, VIDEOS: 18, NEWS: 10 };
export function getPref(key, fallback) {
  try {
    const value = localStorage.getItem(key);
    if (value === null) return fallback;
    if (key.startsWith('rpp_')) {
      const n = Number(value);
      return Number.isInteger(n) && n >= 1 && n <= 100 ? n : fallback;
    }
    if (key === 'theme') return ['system', 'light', 'dark'].includes(value) ? value : fallback;
    return value;
  } catch { return fallback; }
}
export function setPref(key, value) {
  try { localStorage.setItem(key, String(value)); } catch { /* Private storage may be unavailable. */ }
}
export function resultsPerPage(category) { return getPref('rpp_' + category, defaults[category]); }
export function applyTheme() {
  const theme = getPref('theme', 'system');
  if (theme === 'system') delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme;
}
export function colorBrand(text = 'SSSE', id = 'brand') {
  const el = document.getElementById(id);
  if (!el) return;
  el.replaceChildren();
  for (const ch of text) {
    const span = document.createElement('span');
    span.className = 'letter';
    span.textContent = ch;
    span.style.color = `oklch(0.5 0.05 ${crypto.getRandomValues(new Uint32Array(1))[0] % 360})`;
    el.append(span);
  }
}
export async function api(path, options = {}) {
  const response = await fetch(path, { ...options, headers: { 'content-type': 'application/json', ...options.headers } });
  const data = await response.json();
  if (!response.ok) throw new Error(data.error || `Request failed (${response.status})`);
  return data;
}
function backgroundStore(mode, action) {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open('ssse-preferences', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('images');
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const db = request.result;
      const tx = db.transaction('images', mode);
      const result = action(tx.objectStore('images'));
      tx.oncomplete = () => { db.close(); resolve(result.result); };
      tx.onerror = () => { db.close(); reject(tx.error); };
    };
  });
}
export function saveBackground(file) { return backgroundStore('readwrite', s => file ? s.put(file, 'background') : s.delete('background')); }
let backgroundURL;
export async function applyBackground() {
  try {
    const file = await backgroundStore('readonly', s => s.get('background'));
    if (backgroundURL) URL.revokeObjectURL(backgroundURL);
    backgroundURL = file ? URL.createObjectURL(file) : null;
    document.body.classList.toggle('has-bg', !!file);
    document.body.style.backgroundImage = file ? `url("${backgroundURL}")` : '';
    return backgroundURL;
  } catch { return null; }
}
applyTheme();
applyBackground();
