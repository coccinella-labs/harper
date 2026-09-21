# GitHub Workflows

This directory contains all automation that runs in GitHub Actions for the Harper repository. Use this document as a quick reference when you need to understand what a workflow does, why it exists, or which job to update.

## Quick Reference

| Workflow | File | What it does | Key trigger(s) |
| --- | --- | --- | --- |
| Rulesets | `apply-rulesets.yml` | Applies branch ruleset definitions from `.github/rulesets/*.json` to GitHub. Edit `main-branch-protection.json` to change rules; the workflow syncs them on push. | Push to `main` touching rulesets, manual dispatch |
| Project | `add-pr-to-project.yml` | Adds opened, reopened, edited, synchronized, and ready-for-review PRs to the Harper organization project at `https://github.com/orgs/coccinella-labs/projects/10`. Project item creation runs through the Harper app token with organization project access. | PR target events |
| Milestone | `assign-pr-milestone.yml` | Assigns new PRs to `Maintenance`, `Near-term`, or `Long-term` from title and labels. Milestone edits run through the Harper app token. | PR target events |
| Auto Merge | `auto-merge.yml` | Three jobs via `coccinella-labs/auto-merge@v1`: `auto-merge` enables GitHub's built-in auto-merge (waits for `CI (ubuntu-latest)` + 1 review); `auto-merge-now` merges immediately via `BYPASS_TOKEN` bypassing ruleset checks; `cancel-auto-merge` disables queued auto-merge when the `auto-merge` label is removed. PR state changes run through the Harper app token. | `labeled`/`unlabeled`/PR events |
| Bazel | `build-bazel.yml` | Builds `:harper_bin` with Bazel on Linux and macOS, plus a scoped Windows smoke lane that builds `//lib/harper-ui:harper_ui`, runs `//lib/harper-core:harper_core_test`, checks the UI artifact, and dumps `harper_ui` params on failure. See `docs/development/bazel-windows-debugging.md` for the failure-analysis cookbook. | Push/PR to `main`, Bazel branches |
| Bazel Smoke | `bazel-smoke.yml` | Runs `bazel test //...` after merges and daily to catch dependency drift outside PRs. | Push to `main`, daily cron, manual dispatch |
| Benchmarks | `benchmarks.yml` | Runs `cargo bench` nightly and stores results as artifacts. | Daily cron, manual dispatch |
| Integration | `integration.yml` | Executes `cargo test -- --include-ignored` against real services (requires secrets). | PRs touching app code, manual dispatch (with environment input) |
| Install Script | `install-script.yml` | Dry-runs `scripts/install-harper.sh` on Linux, macOS, and Windows to validate platform-to-asset mapping. | PRs touching installer files, manual dispatch |
| Package | `package-test.yml` | Calls `coccinella-labs/release-assets` in preflight mode to build, package, and smoke-test Harper release artifacts for Linux x86_64, Linux aarch64, macOS x86_64, macOS aarch64, and Windows x86_64 without publishing them. | Tag push (`harper-*`), manual dispatch |
| Post-Merge CI | `post-auto-merge-ci.yml` | Re-runs fmt/clippy/tests on `main` after Auto Merge completes; also locks the merged PR via `gh pr lock`. PR lookups and lock actions run through the Harper app token. | Completion of Auto Merge workflow |
| PR Description | `normalize-pr-description.yml` | Rewrites `## Summary`/`## Testing` bullet-style PR bodies into a single paragraph with backtick-wrapped technical terms via `coccinella-labs/prune@v1`. Skips forks and dependabot. | PR opened/edited/ready_for_review |
| Build | `build.yml` | Builds Docker image and runs e2e tests; publishes to GHCR/Docker Hub on merge to `main`. | Push/PR to `main` touching docker files, workflow dispatch |
| CI | `ci.yml` | Runs on `main`/`develop`: validation checks, clippy, docs, unit+integration tests, release build, security audit, e2e tests. Also runs a weekly audit and coverage upload. | Push/PR to `main`/`develop`, weekly cron |
| CLA | `cla.yml` | Enforces the Contributor License Agreement via cla-bot. PR and issue writes run through the Harper app token. | PR opened/synchronized |
| CodeQL | `codeql.yml` | Performs CodeQL static analysis on Rust (security scanning). | Push/PR to `main`, weekly cron |
| Dependency Review | `dependency-review.yml` | Uses GitHub's dependency-review action on PRs. | PR events |
| Docs | `docs.yml` | Builds mkdocs documentation and deploys to GitHub Pages. | Push/PR touching docs |
| PR Title | `fix-pr-title.yml` | Rewrites PR titles into `[scope] message` format via `coccinella-labs/title@v1`. Title edits run through the Harper app token. | PR events on `main` |
| Harper | `harper-check.yml` | Publishes a lightweight app-backed check so Harper automation appears as the Harper GitHub App in PR checks and on `main`. | PR target events, push to `main` |
| Labels | `label-sync.yml` | Syncs repository labels from `config/labeler.yml` via custom script. Label writes run through the Harper app token. | Weekly cron, manual dispatch |
| Issue Labels | `issue-labels.yml` | Auto-labels issues from title/body keywords (`[fix]` to `bug`, `[feat]` to `enhancement`, plus area keywords). Only adds missing labels, never removes. Label writes run through the Harper app token. | Issue opened, edited, reopened |
| Linux Packages | `linux-packages.yml` | Generates Debian packages from published Linux release artifacts and uploads them for package-channel validation. | `harper-[0-9]*` tag push, manual dispatch |
| Lock PRs | `lock-merged-prs.yml` | Locks PRs after merge (via `gh pr lock`). Auto Merge path is handled by `post-auto-merge-ci.yml`. Lock actions run through the Harper app token. | PR closed (merged) |
| Nightly | `nightly.yml` | Uses `coccinella-labs/rust-nightly` reusable workflow: runs tests, builds release, creates prerelease with tag `nightly-{sha}`. Benchmarks disabled for faster runs. | Daily cron (midnight UTC), manual dispatch |
| Nix and Arch Packages | `nix-arch-packages.yml` | Generates Nix and Arch package manifests from published Linux release artifacts and uploads them for package-channel validation. | `harper-[0-9]*` tag push, manual dispatch |
| npm Package | `npm-package.yml` | Packs the npm binary wrapper for a published Harper release artifact without publishing it. | `harper-[0-9]*` tag push, manual dispatch |
| PR Checks | `pr-checks.yml` | Validates PR metadata. | PR events, push to `main` |
| PR Labels | `pr-labels.yml` | Auto-labels PRs via `config/labeler.yml`. Label writes run through the Harper app token. | PR target events |
| Proof | `proof.yml` | Reviews PR bodies via `coccinella-labs/proof@v1`: title convention, required sections, word count, closing-issue validity, file-anchor resolution, house style. Comments only on failure. | PR opened/edited/synchronize/ready_for_review |
| Release | `release.yml` | Creates release PRs or direct package releases for harper-core, harper-ui, harper-firmware, harper-mcp-server, harper-sandbox via `coccinella-labs/release@v1.1.1`, then publishes the installable Harper CLI binary as a dedicated `harper-*` release through `coccinella-labs/release-assets`. Release PR creation and stale overlapping release PR cleanup run through the Harper app token. Merge/direct release flows invoke `release-assets` inline, and manual `harper-*` tag pushes still trigger the same asset publishing path. The workflow passes `HARPER_UPDATE_SIGNING_KEY_PEM_B64` plus the repo-shipped updater public key into that reusable release-assets workflow. | Push to `main` touching lib dirs, tag push (`harper-*`), PR merged, manual dispatch |
| Sandbox | `sandbox.yml` | Tests the sandbox crate, profile config contract, and visible command status line on Linux, macOS, and Windows. It does not require real OS sandbox privileges in CI. | PR/push touching sandbox paths, manual dispatch |
| Second Opinion | `second-opinion.yml` | Model-backed inline PR review via `coccinella-labs/second@v1` on OpenRouter free quota. Comments only on real defects, silent otherwise. | PR opened/synchronize/ready_for_review |
| Stale | `stale-issues.yml` | Reminds on issues inactive for 30 days, adds `needs-response`, and closes them after 14 more days without a human reply. Issue comments and label changes run through the Harper app token. | Daily cron, manual dispatch, issue comments |
| Rust Fix | `rust-auto-fix.yml` | Applies automated `cargo fmt`/`clippy --fix` patches via `coccinella-labs/rust-fix@v1` when `/rust-fix` comment is confirmed with `/confirm`. | Issue comment on PRs |
| Cancel Runs | `cancel-runs.yml` | Cancels in-progress runs when `/cancel-runs` is commented on PRs via `coccinella-labs/cancel@v1`. Also triggers on `cancel-runs` label. | Issue comment on PRs, `cancel-runs` label |
| Lockfiles | `update-lockfiles.yml` | Runs `cargo update` + `CARGO_BAZEL_REPIN=true bazel build :harper_bin` (repins `cargo-bazel-lock.json`), opens PR. PR creation runs through the Harper app token. | Weekly cron (Sunday midnight UTC), manual dispatch |
| Homebrew | `update-homebrew-tap.yml` | Manually updates `coccinella-labs/homebrew-tap/Formula/harper-ai.rb` for a published `harper-*` release by downloading the release tarball, recomputing sha256, patching the formula, and opening a PR in the tap repo. Requires `HOMEBREW_TAP_TOKEN`. | Manual dispatch |
| Windows Packages | `windows-packages.yml` | Generates Scoop and winget manifests from a published Windows release artifact and uploads them for package-channel submission. | `harper-[0-9]*` tag push, manual dispatch |
| VS Code Extension | `vscode-extension.yml` | Packages the Harper Review VS Code extension and uploads the `.vsix` artifact for validation. | PRs touching extension files, manual dispatch |
| Website | `website.yml` | Builds and deploys the website bundle to GitHub Pages. | Push to `main` touching `website/**`, manual dispatch |

