# Upstream Sync - Conflict Resolution Template

This document provides guidance for Claude Code when resolving merge conflicts during upstream synchronization.

## Context

**Repository:** Personal fork of clash-verge-rev/clash-verge-rev
**Purpose:** Sync upstream releases while preserving fork-specific customizations
**Conflict Strategy:** Preserve fork modifications, accept upstream improvements

## Fork-Specific Modifications

All fork-specific code is marked with `// FORK:` comments. These modifications MUST be preserved during conflict resolution.

### Common Fork Patterns

1. **Disabled upstream features:**

   ```yaml
   # FORK: disabled, no Telegram channel
   if: false
   ```

2. **Modified build targets:**

   ```yaml
   # FORK: disabled, no ARM Linux targets needed
   if: false
   ```

3. **Custom repository references:**

   ```typescript
   // FORK: custom repository URL
   const REPO_URL = 'https://github.com/iuin8/clash-verge-rev'
   ```

4. **Simplified workflows:**
   ```yaml
   # FORK: simplified release downloads
   # Only show actually built platforms
   ```

## Conflict Resolution Rules

### Rule 1: Preserve Fork Markers

**Priority:** HIGHEST

When encountering conflicts in code with `// FORK:` markers:

- **ALWAYS** preserve the fork-specific logic
- Accept upstream changes only if they don't conflict with fork intent
- If upstream significantly refactors the area, adapt the fork logic to the new structure

**Example:**

```yaml
# Upstream adds new platforms
- os: windows-latest
  target: x86_64-pc-windows-msvc
+ target: i686-pc-windows-msvc  # New upstream target

# Fork disables some platforms
# FORK: disabled, no ARM Linux targets needed
if: false

# Resolution: Keep fork's if: false, ignore upstream's new target
```

### Rule 2: Accept Upstream Bug Fixes

**Priority:** HIGH

For bug fixes and security patches:

- Accept upstream changes unless they break fork-specific features
- If a fix conflicts with fork code, adapt the fork code to incorporate the fix

**Example:**

```typescript
// Upstream fixes a memory leak
- const cache = new Map();
+ const cache = new WeakMap(); // Prevents memory leak

// Fork has custom cache logic
// FORK: custom cache with TTL
const cache = new TTLCache();

// Resolution: Apply the fix principle to fork code
// FORK: custom cache with TTL (using WeakMap for memory safety)
const cache = new TTLWeakMap();
```

### Rule 3: Merge Non-Conflicting Features

**Priority:** MEDIUM

For new features that don't conflict with fork modifications:

- Accept upstream additions
- Integrate them alongside fork-specific code

**Example:**

```typescript
// Upstream adds new feature
+ function newUpstreamFeature() { ... }

// Fork has custom features
// FORK: custom feature for personal use
function customFeature() { ... }

// Resolution: Keep both
function newUpstreamFeature() { ... }

// FORK: custom feature for personal use
function customFeature() { ... }
```

### Rule 4: Dependency Updates

**Priority:** MEDIUM

For dependency version bumps:

- Accept upstream dependency updates
- Verify compatibility with fork-specific code after resolution

**Example:**

```json
// Upstream updates dependency
- "react": "^18.0.0"
+ "react": "^19.0.0"

// Resolution: Accept the update
"react": "^19.0.0"
```

### Rule 5: Configuration Changes

**Priority:** MEDIUM

For configuration file changes:

- Merge upstream config improvements
- Preserve fork-specific config values

**Example:**

```json
// Upstream adds new config option
{
  "name": "clash-verge-rev",
+ "newOption": true
}

// Fork has custom name
{
  "name": "clash-verge-rev-fork",  // FORK: custom name
}

// Resolution: Merge both
{
  "name": "clash-verge-rev-fork",  // FORK: custom name
  "newOption": true
}
```

## Conflict Resolution Workflow

### Step 1: Identify Conflict Type

Categorize each conflict:

- **Type A:** Fork-marked code vs upstream changes → Preserve fork
- **Type B:** Bug fix vs fork code → Adapt fork to incorporate fix
- **Type C:** New feature vs fork code → Merge both
- **Type D:** Dependency update → Accept upstream
- **Type E:** Unclear → Ask for guidance

### Step 2: Apply Resolution Rules

For each conflict file:

1. Read the entire file to understand context
2. Identify all `// FORK:` markers
3. Apply the appropriate rule based on conflict type
4. Verify the resolution makes logical sense

### Step 3: Verify Resolution

After resolving conflicts:

```bash
# TypeScript/JavaScript files
pnpm typecheck
pnpm lint

# Rust files
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt -- --check

# Run tests if available
pnpm test
cargo test
```

