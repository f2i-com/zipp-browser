#!/usr/bin/env node
/**
 * Sync or verify the pinned `zipp-vm` release tag in `zipp-browser` against
 * the latest published GitHub Release of `f2i-com/zipp.org`.
 *
 * Node built-ins only (no npm install required).
 *
 * Usage:
 *   node tools/release/sync-zipp-release.mjs --check [--allow-older-zipp]
 *   node tools/release/sync-zipp-release.mjs --apply [--tag v0.0.40] [--no-lock]
 */

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, '..', '..');
const MANIFESTS = [
  path.join(REPO_ROOT, 'apps', 'browser', 'Cargo.toml'),
  path.join(REPO_ROOT, 'crates', 'zipp-android-webview', 'Cargo.toml'),
];

const ZIPP_REPO = process.env.ZIPP_REPO || 'f2i-com/zipp.org';
const ZIPP_VM_RE = /(zipp-vm\s*=\s*\{[^}\n]*git\s*=\s*"https:\/\/github\.com\/f2i-com\/zipp\.org"[^}\n]*)(?:tag|branch|rev)\s*=\s*"([^"]+)"/;

function ghHeaders() {
  const headers = {
    Accept: 'application/vnd.github+json',
    'User-Agent': 'zipp-browser-release',
  };
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
  if (token) headers.Authorization = `Bearer ${token}`;
  return headers;
}

export async function fetchLatestZippRelease(fetchImpl = globalThis.fetch) {
  const url = `https://api.github.com/repos/${ZIPP_REPO}/releases/latest`;
  const resp = await fetchImpl(url, { headers: ghHeaders() });
  if (!resp.ok) {
    throw new Error(`Failed to fetch latest ZIPP release from ${url}: HTTP ${resp.status}`);
  }
  const data = await resp.json();
  if (!data?.tag_name || typeof data.tag_name !== 'string') {
    throw new Error(`Unexpected GitHub API payload from ${url}: missing tag_name`);
  }
  return {
    tag: data.tag_name,
    url: data.html_url || `https://github.com/${ZIPP_REPO}/releases/tag/${data.tag_name}`,
  };
}

export function readDeclaredZippTag(manifestPath) {
  const content = fs.readFileSync(manifestPath, 'utf8');
  const m = content.match(ZIPP_VM_RE);
  if (!m) {
    throw new Error(`Could not find zipp-vm git dependency in ${manifestPath}`);
  }
  return m[2];
}

export function writeDeclaredZippTag(manifestPath, tag) {
  const content = fs.readFileSync(manifestPath, 'utf8');
  if (!ZIPP_VM_RE.test(content)) {
    throw new Error(`Could not find zipp-vm git dependency in ${manifestPath}`);
  }
  const updated = content.replace(ZIPP_VM_RE, `$1tag = "${tag}"`);
  if (updated !== content) {
    fs.writeFileSync(manifestPath, updated, 'utf8');
    return true;
  }
  return false;
}

function existingManifests() {
  return MANIFESTS.filter((p) => fs.existsSync(p));
}

function writeGithubOutput(kv) {
  const out = process.env.GITHUB_OUTPUT;
  if (!out) return;
  const lines = Object.entries(kv)
    .map(([k, v]) => `${k}=${v}\n`)
    .join('');
  fs.appendFileSync(out, lines);
}

async function main() {
  const args = process.argv.slice(2);
  const modeApply = args.includes('--apply');
  const modeCheck = args.includes('--check') || !modeApply;
  const allowOlder = args.includes('--allow-older-zipp') || /^(1|true)$/i.test(process.env.ALLOW_OLDER_ZIPP ?? '');
  const noLock = args.includes('--no-lock');
  const tagIdx = args.indexOf('--tag');
  const explicitTag = tagIdx >= 0 ? args[tagIdx + 1] : null;

  const manifests = existingManifests();
  if (manifests.length === 0) {
    throw new Error('No Cargo.toml manifests found to inspect');
  }

  const declaredTags = manifests.map((m) => ({
    path: path.relative(REPO_ROOT, m),
    tag: readDeclaredZippTag(m),
  }));

  const uniqueDeclared = [...new Set(declaredTags.map((d) => d.tag))];
  const latest = explicitTag
    ? { tag: explicitTag, url: `https://github.com/${ZIPP_REPO}/releases/tag/${explicitTag}` }
    : await fetchLatestZippRelease();

  if (modeApply) {
    let changed = false;
    for (const m of manifests) {
      if (writeDeclaredZippTag(m, latest.tag)) {
        console.log(`[zipp-sync] Updated ${path.relative(REPO_ROOT, m)} -> zipp-vm ${latest.tag}`);
        changed = true;
      }
    }
    if (!noLock && changed) {
      console.log(`[zipp-sync] Refreshing workspace Cargo.lock for zipp-vm ${latest.tag}...`);
      execFileSync('cargo', ['update', '-p', 'zipp-vm'], { cwd: REPO_ROOT, stdio: 'inherit' });
      const androidManifest = path.join(REPO_ROOT, 'crates', 'zipp-android-webview', 'Cargo.toml');
      if (fs.existsSync(androidManifest)) {
        console.log(`[zipp-sync] Refreshing Android WebView Cargo.lock for zipp-vm ${latest.tag}...`);
        execFileSync('cargo', ['update', '-p', 'zipp-vm', '--manifest-path', androidManifest], {
          cwd: REPO_ROOT,
          stdio: 'inherit',
        });
      }
    }
    writeGithubOutput({ 'zipp-release': latest.tag, 'zipp-latest': latest.tag });
    console.log(`[zipp-sync] Using ZIPP release ${latest.tag} (${latest.url})`);
    return;
  }

  if (modeCheck) {
    if (uniqueDeclared.length > 1) {
      const detail = declaredTags.map((d) => `${d.path}=${d.tag}`).join(', ');
      throw new Error(`Mismatched zipp-vm tags across manifests: ${detail}`);
    }
    const declared = uniqueDeclared[0];
    if (declared !== latest.tag && !allowOlder) {
      throw new Error(
        `Manifests pin zipp-vm ${declared}, but latest ZIPP release is ${latest.tag}. Run: node tools/release/sync-zipp-release.mjs --apply`
      );
    }
    writeGithubOutput({
      'zipp-release': declared,
      'zipp-latest': latest.tag,
    });
    console.log(`[zipp-sync] Verified zipp-vm ${declared} (latest ZIPP release: ${latest.tag})`);
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((err) => {
    console.error(`[zipp-sync] ERROR: ${err.message}`);
    process.exit(1);
  });
}