> **Tip:** Run `rg -n '^name:' .github/workflows` to see the canonical name shown in the Actions UI.

 > **Naming convention:** Workflow `name:` fields describe *what* the workflow does (`CI`, `Build`, `Benchmarks`). Platform/architecture granularity (`ubuntu-latest`, `macos-latest`, `windows-latest`, `aarch64`, `x86_64`) belongs in job or matrix names inside the workflow e.g. `CI (ubuntu-latest)` not in the top-level workflow name.

## Labels

| Label | Color | Purpose |
| --- | --- | --- |
| `auto-merge` | green | Enables GitHub's built-in auto-merge; waits for `CI (ubuntu-latest)` and 1 approving review before squash-merging. Remove the label to cancel a queued merge. |
| `auto-merge-now` | red | Merges immediately via `BYPASS_TOKEN`, bypassing ruleset status-check and review requirements. Use with caution. |

## Branch Ruleset

The `main` branch is protected by the **Main Branch Protection** ruleset (ID `11608827`). Its definition lives in `.github/rulesets/main-branch-protection.json` and is applied automatically by `apply-rulesets.yml` on every push that touches it. To change a rule, edit the JSON and merge to `main`.

Current rules:
- No deletion or force-push to `main`
- Squash or rebase merge only
- 1 approving review required
- `CI (ubuntu-latest)` must pass
- Bypass: OrganizationAdmin (always)

