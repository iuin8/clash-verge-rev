// Fail fast when a local build would package a stale generated service helper.
// `pnpm build` does not run `pnpm prebuild`; the ignored src-tauri/resources
// directory can therefore contain an old helper. The release workflow runs
// prebuild explicitly, but local builds need the same invariant.

import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const cargoManifest = join(root, 'src-tauri', 'Cargo.toml')
const cargo = readFileSync(cargoManifest, 'utf8')
const dependency = cargo.match(
  /^clash_verge_service_ipc\s*=\s*\{[^}]*\btag\s*=\s*"v?([^"]+)"/m,
)
if (!dependency) {
  throw new Error(
    'clash_verge_service_ipc must declare a tagged service helper version',
  )
}
const expected = dependency[1]
const serviceName =
  process.platform === 'win32'
    ? 'clash-verge-service.exe'
    : 'clash-verge-service'
const servicePath = join(root, 'src-tauri', 'resources', serviceName)

let bytes
try {
  bytes = readFileSync(servicePath)
} catch {
  throw new Error(
    `missing generated service helper ${servicePath}; run "pnpm run prebuild" before building`,
  )
}
const marker = Buffer.from(expected, 'latin1')
if (!bytes.includes(marker)) {
  throw new Error(
    `stale generated service helper ${servicePath}: expected ${expected}; run "pnpm run prebuild" and rebuild`,
  )
}
console.log(`ok: service helper matches ${expected}`)
