/** Native WebView2 acceptance. Disposable Windows Sandbox only; no downloaded browser. */
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdir, readFile, writeFile, access } from 'node:fs/promises';
import { resolve, join, dirname, isAbsolute } from 'node:path';
import { spawn } from 'node:child_process';
import { once } from 'node:events';

// Check before touching paths, opening ports or spawning anything.
if (process.env.P1_SANDBOX_GUEST !== '1' || process.env.USERNAME?.toLowerCase() !== 'wdagutilityaccount' || process.platform !== 'win32') {
  console.error('Refusing host execution: requires Windows Sandbox WDAGUtilityAccount and P1_SANDBOX_GUEST=1.');
  process.exit(1);
}
const args = Object.fromEntries(Array.from({ length: (process.argv.length - 2) / 2 }, (_, i) => [process.argv[2 + i * 2], process.argv[3 + i * 2]]));
for (const name of ['--exe', '--results', '--data-dir']) assert.ok(args[name] && isAbsolute(args[name]), `Required absolute path: ${name}`);
assert.equal(Object.keys(args).length, 3, 'Unknown runner arguments');
const exe = resolve(args['--exe']), results = resolve(args['--results']), dataDir = resolve(args['--data-dir']);
assert.notEqual(dataDir.toLowerCase(), dirname(exe).toLowerCase(), 'Data must be outside application install directory');
const source = join(dirname(dataDir), 'P1 合成原项目'), oldText = 'P1 original old\n', newText = 'P1 replica new\n';
const report = { status: 'running', startedAt: new Date().toISOString(), environment: { user: process.env.USERNAME, computer: process.env.COMPUTERNAME, node: process.version, executable: exe, dataDir }, checks: [], requests: [], screenshots: [], notTested: ['真实付费供应商 API', '原生图片理解', '真实标准用户 UAC'], error: null };
let server, app, cdp, workspace, session, failure;
const children = new Set();
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
async function saveReport() { await writeFile(join(results, 'desktop-result.json'), JSON.stringify(report, null, 2), 'utf8'); }
async function check(name, condition, evidence) {
  report.checks.push({ name, passed: !!condition, evidence });
  await saveReport();
  assert.ok(condition, `Acceptance failed: ${name}`);
}
async function until(fn, description, timeout = 45000) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    if (failure) throw failure;
    try { const result = await fn(); if (result) return result; } catch (error) { last = error; }
    await pause(150);
  }
  throw new Error(`Timed out: ${description}${last ? ` (${last.message})` : ''}`);
}
function launch(command, arguments_, options = {}) {
  const child = spawn(command, arguments_, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'], ...options });
  child._p1StdErr = '';
  child.stderr?.on('data', chunk => { child._p1StdErr = (child._p1StdErr + String(chunk)).slice(-16384); });
  children.add(child); child.on('error', error => { failure ??= error; });
  return child;
}
async function childExit(child, timeout = 15000) {
  if (child.exitCode !== null || child.signalCode !== null) return child.exitCode;
  return Promise.race([once(child, 'exit').then(([code]) => code), pause(timeout).then(() => { throw new Error(`Owned PID ${child.pid} did not exit`); })]);
}
async function terminateOwned(child) {
  if (!child || !children.has(child) || child.exitCode !== null || child.signalCode !== null) return;
  // Never enumerate names or kill unrelated application instances.
  const cleanup = spawn('taskkill.exe', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' });
  await once(cleanup, 'exit');
  await childExit(child).catch(() => {});
}
async function powershell(script) {
  const encoded = Buffer.from(script, 'utf16le').toString('base64');
  const child = spawn('powershell.exe', ['-NoProfile', '-NonInteractive', '-EncodedCommand', encoded], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  children.add(child);
  let stdout = '', stderr = '';
  child.stdout.on('data', chunk => { stdout += chunk; }); child.stderr.on('data', chunk => { stderr += chunk; });
  const code = await childExit(child);
  assert.equal(code, 0, `Guest Win32 check failed: ${stderr}`);
  return stdout.trim();
}
class Cdp {
  constructor(socket) {
    this.socket = socket; this.sequence = 0; this.pending = new Map();
    socket.addEventListener('message', event => {
      const message = JSON.parse(String(event.data));
      if (message.id && this.pending.has(message.id)) {
        const { resolve, reject, timer } = this.pending.get(message.id); clearTimeout(timer); this.pending.delete(message.id);
        if (message.error) reject(new Error(JSON.stringify(message.error))); else resolve(message.result);
      }
    });
    socket.addEventListener('close', () => { for (const { reject, timer } of this.pending.values()) { clearTimeout(timer); reject(new Error('WebView2 CDP disconnected')); } this.pending.clear(); });
  }
  static async connect(url) {
    assert.equal(typeof WebSocket, 'function', 'Node must provide native WebSocket (Node 22+)');
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }); });
    return new Cdp(socket);
  }
  call(method, params = {}) {
    return new Promise((resolve, reject) => {
      const id = ++this.sequence, timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 45000);
      this.pending.set(id, { resolve, reject, timer }); this.socket.send(JSON.stringify({ id, method, params }));
    });
  }
  async evaluate(expression) {
    const result = await this.call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
    return result.result.value;
  }
  close() { this.socket.close(); }
}
let debugPort;
async function attach() {
  const target = await until(async () => {
    report.cdp = { debugPort, appPid: app.pid, appExitCode: app.exitCode, stderr: app._p1StdErr };
    if (app.exitCode !== null) throw new Error(`Guest application exited: ${app.exitCode}`);
    try {
      const pages = await fetch(`http://127.0.0.1:${debugPort}/json/list`).then(r => r.json());
      report.cdp.pages = pages.map(p => ({ type: p.type, url: p.url, hasDebugger: !!p.webSocketDebuggerUrl }));
      await saveReport();
      return pages.find(p => p.type === 'page' && p.webSocketDebuggerUrl && /(?:tauri|localhost)/i.test(p.url));
    } catch (error) {
      report.cdp.probeError = String(error);
      await saveReport();
      return null;
    }
  }, 'owned WebView2 target');
  cdp = await Cdp.connect(target.webSocketDebuggerUrl);
  await cdp.call('Runtime.enable'); await cdp.call('Page.enable');
  await until(() => cdp.evaluate('typeof window.__TAURI_INTERNALS__?.invoke === "function" && !!document.querySelector("textarea")'), 'Tauri application UI');
}
const invoke = (name, body = {}) => cdp.evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(name)},${JSON.stringify(body)})`);
async function click(selector, text) {
  const box = await until(() => cdp.evaluate(`(() => {
    const elements=Array.from(document.querySelectorAll(${JSON.stringify(selector)}));
    const el=elements.find(e=>${text === undefined ? 'true' : `(e.getAttribute('aria-label')||e.textContent||'').trim()===${JSON.stringify(text)}`} && !e.disabled && e.getBoundingClientRect().width && e.getBoundingClientRect().height);
    if(!el)return null;el.scrollIntoView({block:'center'});const r=el.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};
  })()`), `visible enabled ${selector} ${text ?? ''}`);
  await cdp.call('Input.dispatchMouseEvent', { type: 'mousePressed', button: 'left', clickCount: 1, ...box });
  await cdp.call('Input.dispatchMouseEvent', { type: 'mouseReleased', button: 'left', clickCount: 1, ...box });
}
async function screenshot(name) {
  const image = await cdp.call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
  await writeFile(join(results, name), Buffer.from(image.data, 'base64')); report.screenshots.push(name); await saveReport();
}
async function send(text) {
  await until(() => cdp.evaluate('!!document.querySelector("textarea:not(:disabled)")'), 'ready composer');
  await click('textarea'); await cdp.call('Input.insertText', { text }); await click('button', '发送消息');
}
async function approve(tool, allow) {
  const pending = await until(async () => (await invoke('approval_list')).find(a => a.tool === tool), `pending ${tool} approval`);
  await until(() => cdp.evaluate(`Array.from(document.querySelectorAll('[role="dialog"] strong')).some(e=>e.textContent===${JSON.stringify(tool)})`), `${tool} approval dialog`);
  await screenshot(`approval-${tool}-${allow ? 'allow' : 'deny'}.png`);
  await click('[role="dialog"] button', allow ? '批准此次操作' : '拒绝');
  await until(async () => !(await invoke('approval_list')).some(a => a.id === pending.id), `${tool} UI decision settled`);
  return pending;
}
function answer(res, calls = [], content = '') {
  const delta = { content, tool_calls: calls.map((c, index) => ({ index, id: c.id, type: 'function', function: { name: c.name, arguments: JSON.stringify(c.arguments) } })) };
  res.writeHead(200, { 'Content-Type': 'text/event-stream; charset=utf-8', 'Cache-Control': 'no-cache' });
  res.end(`data: ${JSON.stringify({ choices: [{ index: 0, delta, finish_reason: calls.length ? 'tool_calls' : 'stop' }] })}\n\ndata: [DONE]\n\n`);
}
const toolMessage = (body, id) => body.messages.find(m => m.role === 'tool' && m.tool_call_id === id)?.content;
try {
  await mkdir(results, { recursive: true }); await access(exe);
  // A unique disposable data directory is mandatory; do not reuse another run's database.
  await mkdir(dirname(dataDir), { recursive: true });
  await mkdir(dataDir, { recursive: false }); await mkdir(source, { recursive: false }); await mkdir(join(source, '.skills', 'p1'), { recursive: true });
  await writeFile(join(source, 'greet.txt'), oldText, 'utf8');
  await writeFile(join(source, 'AGENTS.md'), 'P1_AGENT_CONTEXT_MARKER: read the p1 skill before changing greet.txt.\n', 'utf8');
  await writeFile(join(source, '.skills', 'p1', 'SKILL.md'), 'P1_SKILL_BODY_MARKER: preserve original until reviewed synchronization.\n', 'utf8');
  await writeFile(join(source, 'verify.ps1'), "$ErrorActionPreference='Stop'\nif ((Get-Content -LiteralPath (Join-Path $PSScriptRoot 'greet.txt') -Raw) -cne \"P1 replica new`n\") { throw 'unexpected greet content' }\nWrite-Output 'P1_VERIFY_OK'\n", 'utf8');
  let requestNumber = 0;
  server = createServer(async (req, res) => {
    try {
      assert.equal(req.method, 'POST'); assert.equal(req.url, '/chat/completions');
      let raw = ''; for await (const chunk of req) { raw += chunk; assert.ok(raw.length < 2 * 1024 * 1024); }
      const body = JSON.parse(raw), step = requestNumber++;
      report.requests.push({ step, path: req.url, body }); await saveReport();
      assert.equal(body.stream, true); assert.equal(req.headers.authorization, undefined, 'fixture must not need a credential');
      if (step === 0) {
        assert.ok(body.messages.some(m => m.role === 'system' && m.content.includes('P1_AGENT_CONTEXT_MARKER') && m.content.includes('SKILL.md')));
        answer(res, [{ id: 'p1-read', name: 'read', arguments: { path: 'greet.txt' } }, { id: 'p1-skill', name: 'read', arguments: { path: '.skills/p1/SKILL.md' } }]);
      } else if (step === 1) {
        assert.equal(toolMessage(body, 'p1-read'), oldText); assert.ok(toolMessage(body, 'p1-skill')?.includes('P1_SKILL_BODY_MARKER'));
        answer(res, [{ id: 'p1-write-denied', name: 'write', arguments: { path: 'greet.txt', content: newText } }]);
      } else if (step === 2) {
        assert.ok(toolMessage(body, 'p1-write-denied')?.includes('APPROVAL_DENIED'));
        assert.equal(await readFile(join(source, 'greet.txt'), 'utf8'), oldText); assert.equal(await readFile(join(workspace.replicaPath, 'greet.txt'), 'utf8'), oldText);
        await check('denied-write-has-no-side-effect', true, { source: oldText, replica: oldText });
        answer(res, [{ id: 'p1-write-approved', name: 'write', arguments: { path: 'greet.txt', content: newText } }]);
      } else if (step === 3) {
        assert.ok(toolMessage(body, 'p1-write-approved')?.includes('greet.txt'));
        assert.equal(await readFile(join(source, 'greet.txt'), 'utf8'), oldText); assert.equal(await readFile(join(workspace.replicaPath, 'greet.txt'), 'utf8'), newText);
        answer(res, [{ id: 'p1-shell', name: 'shell', arguments: { command: '& ./verify.ps1' } }]);
      } else if (step === 4) {
        assert.ok(toolMessage(body, 'p1-shell')?.includes('P1_VERIFY_OK'));
        answer(res, [], 'P1_DONE：副本验证通过，请审阅并同步。');
      } else if (step === 5) {
        assert.ok(toolMessage(body, 'p1-shell')?.includes('P1_VERIFY_OK')); assert.ok(toolMessage(body, 'p1-write-denied')?.includes('APPROVAL_DENIED'));
        assert.ok(body.messages.some(m => m.role === 'user' && m.content.includes('P1_CONTINUE')));
        answer(res, [], 'P1_RESTART_OK：已恢复历史，继续当前会话。');
      } else throw new Error(`Unexpected model request ${step}`);
    } catch (error) { failure = error; if (!res.headersSent) res.writeHead(500); res.end('fixture assertion failed'); }
  });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  const baseUrl = `http://127.0.0.1:${server.address().port}`;
  const reservation = createServer(); reservation.listen(0, '127.0.0.1'); await once(reservation, 'listening'); debugPort = reservation.address().port; await new Promise(resolve => reservation.close(resolve));
  const startApp = () => launch(exe, [], { env: { ...process.env, P1_SANDBOX_GUEST: '1', RUST_HARNESS_DATA_DIR: dataDir, RUST_HARNESS_P1_CDP_PORT: String(debugPort) } });
  app = startApp(); await attach();
  await invoke('provider_save', { config: { id: 'p1-cdp-fixture', name: 'P1 本地验收', builtin: false, compatibility: 'standard', defaultModel: 'p1-fixture', credentialRef: '', proxy: null, endpoints: [{ apiMode: 'chatCompletions', baseUrl }] }, secret: null });
  await invoke('model_save', { config: { id: 'p1-cdp-model', name: 'P1 本地验收模型', providerId: 'p1-cdp-fixture', compatibility: 'standard', apiMode: 'chatCompletions', baseUrl, model: 'p1-fixture', credentialRef: '', proxy: null, maxOutputTokens: 1024 }, secret: null });
  workspace = await invoke('workspace_open', { path: source });
  session = await invoke('session_create', { workspaceId: workspace.id, title: 'P1 原生验收会话' });
  await cdp.call('Page.reload', { ignoreCache: true });
  await until(() => cdp.evaluate('document.body.innerText.includes("P1 原生验收会话") && !!document.querySelector("textarea:not(:disabled)")'), 'seeded UI refresh');
  await send('P1_START：读取指令及技能，修改 greet.txt，验证副本。');
  await approve('write', false); await approve('write', true); await approve('shell', true);
  await until(() => cdp.evaluate('document.body.innerText.includes("P1_DONE") && !document.querySelector("button[aria-label=取消生成]")'), 'completed real desktop turn');
  await check('instructions-and-skill-model-visible', requestNumber === 5, { requests: requestNumber });
  await check('replica-only-before-sync', await readFile(join(source, 'greet.txt'), 'utf8') === oldText && await readFile(join(workspace.replicaPath, 'greet.txt'), 'utf8') === newText, { source, replica: workspace.replicaPath });
  const history = await invoke('session_events', { sessionId: session.id });
  await check('shell-settled-successfully', history.some(e => e.kind === 'tool.result' && e.data.callId === 'p1-shell' && !e.data.isError && e.data.content.includes('P1_VERIFY_OK')), { tool: 'p1-shell' });
  await screenshot('before-sync.png'); await click('button', '查看更改');
  await until(() => cdp.evaluate('document.querySelector("[role=dialog]")?.innerText.includes("greet.txt")'), 'reviewed diff');
  await screenshot('reviewed-diff.png'); await click('[role=dialog] button', '同步选中文件'); await click('[role=dialog] button', '确认应用更改');
  await until(async () => await readFile(join(source, 'greet.txt'), 'utf8') === newText, 'source changed after explicit UI sync');
  await check('reviewed-ui-sync-updates-source', true, { content: newText }); await click('[role=dialog] button', '关闭面板'); await screenshot('after-sync.png');
  // Native close-to-tray uses only the exact application PID started above.
  const windowHandle = await powershell(`$p=[Diagnostics.Process]::GetProcessById(${app.pid});$p.MainWindowHandle.ToInt64()`);
  assert.match(windowHandle, /^\d+$/); assert.notEqual(windowHandle, '0', 'Owned native window must exist');
  const native = 'Add-Type -TypeDefinition \'using System;using System.Runtime.InteropServices;public class P1Win{[DllImport("user32.dll")]public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);[DllImport("user32.dll")]public static extern bool IsWindowVisible(IntPtr h);}\';';
  await powershell(`${native}[P1Win]::PostMessage([IntPtr]${windowHandle},0x10,[IntPtr]::Zero,[IntPtr]::Zero)|Out-Null`);
  await until(async () => (await powershell(`${native}[P1Win]::IsWindowVisible([IntPtr]${windowHandle})`)) === 'False', 'close hides owned window');
  await check('close-keeps-app-alive-in-tray', app.exitCode === null, { pid: app.pid });
  const second = startApp(); const secondCode = await childExit(second);
  await until(async () => (await powershell(`${native}[P1Win]::IsWindowVisible([IntPtr]${windowHandle})`)) === 'True', 'second instance raises original native window');
  await check('single-instance-raises-tray-window', secondCode === 0 && app.exitCode === null, { primaryPid: app.pid, secondPid: second.pid, secondCode });
  cdp.close(); cdp = undefined; await terminateOwned(app);
  await until(async () => { try { await fetch(`http://127.0.0.1:${debugPort}/json/list`, { signal: AbortSignal.timeout(1000) }); return false; } catch { return true; } }, 'old owned WebView2 exits');
  app = startApp(); await attach();
  await until(() => cdp.evaluate('document.body.innerText.includes("P1_DONE") && !!document.querySelector("textarea:not(:disabled)")'), 'restarted persisted history');
  await send('P1_CONTINUE：重启后继续同一个会话。');
  await until(() => cdp.evaluate('document.body.innerText.includes("P1_RESTART_OK") && !document.querySelector("button[aria-label=取消生成]")'), 'restarted conversation completes');
  await check('restart-continues-persisted-history', requestNumber === 6 && (await invoke('session_list', { workspaceId: workspace.id })).some(s => s.id === session.id), { sessionId: session.id, requestCount: requestNumber });
  await screenshot('restart-continued.png'); report.status = 'passed';
} catch (error) {
  report.status = 'failed'; report.error = error.stack ?? String(error);
  if (cdp) { try { await screenshot('failure.png'); } catch {} }
  console.error(report.error); process.exitCode = 1;
} finally {
  cdp?.close();
  for (const child of children) { try { await terminateOwned(child); } catch (error) { report.status = 'failed'; report.error ??= String(error); process.exitCode = 1; } }
  if (server) { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
  report.finishedAt = new Date().toISOString();
  try { await saveReport(); } catch (error) { console.error(error); process.exitCode = 1; }
}
