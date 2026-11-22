import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

function run(command, args) {
  const result = spawnSync(command, args, { stdio: 'inherit' });
  if (result.error) {
    console.error(`Failed to start ${command}: ${result.error.message}`);
    process.exit(1);
  }
  if (result.status !== 0) process.exit(result.status ?? 1);
}

if (process.platform === 'win32') {
  const script = fileURLToPath(new URL('./build-windows.ps1', import.meta.url));
  run('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'BYPASS', '-File', script, '-PrepareOnly']);
}

const tauri = fileURLToPath(new URL('../node_modules/@tauri-apps/cli/tauri.js', import.meta.url));
run(process.execPath, [tauri, 'build', ...process.argv.slice(2)]);
