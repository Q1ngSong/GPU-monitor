import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Tauri merges the main executable only; its additional Cargo binaries need the same step.
if (process.env.TAURI_ENV_TARGET_TRIPLE === 'universal-apple-darwin') {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
  const metadata = spawnSync('cargo', [
    'metadata', '--no-deps', '--format-version', '1',
    '--manifest-path', 'src-tauri/Cargo.toml',
  ], { cwd: root, encoding: 'utf8' });
  if (metadata.error) throw metadata.error;
  if (metadata.status !== 0) throw new Error(metadata.stderr);
  const target = JSON.parse(metadata.stdout).target_directory;
  const profile = process.env.TAURI_ENV_DEBUG === 'true' ? 'debug' : 'release';
  const binary = (arch) => path.join(target, `${arch}-apple-darwin`, profile, 'gpu-monitor-cli');
  const result = spawnSync('lipo', [
    '-create', binary('aarch64'), binary('x86_64'), '-output', binary('universal'),
  ], { cwd: root, stdio: 'inherit' });
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
}
