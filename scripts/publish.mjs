import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const version = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(version ?? '')) throw new Error('Usage: node scripts/publish.mjs X.Y.Z');
const tag = `v${version}`;
function run(command, args, capture = false) {
  const result = spawnSync(command, args, {
    cwd: root, encoding: 'utf8', stdio: capture ? 'pipe' : 'inherit',
    shell: process.platform === 'win32' && command === 'npm',
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed: ${result.stderr ?? result.status}`);
  return result.stdout?.trim() ?? '';
}
const git = (...args) => run('git', args, true);
if (git('branch', '--show-current') !== 'main') throw new Error('Publish from main');
if (git('status', '--porcelain')) throw new Error('Commit your changes before publishing');
run('git', ['fetch', 'origin', 'main', '--tags']);
if (git('rev-parse', 'HEAD') !== git('rev-parse', 'origin/main')) throw new Error('Local and remote main differ');
if (git('tag', '--list', tag)) throw new Error(`${tag} already exists`);
const current = JSON.parse(readFileSync(path.join(root, 'package.json'), 'utf8')).version;
const difference = version.split('.').map((part, index) => Number(part) - Number(current.split('.')[index])).find(n => n !== 0) ?? 0;
// An unpublished initial version can be released without artificially bumping its number.
if (difference < 0 || (difference === 0 && git('tag', '--list', 'v*'))) {
  throw new Error('Choose a newer version (the initial release may keep the existing version)');
}
run('npm', ['run', 'build']);
run('npm', ['test']);
run('cargo', ['fmt', '--manifest-path', 'src-tauri/Cargo.toml', '--', '--check']);
run('cargo', ['test', '--locked', '--manifest-path', 'src-tauri/Cargo.toml']);
if (version !== current) {
  for (const file of ['package.json', 'package-lock.json', 'src-tauri/tauri.conf.json']) {
    const filename = path.join(root, file);
    const data = JSON.parse(readFileSync(filename, 'utf8'));
    if (data.version !== current) throw new Error(`Version mismatch: ${file}`);
    data.version = version;
    if (file === 'package-lock.json') data.packages[''].version = version;
    writeFileSync(filename, `${JSON.stringify(data, null, 2)}\n`);
  }
  for (const [file, pattern] of [
    ['src-tauri/Cargo.toml', /(^version = ")[^"]+("$)/m],
    ['src-tauri/Cargo.lock', /(name = "gpu-monitor"\nversion = ")[^"]+("\n)/],
  ]) {
    const filename = path.join(root, file);
    const text = readFileSync(filename, 'utf8');
    if (!pattern.test(text)) throw new Error(`Missing version in ${file}`);
    writeFileSync(filename, text.replace(pattern, (_match, start, end) => `${start}${version}${end}`));
  }
}
run('git', ['add', 'package.json', 'package-lock.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/tauri.conf.json']);
run('git', ['commit', '--allow-empty', '-m', `Release ${tag}`]);
run('git', ['tag', '-a', tag, '-m', `GPU Monitor ${tag}`]);
run('git', ['push', '--atomic', 'origin', 'main', tag]);
console.log(`Published tag ${tag}; GitHub Actions will build, verify, and publish the installers.`);
