# Multi-Profile Merge — Rule Provider Strategy

**Date:** 2026-04-18
**Status:** Phase 1 shipped, follow-up remains for UX and smart rename
**Scope:** clash-verge-rev (fork)
**Related spec:** `docs/superpowers/specs/2026-03-28-multi-profile-merge-design.md`

---

## Why this doc exists

The original multi-profile merge feature worked for `proxies`, `proxy-groups`, and `rules`, but it could produce an invalid merged config when a supplementary profile contributed `RULE-SET` rules that depended on `rule-providers`.

Phase 1 has already fixed that correctness issue.

This doc now records:

- the historical root cause
- the exact failure shape
- the shipped baseline semantics for `rule-providers`
- the conflict handling model
- the optional smart-rename mode
- the recommended follow-up order

This is meant to save future debugging time and keep follow-up work scoped.

---

## Historical root cause

Before the fix, `src-tauri/src/enhance/multi_merge.rs` merged only these top-level fields from supplementary profiles:

- `proxies`
- `proxy-groups`
- `rules`

All other top-level fields from supplementary profiles were ignored.

That meant this broken sequence could happen:

1. supplementary profile defines `rule-providers.Local-LAN`
2. supplementary profile also defines `RULE-SET,Local-LAN,DIRECT`
3. merge keeps the `rules` entry
4. merge drops the `rule-providers` entry
5. mihomo validates the merged config
6. mihomo fails with `rule set [Local-LAN] not found`

This is not a user typo by default. It is an incomplete merge result.

---

## Concrete failure shape

### Source profile A

Valid on its own. No `rule-providers`.

### Source profile B

Valid on its own. Defines both:

- `rule-providers.Local-LAN`
- `RULE-SET,Local-LAN,...`

### Historical broken merged result

Invalid, because:

- the `RULE-SET` rule survives
- the matching `rule-providers.Local-LAN` definition does not

### User-visible error

Typical mihomo validation failure:

```text
rules[1] [RULE-SET,Local-LAN,DIRECT] error: rule set [Local-LAN] not found
```

---

## Product principle

We should not blame users for an internal merge limitation.

Two different situations must be presented differently:

1. **Internal merge incompleteness**
   - Example: merged config kept a `RULE-SET` reference but dropped the matching `rule-providers` definition.
   - UX stance: say the merged result is incomplete.
   - Do not frame this as "please rename your config".

2. **Real key conflict**
   - Example: two profiles define the same `rule-providers` key with different contents.
   - UX stance: explain the conflict and optionally guide the user to rename one side.

This distinction matters. Otherwise users will see a valid standalone subscription fail after merge and assume the app is gaslighting them.

---

## Shipped baseline semantics

Phase 1 now does the baseline correctness work:

- non-conflicting `rule-providers` merge into the result
- equal definitions dedupe without creating drift
- same-name, different-value definitions are surfaced as explicit conflicts while the earlier definition stays authoritative

The sections below describe that shipped baseline plus the remaining design for follow-up improvements.

## Desired merge semantics

### 1. Non-conflicting `rule-providers` must merge normally

If a supplementary profile contributes a `rule-providers` entry whose key does not exist in the base config yet, that entry should be copied into the merged result.

This is the minimum correctness fix.

### 2. Equal definitions should dedupe silently

If two profiles define the same `rule-providers` key and the normalized values are identical, treat them as duplicates:

- keep one
- do not rename
- do not block activation
- optionally log as an informational dedupe event, not a warning

### 3. Same key + different definition is a real conflict

If two profiles define the same `rule-providers` key but the contents differ, the merge layer should treat that as a conflict.

Default behavior should be conservative:

- preserve the earlier entry
- record the conflict
- surface it in the conflict list
- avoid silent semantic changes unless the user opted into smart rename

---

## Conflict taxonomy

Recommended categories for future conflict entries:

### `missing_dependency`

Merged config references a provider name that is not defined in the merged config.

Example:

- `RULE-SET,Local-LAN,DIRECT` exists
- `rule-providers.Local-LAN` does not

Suggested UX tone:

- explain that the merged result references `Local-LAN` but does not include its definition
- suggest changing merge order or avoiding that profile combination for now
- do not tell the user to rename keys unless there is an actual naming conflict

### `duplicate_same_value`

Two profiles define the same key with identical contents.

Suggested behavior:

- silent dedupe or low-noise info entry

### `duplicate_different_value`

Two profiles define the same key with different contents.