## Editing Guidelines

1. **Prefer reusable actions that are actively maintained.** We pin Bazel jobs to `bazel-contrib/setup-bazel@0.19.0` because the old `bazelbuild/setup-bazelisk` repository is archived and GitHub 404s the newer `bazelbuild/setup-bazel` path.
2. **Document non-obvious behavior.** If a workflow has unusual permissions, secrets, or environment requirements (for example, the lockfile job needing Bazel cache access), add comments in the YAML.
3. **Use the Harper app token for repo mutations.** Prefer `actions/create-github-app-token@v2` plus `HARPER_APP_ID` / `HARPER_APP_PRIVATE_KEY` when a workflow edits PRs, labels, or other repo metadata.
4. **Test locally when possible.** For shell steps that don't rely on GitHub-specific context, run them via `act` or a local script before committing.
5. **Respect required checks.** `pr-checks.yml` gates merges—update its `workflow_run` dependencies whenever you add/remove a workflow that should block PRs.
6. **Keep triggers minimal.** Avoid running heavy jobs on every push; scope `paths:` or branch filters when applicable.

## Troubleshooting

- **Action not found:** Verify the action path and version exist (e.g., `bazel-contrib/setup-bazel@0.15.0`). GitHub's error usually means the tag or repository is missing.
- **Cache warnings:** Archived actions (such as the old Bazel setup) may emit 400s from the cache API. Migrating to an actively maintained action usually resolves this.
- **`auto-merge-now` fails with ruleset violation:** Ensure `BYPASS_TOKEN` secret is set to a PAT from an org admin account with `repo` scope. The `GITHUB_TOKEN` cannot bypass rulesets.
- **Sandbox permissions (Codex/CI reproductions):** Some local sandbox sessions can mark `.git/refs/heads` with macOS provenance flags, blocking branch creation. If you see "Operation not permitted" writing inside `.git`, create/push branches from a fresh session or your host machine; the issue is environmental, not workflow-related.

Feel free to expand this file with additional details (matrix descriptions, secrets used, etc.) as workflows evolve.

## Last Updated
2026-09-11
