# Local-LAN RULE-SET merge repro

This fixture set reproduces the multi-profile merge failure where two source subscriptions are each valid on their own, but the merged result becomes invalid.

## Files

- `01-valid-inline-rule-provider.yaml`
  - standalone valid baseline: a `RULE-SET` with a matching inline `rule-provider`
- `02-invalid-missing-rule-provider.yaml`
  - standalone invalid baseline: missing `rule-providers.Local-LAN`
- `03-invalid-case-mismatch-rule-provider.yaml`
  - standalone invalid baseline: exact-name mismatch
- `10-merge-primary-valid.yaml`
  - primary profile for merge repro, valid alone, no `rule-providers`
- `11-merge-supplement-valid.yaml`
  - supplementary profile for merge repro, valid alone, defines both `rule-providers.Local-LAN` and `RULE-SET,Local-LAN,...`
- `12-expected-broken-merged-output.yaml`
  - historical invalid merged output from the pre-fix `multi_profile_merge` behavior

## Repro claim

This fixture set captures the historical breakage that used to happen in
`src-tauri/src/enhance/multi_merge.rs` before the merge fix landed.

Under the old behavior:

- the primary profile contributed all top-level fields
- supplementary profiles contributed only `proxies`, `proxy-groups`, and `rules`
- supplementary `rule-providers` were ignored

That merge order broke:

1. `10-merge-primary-valid.yaml`
2. `11-merge-supplement-valid.yaml`

The merged config keeps:

- `IP-CIDR,10.0.0.0/8,DIRECT`
- `RULE-SET,Local-LAN,DIRECT`

But drops:

- `rule-providers.Local-LAN`

mihomo then fails validation with the same class of error:

```text
rules[1] [RULE-SET,Local-LAN,DIRECT] error: rule set [Local-LAN] not found
```

## Why `rules[1]`

The supplementary profile intentionally prepends one ordinary rule before the `RULE-SET`:

```yaml
rules:
  - IP-CIDR,10.0.0.0/8,DIRECT
  - RULE-SET,Local-LAN,DIRECT
  - MATCH,DIRECT
```

After merge, `RULE-SET,Local-LAN,DIRECT` lands at index `1`, matching the screenshot-style error shape.

The Rust regression tests now `include_str!` these exact YAML files, so the docs
and the automated repro stay in sync.

## Expected manual verification

### Standalone validation

These should both validate on their own:

- `10-merge-primary-valid.yaml`
- `11-merge-supplement-valid.yaml`

### Broken merge order

Merge in this order:

1. primary = `10-merge-primary-valid.yaml`
2. supplementary = `11-merge-supplement-valid.yaml`

Expected result:

- merged config looks like `12-expected-broken-merged-output.yaml`
- validation fails with missing `Local-LAN`

### Control check

Reverse the order:

1. primary = `11-merge-supplement-valid.yaml`
2. supplementary = `10-merge-primary-valid.yaml`

Expected result:

- merged config remains valid
- because the primary profile keeps `rule-providers.Local-LAN`

## What this proves

This repro is designed to prove a specific root cause:

- the issue is not that the source subscriptions are invalid
- the issue is not that `Local-LAN` is misspelled
- the issue is that the merge layer preserves supplementary `rules` but drops supplementary `rule-providers`
