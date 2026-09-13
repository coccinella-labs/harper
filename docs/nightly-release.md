# Nightly Release

Automated nightly builds with draft releases.

## Summary

Enable draft release creation in the nightly workflow, introduce tagged nightly releases, and keep the workflow lean by disabling benchmarks.

## Changes

### Nightly Workflow

- Enabled `create-release: true` for draft release creation
- Set `release-tag: nightly` for tagged nightly releases
- Disabled `run-benchmarks` to allow the release creation step to execute

## What It Does

1. Runs full test suite: `cargo test --all-features --workspace`
2. Builds release: `cargo build --release --all-features --workspace`
3. Creates draft release with binary artifacts

## Release Details

- Draft: ✅
- Name: `nightly-{sha}`
- Tag: `nightly-{sha}`

## Artifacts

Supports multiple binary paths:

- `target/release/harper`
- `bazel-bin/harper`
- `bazel-bin/harper-ui/harper`

## Benchmarking Strategy

Keep `run-benchmarks: false` for now:

- Nightly already takes ~17 minutes (cold cache)
- Benchmarks would add ~15-30 minutes more
- No trend tracking (output only printed to terminal)
- No automated comparison across runs

### Recommended Approach

- Run benchmarks manually when performance measurement is needed
- Introduce separate benchmarking workflow later with proper tracking tools

## Trigger

- **Schedule**: Daily at midnight UTC (`cron: '0 0 * * *'`)
- **Manual**: Workflow dispatch available

## Run Manually

```bash
gh workflow run nightly.yml
```

## Links

- [Nightly workflow](https://github.com/coccinella-labs/harper/actions?q=workflow%3Anightly)
- [Draft releases](https://github.com/coccinella-labs/harper/releases?q=nightly)
- [Reusable workflow](https://github.com/coccinella-labs/rust-nightly)
