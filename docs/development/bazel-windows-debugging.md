# Bazel Windows Smoke Debugging

Use this guide when `.github/workflows/build-bazel.yml` fails in the `build-windows-smoke` job.

## Scope

The Windows lane builds the full binary, matching Linux and macOS (restored after the `rules_rust` 0.74 bump picked up the upstream params-files fix for E0463, see #347):

- build `:harper_bin` (alias for `//lib/harper-ui:harper`)
- test `//lib/harper-core:harper_core_test`
- verify the built `harper_bin` artifact exists
- run `:harper_bin -- --version`

It is a cross-platform smoke lane, not the full executable matrix.

## Failure Order

Read the workflow in this order:

1. `Build with Bazel`
2. `Dump harper_bin Bazel params on failure`
3. `Test harper-core with Bazel`
4. `Check harper-ui artifact`

The params dump is keyed off the actual `Build with Bazel` step outcome. If the UI build fails, the dump step is the source of truth.

## Diagnostics Already in the Workflow

The Windows build step runs Bazel with:

- `--verbose_failures`
- `-s`

If `:harper_bin` fails, the workflow also:

- dumps `harper-*.params`
- checks whether each `--extern=...` artifact exists
- checks whether each `-Ldependency=...` directory exists
- runs a direct `rustc` smoke compile against the dumped `serde` extern

These diagnostics are meant to separate:

- missing Bazel artifacts
- broken dependency search paths
- rustc extern-loading failures
- full-target-only compile failures

## How to Read the Params Dump

The important lines are:

- `--extern=<crate>=<path>`
- `-Ldependency=<path>`
- `--sysroot=<path>`
- `--target=<triple>`

Interpret them as follows:

- if one or more `--extern` files are missing, the issue is Bazel artifact materialization or target wiring
- if one or more `-Ldependency` directories are missing, the issue is dependency propagation
- if all externs and dependency directories exist, the failure is deeper than simple BUILD wiring

## How to Read the Direct `rustc` Smoke Compile

The smoke compile reuses:

- the dumped target triple
- the dumped sysroot
- one dumped extern (`serde`)
- all dumped `-Ldependency` directories

Interpret the result as follows:

- if the direct smoke compile fails with `can't find crate`, rustc cannot load externs correctly in that Windows action context
- if the direct smoke compile succeeds, the failure is specific to the full `harper_bin` compile shape rather than basic extern loading

## Upstream Repro

The previous full Windows `:harper_bin` lane was reduced to a direct `rustc` invocation:

- source file:
  - `use serde as _;`
  - `fn main() {}`
- target:
  - `x86_64-pc-windows-msvc`
- sysroot:
  - taken from the dumped `harper_bin` params file
- extern:
  - `--extern=serde=.../libserde-*.rlib`
- dependency search paths:
  - all dumped `-Ldependency=...` entries
- invocation form:
  - `rustc.exe @smoke.rustc.params`

Observed result on Windows before narrowing the smoke lane:

- all referenced files and directories exist
- direct smoke compile failed with:
  - `error[E0463]: can't find crate for 'serde'`

This matters because it removes Harper-specific compile shape from the equation. The failure reproduced with a single externed crate and is tracked upstream.

## Upstream Issue Evidence

If this needs to be re-opened upstream or compared against a newer `rules_rust` release, carry these facts:

- the dumped params file contains the expected `--extern` entries
- every checked extern artifact exists on disk
- every checked `-Ldependency` directory exists on disk
- a direct `rustc.exe @response-file` smoke compile using only `serde` failed with `E0463`
- adding the extern crate's parent directory as another `-Ldependency=...` path does not change the result

That is enough to show the problem is deeper than:

- missing BUILD deps
- missing crate-universe outputs
- obvious path nonexistence
- `harper_bin` target complexity

## Known Failure Classes

### Missing BUILD deps

Typical symptoms:

- `E0463` for first-party or third-party crates
- dumped params file does not contain the expected `--extern` entries

Expected fix area:

- `lib/harper-ui/BUILD`

### Missing artifacts despite correct params

Typical symptoms:

- dumped params file contains `--extern`
- artifact existence check reports `False`

Expected fix area:

- Bazel artifact generation
- crate-universe integration
- `cargo-bazel-lock.json` drift

### Externs exist but `rustc` still reports `E0463`

Typical symptoms:

- params file contains expected `--extern`
- existence checks are all `True`
- direct smoke compile fails similarly

Expected fix area:

- Windows-specific `rules_rust` behavior
- proc-macro loading
- rustc path resolution under the Bazel action context

### Full `harper_bin` compile fails but direct smoke compile passes

Typical symptoms:

- direct smoke compile succeeds
- `:harper_bin` still fails

Expected fix area:

- target-specific compile shape
- proc-macro interaction
- larger dependency graph behavior

## Local Reproduction

Start with the narrowest useful commands:

- `CARGO_BAZEL_REPIN=1 bazel build :harper_bin`
- `bazel aquery --include_commandline 'mnemonic("Rustc", :harper_bin)'`

Use `aquery` to compare the `harper_bin` Rustc action across platforms:

- wrapper mode
- params file path
- `--extern` set
- `-Ldependency` set
- `--sysroot`
- target triple

## Files To Update Together

When changing the Windows smoke behavior, keep these in sync:

- `.github/workflows/build-bazel.yml`
- `.github/workflows/README.md`
- this document: `docs/development/bazel-windows-debugging.md`

If the change affects generated Bazel crate wiring, also check:

- `cargo-bazel-lock.json`

## Rule of Thumb

Do not keep guessing in `lib/harper-ui/BUILD` once the params dump already proves the externs are present. At that point, add a diagnostic that narrows the Windows rustc behavior further.
