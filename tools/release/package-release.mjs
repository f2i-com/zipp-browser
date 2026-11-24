#!/usr/bin/env node
/**
 * Release packager for Zipp Browser (portable desktop releases + Android WebView bundle).
 *
 * Node built-ins only (zero npm dependencies).
 *
 * Produces:
 *   - Windows portable (NO installer):
 *       zipp-browser-<tag>-windows-x86_64-portable.exe   (standalone portable binary)
 *       zipp-browser-<tag>-windows-x86_64-portable.zip   (portable folder with zipp-browser.exe + portable.txt + README.md)
 *   - Linux portable:
 *       zipp-browser-<tag>-linux-x86_64.tar.gz
 *   - Android WebView / SoftN Mobile native runtime:
 *       zipp-browser-<tag>-android-webview.zip           (jniLibs/{arm64-v8a,x86_64}/libsoftn_mobile.so + manifest.json + SHA256SUMS.txt)
 *   - Checksums:
 *       SHA256SUMS.txt
 *
 * Usage:
 *   node tools/release/package-release.mjs --windows [--tag v0.1.2] [--out dist-release]
 *   node tools/release/package-release.mjs --linux   [--tag v0.1.2] [--out dist-release]
 *   node tools/release/package-release.mjs --android [--build-android] [--abi all|arm64-v8a|x86_64] [--tag v0.1.2]
 *   node tools/release/package-release.mjs --sums    [--out dist-release]
 */

import { execFileSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import zlib from 'node:zlib';
import { readDeclaredZippTag } from './sync-zipp-release.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, '..', '..');
const ANDROID_CRATE_DIR = path.join(REPO_ROOT, 'crates', 'zipp-android-webview');

const ANDROID_ABIS = {
  'arm64-v8a': { rustTarget: 'aarch64-linux-android', clangPrefix: 'aarch64-linux-android26-clang' },
  x86_64: { rustTarget: 'x86_64-linux-android', clangPrefix: 'x86_64-linux-android26-clang' },
};

// ---------------------------------------------------------------------------
// Minimal deterministic ZIP writer (DEFLATE, Node built-ins only)
// ---------------------------------------------------------------------------

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let j = 0; j < 8; j++) {
      c = (c & 1) !== 0 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[i] = c >>> 0;
  }
  return table;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  }
  return (c ^ 0xffffffff) >>> 0;
}

/**
 * Write a standard ZIP archive containing `{ name: string, data: Buffer }[]`.
 * Uses fixed DOS timestamp so output archives are reproducible.
 */
export function writeZip(entries, outPath) {
  const localParts = [];
  const centralParts = [];
  let offset = 0;

  for (const entry of entries) {
    const nameBuf = Buffer.from(entry.name.replace(/\\/g, '/'), 'utf8');
    const raw = Buffer.isBuffer(entry.data) ? entry.data : Buffer.from(entry.data);
    const compressed = zlib.deflateRawSync(raw, { level: 6 });
    const useDeflate = compressed.length < raw.length;
    const body = useDeflate ? compressed : raw;
    const method = useDeflate ? 8 : 0;
    const crc = crc32(raw);

    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0); // local file header sig
    local.writeUInt16LE(20, 4); // version needed
    local.writeUInt16LE(0x0800, 6); // UTF-8 flag
    local.writeUInt16LE(method, 8);
    local.writeUInt16LE(0, 10); // mod time
    local.writeUInt16LE(0x5421, 12); // fixed DOS date (2022-01-01)
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(body.length, 18);
    local.writeUInt32LE(raw.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    local.writeUInt16LE(0, 28); // extra len
    localParts.push(local, nameBuf, body);

    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0); // central dir sig
    central.writeUInt16LE(20, 4); // version made by
    central.writeUInt16LE(20, 6); // version needed
    central.writeUInt16LE(0x0800, 8); // UTF-8 flag
    central.writeUInt16LE(method, 10);
    central.writeUInt16LE(0, 12);
    central.writeUInt16LE(0x5421, 14);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(body.length, 20);
    central.writeUInt32LE(raw.length, 24);
    central.writeUInt16LE(nameBuf.length, 28);
    central.writeUInt16LE(0, 30); // extra len
    central.writeUInt16LE(0, 32); // comment len
    central.writeUInt16LE(0, 34); // disk start
    central.writeUInt16LE(0, 36); // internal attrs
    central.writeUInt32LE(0, 38); // external attrs
    central.writeUInt32LE(offset, 42);
    centralParts.push(central, nameBuf);

    offset += local.length + nameBuf.length + body.length;
  }

  const centralBuf = Buffer.concat(centralParts);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(0, 4);
  eocd.writeUInt16LE(0, 6);
  eocd.writeUInt16LE(entries.length, 8);
  eocd.writeUInt16LE(entries.length, 10);
  eocd.writeUInt32LE(centralBuf.length, 12);
  eocd.writeUInt32LE(offset, 16);
  eocd.writeUInt16LE(0, 20);

  fs.mkdirSync(path.dirname(outPath), { recursive: true });
  fs.writeFileSync(outPath, Buffer.concat([...localParts, centralBuf, eocd]));
}