Suggested behavior:

- warning entry
- offer either manual rename guidance or smart rename handling

---

## Smart rename mode

### Goal

Allow advanced users to auto-resolve same-name, different-value `rule-providers` conflicts when they explicitly enable it.

### Recommended default

`OFF`

Reason:

- renaming changes config semantics
- automatic renames should be opt-in, not surprise behavior

### Recommended scope for v1

Only apply smart rename to:

- `rule-providers`

Do not expand the first implementation to every name-bearing structure in the config. Keep the blast radius small.

### Rename strategy

When smart rename is enabled and a supplementary profile conflicts with an existing `rule-providers` key:

1. rename the later profile's provider key
2. update the later profile's `RULE-SET,<name>,...` references to the new provider name
3. merge the rewritten profile result
4. record the rename in conflict logs / merge logs

### Important implementation rule

Do **not** do a blind global string replacement on the final YAML text.

Instead, do a structured rewrite inside the specific supplementary profile before it is merged into the base config.

This keeps the transformation explainable and limits accidental rewrites.

### Naming strategy

Avoid unstable generic suffixes when possible.

Less desirable:

- `Local-LAN_1`
- `Local-LAN_2`

Preferred:

- `Local-LAN__p2`
- `Local-LAN__<profile_slug>`
- `Local-LAN__<short_uid>`

Why:

- more stable across runs
- easier to debug from logs
- easier for users to map back to the source profile

If a simple numeric suffix is kept for UX reasons, it should still be derived from stable merge order and exposed in the logs.

---

## Toggle proposal

Recommended config concept:

```text
smartRenameRuleProviders: false
```

Behavior:

- `false`
  - merge non-conflicting providers normally
  - dedupe identical providers
  - report same-name/different-value conflicts in the conflict list
  - guide users to rename manually if they want both definitions active

- `true`
  - merge non-conflicting providers normally
  - dedupe identical providers
  - auto-rename later conflicting providers
  - rewrite related `RULE-SET` references in the same source profile
  - show the rename map in conflict / merge logs

---

## UX guidance

### When the issue is a real naming conflict

Friendly conflict guidance is good.

Example direction:

- detected conflicting rule-provider key: `Local-LAN`
- sources: `Profile A`, `Profile B`
- these two definitions use the same name but different contents
- suggestion: rename one provider to a more specific name, such as `Local-LAN-Office`
- remember to update matching `RULE-SET` references too

### When the issue is a missing dependency after merge

Do not push users toward rename-first guidance.

Better direction:

- merged config references `Local-LAN` but its definition is missing
- this usually means one source profile contributed the rule and the definition was not preserved in the merged result
- for now, try putting the profile that defines `Local-LAN` earlier in the merge order, or do not merge these profiles together

---

## Recommended implementation order

### Phase 1: correctness

1. merge non-conflicting `rule-providers`
2. keep identical definitions as dedupes
3. detect and log same-name/different-value conflicts
4. add regression coverage for the `Local-LAN not found` case

This phase should fix the main real-world failure without introducing auto-rewrite behavior.

### Phase 2: explainability

1. add conflict categories
2. improve conflict list wording
3. show affected key, source profiles, and suggested next action

### Phase 3: smart rename

1. add opt-in toggle
2. rewrite conflicting later provider names inside the source profile structure
3. rewrite matching `RULE-SET` references from that same source profile
4. expose rename mapping in logs and UI

---

## Regression cases to keep around

At minimum, preserve fixtures or tests for these cases:

1. **non-conflicting provider merge**
   - base has no `Local-LAN`
   - supplementary profile defines `rule-providers.Local-LAN` and matching `RULE-SET`
   - merged result should remain valid

2. **same key, same value**
   - both profiles define identical `rule-providers.Local-LAN`
   - merged result should be valid without rename

3. **same key, different value, smart rename off**
   - conflict should be reported
   - merged result should not silently rewrite semantics

4. **same key, different value, smart rename on**
   - later provider key should be rewritten
   - related `RULE-SET` references should be rewritten too
   - merged result should be valid

5. **order sensitivity check**
   - if smart rename is off, logs should make it obvious which profile won and why

---

## Short decision summary

If we only keep one thing in mind later, it is this:

- merging `rules` without merging their dependent `rule-providers` is incorrect
- non-conflicting `rule-providers` should merge by default
- same-name/different-value provider conflicts should be explicit
- auto-rename is a useful opt-in feature, not the baseline correctness fix

That is the whole game.
