import fs from 'fs'
import path from 'path'

const SERVICE_CRATE_NAME = 'clash_verge_service_ipc'

function resolveServiceVersionFromCargoLock(lock) {
  const packageBlocks = lock.split(/\n(?=\[\[package\]\])/)

  for (const block of packageBlocks) {
    if (!block.includes(`name = "${SERVICE_CRATE_NAME}"`)) continue

    const match = block.match(/version = "([^"]+)"/)
    return match ? `v${match[1]}` : null
  }

  return null
}

const lockPath = path.join(process.cwd(), 'Cargo.lock')
const serviceVersion = resolveServiceVersionFromCargoLock(fs.readFileSync(lockPath, 'utf-8'))

if (!serviceVersion) {
  console.error(`Unable to resolve ${SERVICE_CRATE_NAME} version from ${lockPath}`)
  process.exit(1)
}

console.log(serviceVersion)