function sha256File(filePath) {
  return crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex');
}

function sha256Buffer(buf) {
  return crypto.createHash('sha256').update(buf).digest('hex');
}

function readWorkspaceVersion() {
  const toml = fs.readFileSync(path.join(REPO_ROOT, 'Cargo.toml'), 'utf8');
  const m = toml.match(/^\s*version\s*=\s*"([^"]+)"/m);
  return m ? m[1] : '0.1.2';
}

function findNdkBin() {
  const candidates = [
    process.env.ANDROID_NDK_HOME,
    process.env.ANDROID_NDK_ROOT,
    process.env.NDK_HOME,
  ].filter(Boolean);

  let ndkDir = candidates.find((d) => fs.existsSync(d));
  if (!ndkDir) {
    const sdkRoots = [
      process.env.ANDROID_HOME,
      process.env.ANDROID_SDK_ROOT,
      path.join(os.homedir(), 'AppData', 'Local', 'Android', 'Sdk'),
      path.join(os.homedir(), 'Android', 'Sdk'),
      path.join(os.homedir(), 'Library', 'Android', 'sdk'),
      '/usr/local/lib/android/sdk',
    ].filter(Boolean);
    for (const root of sdkRoots) {
      const ndkRoot = path.join(root, 'ndk');
      if (!fs.existsSync(ndkRoot)) continue;
      const versions = fs.readdirSync(ndkRoot).sort().reverse();
      const chosen = versions.find((v) => v.startsWith('27.')) || versions[0];
      if (chosen) {
        ndkDir = path.join(ndkRoot, chosen);
        break;
      }
    }
  }
  if (!ndkDir) {
    throw new Error('Android NDK not found. Set ANDROID_NDK_HOME or ANDROID_HOME.');
  }
  const hostTag =
    process.platform === 'win32'
      ? 'windows-x86_64'
      : process.platform === 'darwin'
        ? 'darwin-x86_64'
        : 'linux-x86_64';
  const bin = path.join(ndkDir, 'toolchains', 'llvm', 'prebuilt', hostTag, 'bin');
  if (!fs.existsSync(bin)) {
    throw new Error(`Android NDK llvm toolchain bin not found at ${bin}`);
  }
  return bin;
}

function buildAndroidAbi(abi) {
  const spec = ANDROID_ABIS[abi];
  if (!spec) throw new Error(`Unsupported Android ABI: ${abi}`);
  const ndkBin = findNdkBin();
  const ccExt = process.platform === 'win32' ? '.cmd' : '';
  const arExt = process.platform === 'win32' ? '.exe' : '';
  const cc = path.join(ndkBin, `${spec.clangPrefix}${ccExt}`);
  const ar = path.join(ndkBin, `llvm-ar${arExt}`);
  const key = spec.rustTarget.replace(/-/g, '_');
  const upperKey = key.toUpperCase();

  const env = {
    ...process.env,
    [`CC_${key}`]: cc,
    [`AR_${key}`]: ar,
    [`CARGO_TARGET_${upperKey}_LINKER`]: cc,
  };

  console.log(`[release] Building zipp-android-webview (${abi} / ${spec.rustTarget})...`);
  execFileSync(
    'cargo',
    [
      'build',
      '--release',
      '--manifest-path',
      path.join(ANDROID_CRATE_DIR, 'Cargo.toml'),
      '--target',
      spec.rustTarget,
    ],
    { cwd: REPO_ROOT, env, stdio: 'inherit' }
  );
}

