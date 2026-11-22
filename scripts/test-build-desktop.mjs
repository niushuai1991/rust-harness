import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, copyFileSync, readFileSync, writeFileSync, rmSync, existsSync, realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

function runFixture({ prepareExit = 0, tauriExit = 0 } = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'desktop-build-test-'));
  try {
    mkdirSync(join(dir, 'scripts'), { recursive: true });
    mkdirSync(join(dir, 'node_modules', '@tauri-apps', 'cli'), { recursive: true });
    copyFileSync(join(root, 'package.json'), join(dir, 'package.json'));
    const runner = join(root, 'scripts', 'build-desktop.mjs');
    if (existsSync(runner)) copyFileSync(runner, join(dir, 'scripts', 'build-desktop.mjs'));
    writeFileSync(join(dir, 'scripts', 'build-windows.ps1'), `param([switch]$PrepareOnly)\nif (-not $PrepareOnly) { exit 41 }\nSet-Content -LiteralPath $env:PREPARE_TRACE -Value 'prepared'\nexit ${prepareExit}\n`);
    writeFileSync(join(dir, 'node_modules', '@tauri-apps', 'cli', 'tauri.js'), `require('fs').writeFileSync(process.env.TAURI_TRACE, JSON.stringify(process.argv.slice(2))); process.exit(${tauriExit});`);
    const prepareTrace = join(dir, 'prepared.txt');
    const tauriTrace = join(dir, 'tauri.json');
    const result = spawnSync(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['run', 'desktop:build', '--', '--debug'], {
      cwd: dir, encoding: 'utf8', shell: process.platform === 'win32',
      env: { ...process.env, PREPARE_TRACE: prepareTrace, TAURI_TRACE: tauriTrace },
    });
    return { status: result.status, stdout: result.stdout, stderr: result.stderr,
      prepared: existsSync(prepareTrace), tauriArgs: existsSync(tauriTrace) ? JSON.parse(readFileSync(tauriTrace, 'utf8')) : null };
  } finally {
    const realDir = realpathSync(dir);
    if (dirname(realDir) !== realpathSync(tmpdir()) || !basename(realDir).startsWith('desktop-build-test-')) throw new Error('Unexpected fixture path');
    rmSync(realDir, { recursive: true, force: true });
  }
}

test('desktop build prepares Windows assets only on Windows and forwards Tauri arguments', () => {
  const result = runFixture();
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  assert.equal(result.prepared, process.platform === 'win32');
  assert.deepEqual(result.tauriArgs, ['build', '--debug']);
});

test('Tauri failure is returned to npm', () => {
  const result = runFixture({ tauriExit: 23 });
  assert.equal(result.status, 23, `${result.stdout}\n${result.stderr}`);
});

test('Windows preparation failure stops the build', { skip: process.platform !== 'win32' }, () => {
  const result = runFixture({ prepareExit: 17 });
  assert.equal(result.status, 17, `${result.stdout}\n${result.stderr}`);
  assert.equal(result.tauriArgs, null);
});
