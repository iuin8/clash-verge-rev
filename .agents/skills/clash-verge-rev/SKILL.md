```markdown
# clash-verge-rev Development Patterns

> Auto-generated skill from repository analysis

## Overview
This skill covers the core development patterns and conventions used in the `clash-verge-rev` repository, a Rust-based project with a React frontend. You'll learn about file naming, import/export styles, commit message patterns, and how to structure and run tests. This guide is designed to help maintain consistency and efficiency when contributing to the codebase.

## Coding Conventions

### File Naming
- Use **camelCase** for all file names.
  - Example: `userSettings.rs`, `networkConfig.tsx`

### Import Style
- Use **alias imports** to clarify module usage.
  - Example (Rust):
    ```rust
    use crate::network as net;
    ```
  - Example (React/JS):
    ```javascript
    import { fetchData as getData } from './apiUtils';
    ```

### Export Style
- Use **named exports** for all modules.
  - Example (Rust):
    ```rust
    pub fn connect() { /* ... */ }
    pub fn disconnect() { /* ... */ }
    ```
  - Example (React/JS):
    ```javascript
    export function startService() { /* ... */ }
    export function stopService() { /* ... */ }
    ```

### Commit Message Patterns
- Use **conventional commits** with the `fix` prefix for bug fixes.
  - Example:
    ```
    fix: resolve connection timeout issue
    ```
- Keep commit messages concise (average: ~46 characters).

## Workflows

### Code Contribution
**Trigger:** When adding new features or fixing bugs  
**Command:** `/contribute`

1. Create a new branch from `main`.
2. Follow camelCase naming for new files.
3. Use alias imports and named exports.
4. Write or update tests as needed (see Testing Patterns).
5. Commit changes using the conventional commit format.
6. Open a pull request for review.

### Bug Fixing
**Trigger:** When resolving a reported bug  
**Command:** `/fix-bug`

1. Checkout a new branch: `fix/short-description`.
2. Make your changes, following coding conventions.
3. Write or update relevant tests.
4. Commit with a `fix:` prefix and concise message.
5. Push and open a pull request.

## Testing Patterns

- **Test File Naming:**  
  - Use the pattern `*.test.*` for test files.
    - Example: `userSettings.test.rs`, `apiUtils.test.ts`
- **Testing Framework:**  
  - Not explicitly specified; follow standard Rust or React testing practices.
- **Test Placement:**  
  - Place test files alongside the modules they test or in a dedicated `tests` directory.

**Example (Rust):**
```rust
// userSettings.test.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_settings() {
        // test implementation
    }
}
```

**Example (React/JS):**
```javascript
// apiUtils.test.ts
import { fetchData } from './apiUtils';

test('fetchData returns expected data', () => {
    // test implementation
});
```

## Commands
| Command      | Purpose                                   |
|--------------|-------------------------------------------|
| /contribute  | Start a new feature or general contribution|
| /fix-bug     | Begin a bug fix workflow                  |
```