function packageWindowsPortable(tag, outDir) {
  const exeSrc = path.join(REPO_ROOT, 'target', 'release', 'zipp-browser.exe');
  if (!fs.existsSync(exeSrc)) {
    throw new Error(`Windows release binary not found at ${exeSrc}. Run cargo build --release --bin zipp-browser first.`);
  }
  const readmeSrc = path.join(REPO_ROOT, 'README.md');
  fs.mkdirSync(outDir, { recursive: true });

  // 1. Standalone single-file portable executable (no installer)
  const portableExeName = `zipp-browser-${tag}-windows-x86_64-portable.exe`;
  const portableExeOut = path.join(outDir, portableExeName);
  fs.copyFileSync(exeSrc, portableExeOut);
  console.log(`[release] Wrote ${portableExeOut} (${(fs.statSync(portableExeOut).size / 1048576).toFixed(2)} MB)`);

  // 2. Self-contained portable ZIP folder (keeps settings/cookies/history/cache in ./data/)
  const portableNote = Buffer.from(
    [
      'ZIPP Browser Portable Mode (Part of the ZIPP Project · Apache-2.0)',
      '==================================================================',
      '',
      'Because this portable.txt marker file sits beside zipp-browser.exe,',
      'all browser settings, cookies, history, and HTTP cache are stored inside',
      'the local ./data/ folder instead of %LOCALAPPDATA%\\Zipp.',
      '',
      'To use the standard user profile directory instead, delete portable.txt.',
      '',
    ].join('\r\n'),
    'utf8'
  );

  const licenseSrc = path.join(REPO_ROOT, 'LICENSE-APACHE');
  const zipName = `zipp-browser-${tag}-windows-x86_64-portable.zip`;
  const zipOut = path.join(outDir, zipName);
  const entries = [
    { name: 'zipp-browser.exe', data: fs.readFileSync(exeSrc) },
    { name: 'portable.txt', data: portableNote },
  ];
  if (fs.existsSync(licenseSrc)) {
    entries.push({ name: 'LICENSE-APACHE', data: fs.readFileSync(licenseSrc) });
  }
  if (fs.existsSync(readmeSrc)) {
    entries.push({ name: 'README.md', data: fs.readFileSync(readmeSrc) });
  }
  writeZip(entries, zipOut);
  console.log(`[release] Wrote ${zipOut} (${(fs.statSync(zipOut).size / 1048576).toFixed(2)} MB)`);
}

function packageLinuxPortable(tag, outDir) {
  const binSrc = path.join(REPO_ROOT, 'target', 'release', 'zipp-browser');
  if (!fs.existsSync(binSrc)) {
    throw new Error(`Linux release binary not found at ${binSrc}.`);
  }
  fs.mkdirSync(outDir, { recursive: true });
  const stage = fs.mkdtempSync(path.join(os.tmpdir(), 'zipp-linux-stage-'));
  try {
    fs.copyFileSync(binSrc, path.join(stage, 'zipp-browser'));
    fs.chmodSync(path.join(stage, 'zipp-browser'), 0o755);
    const licenseSrc = path.join(REPO_ROOT, 'LICENSE-APACHE');
    if (fs.existsSync(licenseSrc)) {
      fs.copyFileSync(licenseSrc, path.join(stage, 'LICENSE-APACHE'));
    }
    const readmeSrc = path.join(REPO_ROOT, 'README.md');
    if (fs.existsSync(readmeSrc)) {
      fs.copyFileSync(readmeSrc, path.join(stage, 'README.md'));
    }
    const tarName = `zipp-browser-${tag}-linux-x86_64.tar.gz`;
    const tarOut = path.join(outDir, tarName);
    execFileSync('tar', ['-czf', tarOut, '-C', stage, '.'], { stdio: 'inherit' });
    console.log(`[release] Wrote ${tarOut} (${(fs.statSync(tarOut).size / 1048576).toFixed(2)} MB)`);
  } finally {
    fs.rmSync(stage, { recursive: true, force: true });
  }
}

