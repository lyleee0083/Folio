# Folio Repository Instructions

## Scope

- Confirm the repository root, target branch, worktree status, and applicable instructions before editing. Keep changes within the authorized task and preserve existing work.
- Track build source, build configuration and scripts, test and validation code, and essential resources and documentation. Maintained configuration and rule data are source. Keep temporary files in the task directory. Deliver build packages and validation outputs through the project artifact channel when authorized.

## GitHub

- Prefer the verified official `gh`. On Windows, resolve the executable without running it and verify its contents with trusted installer evidence or an official release package. Use the verified absolute path and SHA-256. Use `gh api` for operations without a dedicated command. An authorized connector may perform the same read-only operation if the CLI is unavailable or unverified and repair is outside scope. Respect access denials.
- Every GitHub repository must have an active `Protect main` ruleset for its default branch. Require pull requests and block branch deletion and force pushes. Require status checks that actually run on pull requests targeting the protected branch.
- Enable and verify `delete_branch_on_merge: true` when creating or configuring a repository. Before merging a pull request, verify this setting in its source repository, including a fork, and report any limit on automatic branch deletion.
- Before writing to a shared branch, verify the current head and check for competing writers. Follow the repository rules for reviews and checks, and verify the remote commit and source branch cleanup after merging.
- Maintain one H1 in the primary README, with the project name and purpose. Where applicable, keep the H2 order Features, Quick Start, Configuration, Build and Test, Repository Structure, Releases, and License. Use the established language, fenced code blocks with language identifiers, and relative links for repository files.
- Name and link the upstream repository in the README, and record the source tag or commit when known. Credit imported components in the README or linked third-party notices and preserve applicable license and copyright notices.

## Build and Verification

- Target Windows 10 and 11 x64. The Rust application is in `src/rust/`, WiX sources are in `src/installer/`, and imported components are documented in `docs/THIRD-PARTY-NOTICES.md`.
- Reuse `scripts/Build-Native.ps1`, `scripts/Build-Installer.ps1`, and `scripts/Test-Installer.ps1`. Keep PowerShell scripts compatible with Windows PowerShell 5.1 and PowerShell 7. Use the Windows x64 MSVC target for Rust verification.
- Run the installer checks when changing install, upgrade, or uninstall behavior. The `Verify Folio` check covers pull requests to `main`.
- Deliver MSI packages and checksums through Actions or Releases. Check the workflow's automatic release conditions and the task authorization before triggering a main-branch or manual build.