### Step 4: Commit with Clear Message

```bash
git add .
git commit -m "chore: resolve conflicts for upstream v{VERSION}

Conflict resolution summary:
- Preserved fork-specific modifications in {files}
- Accepted upstream bug fixes in {files}
- Merged new features in {files}

All tests passing."
```

## Special Cases

### Case 1: Workflow File Conflicts

Workflow files (`.github/workflows/*.yml`) often have fork-specific modifications:

- Disabled jobs (Telegram notifications, winget submissions)
- Modified build matrices
- Custom repository references

**Resolution:** Preserve all fork modifications, accept only non-conflicting upstream improvements.

### Case 2: Package Version Conflicts

When both upstream and fork bump versions:

- Use the higher version number
- Update all related files consistently (package.json, Cargo.toml, tauri.conf.json)

### Case 3: Documentation Conflicts

For README, CHANGELOG, and docs:

- Accept upstream documentation updates
- Preserve fork-specific documentation sections
- Merge both if they document different aspects

### Case 4: Build Script Conflicts

For scripts in `scripts/`:

- Preserve fork-specific script modifications
- Accept upstream script improvements if they don't break fork usage
- Test scripts after resolution

## When to Ask for Help

Ask for human guidance when:

1. **Unclear intent:** Cannot determine if upstream change should override fork modification
2. **Breaking changes:** Upstream refactors significantly affect fork-specific features
3. **Test failures:** Conflict resolution causes tests to fail
4. **Security concerns:** Conflict involves authentication, secrets, or security-sensitive code

**How to ask:**

````markdown
@{username} I need guidance on this conflict:

**File:** `path/to/file.ts`
**Conflict:** Upstream refactored authentication flow, but we have fork-specific auth logic

**Upstream change:**

```diff
- oldAuthMethod()
+ newAuthMethod()
```
````

**Fork modification:**

```typescript
// FORK: custom auth for personal use
customAuthMethod()
```

**Question:** Should I:
A) Adapt fork's customAuthMethod to use newAuthMethod internally?
B) Keep fork's customAuthMethod as-is and ignore upstream change?
C) Something else?

````

## Testing Checklist

After resolving all conflicts:

- [ ] All `// FORK:` markers are preserved
- [ ] TypeScript compiles: `pnpm typecheck`
- [ ] Linting passes: `pnpm lint`
- [ ] Rust compiles: `cargo clippy-all`
- [ ] Rust formatting: `cargo fmt -- --check`
- [ ] Tests pass: `pnpm test && cargo test`
- [ ] Build succeeds: `pnpm build`
- [ ] Commit message is clear and descriptive

## Example Conflict Resolutions

### Example 1: Workflow Conflict

**Conflict:**
```yaml
<<<<<<< HEAD
  notify-telegram:
    if: false # FORK: disabled, no Telegram channel
=======
  notify-telegram:
    if: ${{ !contains(github.ref_name, '-rc') }}
    needs: [update_tag, release-update]
>>>>>>> upstream/v1.7.7
````

**Resolution:**

```yaml
notify-telegram:
  if: false # FORK: disabled, no Telegram channel
  needs: [update_tag, release-update] # Keep upstream's needs update
```

**Reasoning:** Preserve fork's disabled state, but accept upstream's dependency update.

### Example 2: Code Logic Conflict

**Conflict:**

```typescript
<<<<<<< HEAD
// FORK: custom update check interval
const UPDATE_INTERVAL = 24 * 60 * 60 * 1000; // 24 hours
=======
const UPDATE_INTERVAL = 12 * 60 * 60 * 1000; // 12 hours (upstream improvement)
>>>>>>> upstream/v1.7.7
```

**Resolution:**

```typescript
// FORK: custom update check interval
const UPDATE_INTERVAL = 24 * 60 * 60 * 1000 // 24 hours
```

**Reasoning:** Fork intentionally uses longer interval, preserve fork preference.

### Example 3: Dependency Conflict

**Conflict:**

```json
<<<<<<< HEAD
  "dependencies": {
    "react": "^19.0.0"
  }
=======
  "dependencies": {
    "react": "^19.1.0"
  }
>>>>>>> upstream/v1.7.7
```

**Resolution:**

```json
  "dependencies": {
    "react": "^19.1.0"
  }
```

**Reasoning:** Accept upstream's newer dependency version (likely includes bug fixes).

## Summary

**Golden Rule:** When in doubt, preserve fork modifications and ask for guidance.

The goal is to keep the fork up-to-date with upstream improvements while maintaining the custom modifications that make this fork unique.
