# Folio Repository Instructions

## Scope

- Confirm the repository root, target branch, worktree status, and applicable instructions before editing. Keep changes within the authorized task and preserve existing work.
- Track build source, build configuration and scripts, test and validation code, and essential resources and documentation. Maintained configuration and rule data are source. Keep temporary files in the task directory. Deliver build packages and validation outputs through the project artifact channel when authorized.

## GitHub

- Make fixes directly in the maintained source code, configuration, or build logic that owns the behavior, including in forks. Do not add patch files, build-time source rewrites, or redundant wrappers as substitutes for direct fixes.
- Start new upstream-based work and upstream updates from the latest commit on the applicable upstream branch. Verify the branch tip remotely and record the upstream repository, branch, and full commit SHA. Use the upstream default branch unless the task specifies another branch or revision.
- Run all repository builds and compilation exclusively in GitHub Actions; do not build or compile locally. Deliver the verified artifacts from the successful workflow run.
- Prefer the verified official `gh`. On Windows, resolve the executable without running it and verify its contents with trusted installer evidence or an official release package. Use the verified absolute path and SHA-256. Use `gh api` for operations without a dedicated command. An authorized connector may perform the same read-only operation if the CLI is unavailable or unverified and repair is outside scope. Respect access denials.
- Every GitHub repository must have an active `Protect main` ruleset for its default branch. Require pull requests and block branch deletion and force pushes. Require status checks that actually run on pull requests targeting the protected branch.
- Enable and verify `delete_branch_on_merge: true` when creating or configuring a repository. Before merging a pull request, verify this setting in its source repository, including a fork, and report any limit on automatic branch deletion.
- Before writing to a shared branch, verify the current head and check for competing writers. Follow the repository rules for reviews and checks, and verify the remote commit and source branch cleanup after merging.
- Use Simplified Chinese for README and GitHub Release titles and bodies, including generated notes, unless directed otherwise. Keep official names and identifiers.
- Start README with one project-name H1 and an overview, then applicable H2s in order 功能 → 快速开始 → 配置 → 构建与测试 → 仓库结构 → 版本发布 → 许可证.
- Use `<project> <version>` for Release titles. Follow the summary with applicable H2s in order 主要变更 → 下载与校验 → 安装与升级 → 验证状态.
- Use tagged code fences, relative README and tag-pinned Release file links. Keep upstream links, known refs, credits, licenses and copyright notices.

## Build and Verification

- Target Windows 10 and 11 x64. The Rust application is in `src/rust/`, WiX sources are in `src/installer/`, and imported components are documented in `docs/THIRD-PARTY-NOTICES.md`.
- Maintain and reuse `scripts/Build-Native.ps1`, `scripts/Build-Installer.ps1`, and `scripts/Test-Installer.ps1` in the GitHub Actions workflow. Keep PowerShell scripts compatible with Windows PowerShell 5.1 and PowerShell 7, targeting Windows x64 MSVC for Rust builds and tests.
- Verify Rust builds, tests, and install, upgrade, and uninstall checks exclusively through the `Verify Folio` GitHub Actions workflow (`.github/workflows/build-installer.yml`) on pull requests to `main` or authorized workflow runs.
- Deliver MSI packages and checksums through GitHub Actions artifacts or Releases. Check the workflow's automatic release conditions and the task authorization before triggering a main-branch or manual workflow run.

## Versioned deliverables

- When a change affects a versioned deliverable, build and deliver the latest authorized repository state. Advance the project release version and use it consistently in the output filename, archive directory, package metadata and the executable or application's reported version where applicable.
- Record upstream versions and revisions separately. Check the version and checksum in the final artifact. Follow the authorized scope for publishing and deployment.

## Release asset integrity

- Deliver native usable files directly with `SHA256SUMS.txt`. Publish the same verified build outputs to GitHub Releases with identical filenames and SHA-256 hashes.
- Match the Release tag to the verified build commit. Check every published asset against the delivery files and checksum manifest. Report missing source, build or Release evidence.
