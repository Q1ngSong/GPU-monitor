import { copyFileSync, mkdirSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const version = JSON.parse(readFileSync(path.join(root, 'package.json'), 'utf8')).version;
if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Invalid release version');
if (process.env.GITHUB_REF_TYPE === 'tag' && process.env.GITHUB_REF_NAME !== `v${version}`) {
  throw new Error('Release tag and application version do not match');
}
const out = path.join(root, 'app/release');
const installerName = (platform) => `GPU-Monitor_${version}_${platform === 'windows' ? 'windows-x64-setup.exe' : 'macos-universal.dmg'}`;
const cliName = (platform) => `gpu-monitor-cli_${version}_${platform === 'windows' ? 'windows-x64.exe' : 'macos-universal.tar.gz'}`;
if (process.argv.includes('--verify')) {
  for (const platform of ['windows', 'macos']) {
    for (const file of [installerName(platform), cliName(platform)]) {
      if (!statSync(path.join(out, file)).size) throw new Error(`Empty release file: ${file}`);
    }
  }
} else {
  const mac = process.platform === 'darwin';
  if (!mac && process.platform !== 'win32') throw new Error('Build on Windows or macOS');
  const platform = mac ? 'macos' : 'windows';
  const release = path.join(root, 'src-tauri/target', mac ? 'universal-apple-darwin/release' : 'release');
  const bundle = path.join(release, 'bundle', mac ? 'dmg' : 'nsis');
  const installers = readdirSync(bundle).filter(file => file.endsWith(mac ? '.dmg' : '.exe'));
  if (installers.length !== 1) throw new Error('Expected exactly one installer');
  mkdirSync(out, { recursive: true });
  copyFileSync(path.join(bundle, installers[0]), path.join(out, installerName(platform)));
  if (mac) {
    // Use the signed CLI from the bundle, and preserve its executable permission in the archive.
    execFileSync('tar', ['-czf', path.join(out, cliName(platform)), '-C',
      path.join(release, 'bundle/macos/GPU Monitor.app/Contents/MacOS'), 'gpu-monitor-cli'],
      { env: { ...process.env, COPYFILE_DISABLE: '1' } });
  } else {
    copyFileSync(path.join(release, 'gpu-monitor-cli.exe'), path.join(out, cliName(platform)));
  }
}
