// Optional real-browser smoke test: nix develop --command node tests/browser.mjs
// Uses the installed Chrome/Chromium and Node's built-in CDP WebSocket client.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { resolve } from 'node:path';
import net from 'node:net';

const binary = resolve(process.env.SSSE_TEST_BINARY || 'target/debug/ssse');
const directory = await mkdtemp('/tmp/opencode/ssse-browser-');
const socket = `${directory}/ssse.sock`;
const portServer = net.createServer();
await new Promise(r => portServer.listen(0, '127.0.0.1', r));
const port = portServer.address().port;
await new Promise(r => portServer.close(r));
const origin = `http://127.0.0.1:${port}`;
const server = spawn(binary, ['serve', '--data-dir', directory, '--bind', `127.0.0.1:${port}`], {
  cwd: directory, env: { ...process.env, SSSE_INSECURE_COOKIE: '1' }, stdio: ['ignore', 'pipe', 'pipe']
});
let serverLog = '';
server.stdout.on('data', b => serverLog += b); server.stderr.on('data', b => serverLog += b);
let chrome, client;
async function until(check, label) {
  const deadline = Date.now() + 15000;
  while (Date.now() < deadline) { if (await check()) return; await new Promise(r => setTimeout(r, 50)); }
  throw new Error(`Timed out: ${label}\n${serverLog}`);
}
async function commandSocket(request) {
  return new Promise((resolve, reject) => {
    const connection = net.createConnection(socket); let body = '';
    connection.on('connect', () => connection.end(JSON.stringify(request) + '\n'));
    connection.on('data', b => body += b); connection.on('error', reject);
    connection.on('end', () => resolve(JSON.parse(body)));
  });
}
try {
  await until(async () => { try { return (await fetch(`${origin}/api/stats`)).ok; } catch { return false; } }, 'server startup');
  assert.equal((await commandSocket({ command: 'password_change', username: 'admin', password: 'browser-secret' })).ok, true);
  const duplicate = spawn(binary, ['serve', '--data-dir', directory, '--bind', '127.0.0.1:0'], { stdio: 'ignore' });
  assert.notEqual(await new Promise(r => duplicate.once('exit', r)), 0, 'duplicate instance must not touch active crawl state');
  const cli = JSON.parse(execFileSync(binary, ['crawl', 'status', '--socket', socket], { encoding: 'utf8' }));
  assert.equal(cli.running, false);
  execFileSync('sqlite3', [`${directory}/ssse.db`, `
    INSERT INTO pages(url,domain,title,text,crawled_at) VALUES('https://example.org/community','example.org','Community news','hello community',1);
    INSERT INTO images(url,domain,title,alt) VALUES('https://example.org/photo.png','example.org','Community photo','community');
    INSERT INTO images_fts(images_fts) VALUES('rebuild');
    INSERT INTO domains VALUES('example.org',2);
    INSERT INTO crawl_history(completed_at,duration_ms,domains,pages,images,videos,news,refused,processed) VALUES(1793350000,1200,1,1,1,0,0,0,1);
  `]);
  chrome = spawn(process.env.CHROME || 'google-chrome', [
    '--headless', '--no-sandbox', '--disable-dev-shm-usage', '--disable-background-networking',
    '--remote-debugging-port=0', `--user-data-dir=${directory}/chrome`, 'about:blank'
  ], { stdio: ['ignore', 'ignore', 'pipe'] });
  let endpoint = '', chromeLog = '';
  chrome.stderr.on('data', b => { chromeLog += b; endpoint = chromeLog.match(/DevTools listening on (ws:\/\/\S+)/)?.[1] || ''; });
  chrome.on('error', error => { chromeLog += error.message; });
  await until(() => endpoint, 'Chrome DevTools startup');
  const debugOrigin = endpoint.replace(/^ws:/, 'http:').split('/devtools/')[0];
  const page = await fetch(`${debugOrigin}/json/new?${origin}`, { method: 'PUT' }).then(r => r.json());
  client = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { client.addEventListener('open', resolve, { once: true }); client.addEventListener('error', reject, { once: true }); });
  let id = 0; const callbacks = new Map(); const exceptions = [];
  client.addEventListener('message', event => {
    const message = JSON.parse(event.data);
    if (message.method === 'Runtime.exceptionThrown') exceptions.push(message.params.exceptionDetails);
    if (message.id) { const callback = callbacks.get(message.id); callbacks.delete(message.id); message.error ? callback.reject(new Error(JSON.stringify(message.error))) : callback.resolve(message.result); }
  });
  const send = (method, params = {}) => new Promise((resolve, reject) => { const call = ++id; callbacks.set(call, { resolve, reject }); client.send(JSON.stringify({ id: call, method, params })); });
  const evaluate = async expression => {
    const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  const waitJS = (expression, label) => until(async () => { try { return await evaluate(expression); } catch { return false; } }, label);
  const navigate = async path => { await send('Page.navigate', { url: origin + path }); await waitJS(`location.pathname === ${JSON.stringify(path.split('?')[0])} && document.readyState === 'complete'`, path); };
  await send('Runtime.enable'); await send('Network.enable');
  await send('Network.setBlockedURLs', { urls: ['https://*'] });
  await navigate('/');
  await waitJS(`document.getElementById('brand')?.textContent === 'SSSE'`, 'home JavaScript');
  await evaluate(`document.getElementById('homeQ').value='community';document.getElementById('homeForm').requestSubmit()`);
  await waitJS(`document.querySelectorAll('#resultList .card').length === 1`, 'page search');
  assert.equal(await evaluate(`document.getElementById('q').value`), 'community');
  await send('Page.reload');
  await waitJS(`document.querySelectorAll('#resultList .card').length === 1 && !document.getElementById('results').hidden`, 'search survives reload');
  await evaluate(`document.querySelector('[data-cat="IMAGES"]').click()`);
  await waitJS(`document.querySelector('#resultList .image-preview') !== null`, 'image category');
  await evaluate(`document.querySelector('.image-preview').click()`);
  assert.equal(await evaluate(`document.querySelector('dialog').open`), true);
  await evaluate(`document.querySelector('dialog button').click()`);
  await waitJS(`document.querySelector('dialog') === null`, 'preview close');
  await navigate('/settings.html');
  await waitJS(`document.querySelectorAll('#rpp input').length === 4`, 'public browser preferences');
  assert.equal(await evaluate(`document.getElementById('adminSection').hidden`), true);
  await evaluate(`document.getElementById('theme').value='dark';document.getElementById('theme').dispatchEvent(new Event('change'))`);
  await evaluate(`const dt=new DataTransfer();dt.items.add(new File([Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jp1sAAAAASUVORK5CYII='),c=>c.charCodeAt(0))],'test.png',{type:'image/png'}));document.getElementById('bgFile').files=dt.files;document.getElementById('bgFile').dispatchEvent(new Event('change'))`);
  await waitJS(`document.querySelector('#bgPreview img')?.src.startsWith('blob:')`, 'IndexedDB background');
  await navigate('/');
  await waitJS(`document.documentElement.dataset.theme === 'dark' && document.body.classList.contains('has-bg')`, 'preferences across pages');
  await navigate('/settings.html');
  await evaluate(`document.getElementById('password').value='browser-secret';document.getElementById('loginForm').requestSubmit()`);
  await waitJS(`!document.getElementById('adminSection').hidden && document.querySelectorAll('#settingsForm input').length === 8`, 'admin login');
  await evaluate(`document.querySelector('[name="max_depth"]').value='0';document.getElementById('settingsForm').requestSubmit()`);
  await waitJS(`document.getElementById('message').textContent.includes('Settings saved')`, 'settings save');
  assert.equal((await evaluate(`fetch('/api/admin/settings').then(r=>r.json())`)).max_depth, 0);
  await evaluate(`document.getElementById('removeBg').click()`);
  await waitJS(`!document.body.classList.contains('has-bg')`, 'background removal');
  await evaluate(`document.getElementById('logout').click()`);
  await waitJS(`document.getElementById('adminSection').hidden`, 'logout');
  await navigate('/stats.html');
  await waitJS(`document.querySelectorAll('#history tr').length === 1 && typeof Chart !== 'undefined' && Object.keys(Chart.instances).length === 1`, 'statistics and graph');
  assert.equal(await evaluate(`document.querySelectorAll('#series input[type="checkbox"]').length`), 7);
  await evaluate(`document.querySelectorAll('#series input')[6].click()`);
  assert.equal(await evaluate(`Object.values(Chart.instances)[0].isDatasetVisible(6)`), true);
  await send('Emulation.setDeviceMetricsOverride', { width: 360, height: 800, deviceScaleFactor: 1, mobile: true });
  for (const path of ['/', '/?q=community', '/settings.html', '/stats.html']) {
    await navigate(path);
    await new Promise(r => setTimeout(r, 150));
    assert.equal(await evaluate(`document.documentElement.scrollWidth <= innerWidth`), true, `mobile overflow at ${path}`);
  }
  assert.deepEqual(exceptions, []);
  console.log('Browser smoke test passed: CLI socket, search/reload/categories, image preview, public preferences/IndexedDB, login/settings/logout, statistics graph.');
} finally {
  if (client) client.close();
  if (chrome && chrome.exitCode === null) { chrome.kill(); await new Promise(r => chrome.once('exit', r)); }
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  await rm(directory, { recursive: true, force: true });
}
