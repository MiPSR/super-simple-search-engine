import { getPref, setPref, applyTheme, applyBackground, saveBackground, colorBrand, api } from './prefs.js';
colorBrand();
const $ = id => document.getElementById(id);
let loggedIn = false;
function message(text) { $('message').textContent = text; }
function authenticated(value) {
  loggedIn = value; $('loginSection').hidden = value; $('adminSection').hidden = !value;
  if (!value) $('password').value = '';
}
async function action(work) {
  try { await work(); } catch (error) { message(error.message); }
}
$('theme').value = getPref('theme', 'system');
$('theme').addEventListener('change', () => { setPref('theme', $('theme').value); applyTheme(); });
async function showBackground() {
  const url = await applyBackground(); $('bgPreview').replaceChildren();
  if (url) { const image = document.createElement('img'); image.src = url; image.alt = 'Current custom background'; $('bgPreview').append(image); }
}
$('bgFile').addEventListener('change', () => action(async () => {
  const file = $('bgFile').files[0]; if (!file) return;
  if (!file.type.startsWith('image/')) throw new Error('Select an image file.');
  await saveBackground(file); await showBackground(); message('Background saved in this browser.');
}));
$('removeBg').addEventListener('click', () => action(async () => { await saveBackground(null); await showBackground(); $('bgFile').value = ''; }));
showBackground();
for (const category of ['PAGES', 'IMAGES', 'VIDEOS', 'NEWS']) {
  const fallback = category === 'IMAGES' || category === 'VIDEOS' ? 18 : 10;
  const label = document.createElement('label'); label.textContent = `${category} `;
  const input = document.createElement('input'); input.type = 'number'; input.min = 1; input.max = 100; input.step = 1; input.value = getPref(`rpp_${category}`, fallback);
  input.addEventListener('change', () => {
    const n = Number(input.value);
    if (!Number.isInteger(n) || n < 1 || n > 100) { input.value = fallback; message('Results per page must be 1–100.'); }
    setPref(`rpp_${category}`, input.value);
  }); label.append(input); $('rpp').append(label);
}
$('loginForm').addEventListener('submit', e => { e.preventDefault(); action(async () => {
  await api('/api/admin/login', { method: 'POST', body: JSON.stringify({ username: $('username').value, password: $('password').value }) });
  $('password').value = ''; message('Logged in.'); await loadSettings();
}); });
$('logout').addEventListener('click', () => action(async () => { await api('/api/admin/logout', { method: 'POST' }); authenticated(false); message('Logged out.'); }));
const labels = {
  crawl_interval_days: 'Automatic crawl interval (days; 0 = manual only)', worker_count: 'Workers (0 = automatic)',
  worker_delay_ms: 'Worker delay (milliseconds)', domain_delay_ms: 'Domain delay (milliseconds)',
  fifo_refill_ms: 'FIFO refill check (milliseconds)', max_depth: 'Maximum depth per domain (0 = unlimited)',
  max_text_bytes: 'Maximum extracted text (bytes; 0 = unlimited)', max_html_bytes: 'Maximum response body (bytes; 0 = unlimited)'
};
async function loadSettings() {
  const data = await api('/api/admin/settings');
  $('settingsForm').replaceChildren();
  for (const [key, value] of Object.entries(data)) {
    const label = document.createElement('label'); label.textContent = labels[key] || key;
    const input = document.createElement('input'); input.type = 'number'; input.name = key; input.min = 0; input.step = 1; input.required = true; input.value = value;
    label.append(input); $('settingsForm').append(label);
  }
  const button = document.createElement('button'); button.textContent = 'Save Settings'; $('settingsForm').append(button);
  const [seeds, filters] = await Promise.all([api('/api/admin/seeds'), api('/api/admin/filters')]);
  $('seeds').value = seeds.join('\n'); $('bannedDomains').value = filters.banned_domains.join('\n');
  $('bannedWords').value = filters.banned_words.map(w => `${w.mode}:${w.word}`).join('\n'); $('promoted').value = filters.promoted_domains.join('\n');
  authenticated(true);
  await refreshStatus();
}
$('settingsForm').addEventListener('submit', e => { e.preventDefault(); action(async () => {
  const settings = Object.fromEntries([...$('settingsForm').querySelectorAll('input')].map(i => [i.name, Number(i.value)]));
  await api('/api/admin/settings', { method: 'PUT', body: JSON.stringify(settings) }); message('Settings saved; apply to next crawl.');
}); });
function lines(id) { return $(id).value.split('\n').map(s => s.trim()).filter(Boolean); }
for (const [button, path, payload] of [
  ['saveSeeds', 'seeds', () => ({ seeds: lines('seeds') })],
  ['saveDomains', 'banned-domains', () => ({ items: lines('bannedDomains') })],
  ['savePromoted', 'promoted', () => ({ items: lines('promoted') })],
  ['saveWords', 'banned-words', () => ({ items: lines('bannedWords').map(line => {
    const colon = line.indexOf(':'); if (colon < 0) throw new Error('Prefix each term with whole: or substring:');
    return { mode: line.slice(0, colon).trim(), word: line.slice(colon + 1).trim() };
  }) })]
]) $(button).addEventListener('click', () => action(async () => { await api(`/api/admin/${path}`, { method: 'PUT', body: JSON.stringify(payload()) }); message('Saved. Bans and seeds apply to the next crawl.'); }));
$('passwordForm').addEventListener('submit', e => { e.preventDefault(); action(async () => {
  await api('/api/admin/password', { method: 'POST', body: JSON.stringify({ username: $('pwUsername').value, old_password: $('oldPassword').value, new_password: $('newPassword').value }) });
  $('passwordForm').reset(); authenticated(false); message('Password changed. All sessions invalidated.');
}); });
async function refreshStatus() {
  if (!loggedIn) return;
  try { $('crawlStatus').textContent = JSON.stringify(await api('/api/admin/crawl/status'), null, 2); }
  catch (error) { authenticated(false); message(error.message); }
}
for (const [id, command] of [['crawlStart', 'start'], ['crawlStop', 'stop'], ['crawlRestart', 'restart']]) {
  $(id).addEventListener('click', () => action(async () => {
    $(id).disabled = true;
    try { await api(`/api/admin/crawl/${command}`, { method: 'POST' }); await refreshStatus(); message(`Crawl ${command} completed.`); }
    finally { $(id).disabled = false; }
  }));
}
setInterval(refreshStatus, 3000);
loadSettings().catch(() => authenticated(false));