function packageAndroidWebView(tag, version, outDir, abis, buildFirst) {
  const zippRelease = readDeclaredZippTag(path.join(ANDROID_CRATE_DIR, 'Cargo.toml'));
  const entries = [];
  const shaMap = {};
  const sumLines = [];

  for (const abi of abis) {
    const spec = ANDROID_ABIS[abi];
    if (!spec) throw new Error(`Unsupported ABI: ${abi}`);
    if (buildFirst) {
      buildAndroidAbi(abi);
    }
    const soPath = path.join(
      ANDROID_CRATE_DIR,
      'target',
      spec.rustTarget,
      'release',
      'libsoftn_mobile.so'
    );
    if (!fs.existsSync(soPath)) {
      throw new Error(
        `Compiled Android library missing for ${abi} at ${soPath}. Re-run with --build-android.`
      );
    }
    const soBytes = fs.readFileSync(soPath);
    const relPath = `jniLibs/${abi}/libsoftn_mobile.so`;
    const digest = sha256Buffer(soBytes);
    shaMap[relPath] = digest;
    sumLines.push(`${digest}  ${relPath}`);
    entries.push({ name: relPath, data: soBytes });
    console.log(`[release] Staged ${relPath} (${(soBytes.length / 1048576).toFixed(2)} MB, sha256=${digest.slice(0, 12)}…)`);
  }

  const manifest = {
    schema: 1,
    tag,
    version,
    zipp_release: zippRelease,
    library: 'libsoftn_mobile.so',
    abis,
    sha256: shaMap,
  };
  entries.push({
    name: 'manifest.json',
    data: Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`, 'utf8'),
  });
  entries.push({
    name: 'SHA256SUMS.txt',
    data: Buffer.from(`${sumLines.join('\n')}\n`, 'utf8'),
  });
  const licenseSrc = path.join(REPO_ROOT, 'LICENSE-APACHE');
  if (fs.existsSync(licenseSrc)) {
    entries.push({ name: 'LICENSE-APACHE', data: fs.readFileSync(licenseSrc) });
  }

  const zipName = `zipp-browser-${tag}-android-webview.zip`;
  const zipOut = path.join(outDir, zipName);
  writeZip(entries, zipOut);
  console.log(`[release] Wrote ${zipOut} (${(fs.statSync(zipOut).size / 1048576).toFixed(2)} MB)`);
}

function writeReleaseChecksums(outDir) {
  if (!fs.existsSync(outDir)) {
    throw new Error(`Release output directory not found: ${outDir}`);
  }
  const files = fs
    .readdirSync(outDir)
    .filter((f) => f !== 'SHA256SUMS.txt' && fs.statSync(path.join(outDir, f)).isFile())
    .sort();
  const lines = files.map((f) => `${sha256File(path.join(outDir, f))}  ${f}`);
  const outPath = path.join(outDir, 'SHA256SUMS.txt');
  fs.writeFileSync(outPath, `${lines.join('\n')}\n`, 'utf8');
  console.log(`[release] Wrote ${outPath} (${files.length} files)`);
}

function main() {
  const args = process.argv.slice(2);
  const version = readWorkspaceVersion();
  const tagIdx = args.indexOf('--tag');
  const tag = tagIdx >= 0 ? args[tagIdx + 1] : `v${version}`;
  const outIdx = args.indexOf('--out');
  const outDir = path.resolve(REPO_ROOT, outIdx >= 0 ? args[outIdx + 1] : 'dist-release');
  const abiIdx = args.indexOf('--abi');
  const abiArg = abiIdx >= 0 ? args[abiIdx + 1] : 'all';
  const abis = abiArg === 'all' ? Object.keys(ANDROID_ABIS) : abiArg.split(',').map((s) => s.trim()).filter(Boolean);

  const doWindows = args.includes('--windows');
  const doLinux = args.includes('--linux');
  const doAndroid = args.includes('--android');
  const buildAndroid = args.includes('--build-android');
  const doSums = args.includes('--sums') || doWindows || doLinux || doAndroid;

  if (!doWindows && !doLinux && !doAndroid && !args.includes('--sums')) {
    console.error(
      'Usage: node tools/release/package-release.mjs [--windows] [--linux] [--android [--build-android] [--abi all|arm64-v8a|x86_64]] [--sums] [--tag vX.Y.Z] [--out dir]'
    );
    process.exit(1);
  }

  if (doWindows) packageWindowsPortable(tag, outDir);
  if (doLinux) packageLinuxPortable(tag, outDir);
  if (doAndroid) packageAndroidWebView(tag, version, outDir, abis, buildAndroid);
  if (doSums) writeReleaseChecksums(outDir);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
