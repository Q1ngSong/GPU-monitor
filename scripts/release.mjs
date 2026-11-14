import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const env = { ...process.env };
// Cargo splits RUSTFLAGS on whitespace; the encoded form also supports spaces in paths.
const flags = env.CARGO_ENCODED_RUSTFLAGS !== undefined
  ? env.CARGO_ENCODED_RUSTFLAGS.split('\x1f').filter(Boolean)
  : (env.RUSTFLAGS ?? '').split(/\s+/).filter(Boolean);
const prefixes = [
  [homedir(), '/home'],
  [path.resolve(env.CARGO_HOME || path.join(homedir(), '.cargo')), '/cargo'],
  [root, '/workspace'],
];
for (const [source, destination] of prefixes) {
  for (const spelling of new Set([source, source.replaceAll('\\', '/')])) {
    flags.push(`--remap-path-prefix=${spelling}=${destination}`);
  }
}
env.CARGO_ENCODED_RUSTFLAGS = flags.join('\x1f');
delete env.RUSTFLAGS;

const args = process.argv.slice(2);
const cliOnly = args[0] === '--cli';
if (cliOnly) args.shift();
const result = spawnSync(cliOnly ? 'cargo' : process.execPath, cliOnly
  ? ['build', '--release', '--locked', '--manifest-path', 'src-tauri/Cargo.toml', '--bin', 'gpu-monitor-cli', ...args]
  : [path.join(root, 'node_modules/@tauri-apps/cli/tauri.js'), 'build', ...args], {
  cwd: root, env, stdio: 'inherit', windowsHide: true,
});
if (result.error) console.error(result.error.message);
process.exitCode = result.status ?? 1;
