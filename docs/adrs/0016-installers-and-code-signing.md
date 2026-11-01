# ADR-0016: Distribute signed Windows installers and Linux packages

- Status: Proposed
- Date: 2026-11-01

## Context

ADR-0015 lets users install Armarium with `cargo install`. This needs a Rust toolchain. We also want to give ready-to-run files for each platform.

- Windows is the first target. Linux is second. macOS is not planned now.
- A Windows installer without a code signature triggers a SmartScreen warning.
- Since June 2023, CAs must keep the private key of a code signing certificate in hardware. A `.pfx` file in a GitHub secret is no longer possible.
- The maintainer is an individual. [SignPath Foundation](https://signpath.org/) signs open source projects for free, and it has a GitHub Actions integration. The publisher name in the signature is "SignPath Foundation", not a person.
- SignPath signs Windows files (Authenticode). It does not replace Apple signing and notarization.
- The application finds `armarium-mcp` next to its own executable, so the installers must include it as a sidecar.

## Decision

1. The release workflow builds these files:

   | OS | Files | Signing |
   |---|---|---|
   | Windows (x86_64) | NSIS `.exe` and `.msi` | Authenticode through SignPath |
   | Linux (x86_64) | `.AppImage` and `.deb`, built on `ubuntu-22.04` | None |

2. The sidecar `armarium-mcp` is built for each OS and named `armarium-mcp-<target triple>`. The Linux job reads the triple from `rustc -vV`.
3. Windows signing uses the SignPath GitHub Action, not the `signCommand` of Tauri.
   - SignPath signs a finished file in a separate, asynchronous step. `signCommand` runs inside the Tauri build and expects a synchronous command.
   - The build job uploads the installers as a flat artifact. The `sign-windows` job sends it to SignPath and waits. The `release` job uses the signed files.
   - Signing runs only for a `v*` tag, and only when the repository variable `SIGNPATH_ORGANIZATION_ID` is set. Without it, the workflow ships unsigned installers. This lets us test the pipeline before SignPath approves the project.
   - Settings: the secret `SIGNPATH_API_TOKEN`, and the variables `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, and `SIGNPATH_SIGNING_POLICY_SLUG`.
   - The action is pinned by commit SHA, because it receives the API token.
   - The release signing policy of SignPath is restricted to tag builds of this repository and asks for a manual approval. The workflow adds no separate approval.
4. The `release` job writes `SHA256SUMS.txt` for all files and attaches everything to a draft GitHub Release. The crates.io publish job waits for this job, so a failed installer build blocks the crates.io release.
5. We do not build for macOS now. The Cargo install path still works there.
6. Windows binaries inside the installers (the application and the uninstaller) are signed only if the SignPath artifact configuration lists them. The first configuration signs the installers only.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| Azure Artifact Signing | Easy to run, but individual developers are accepted only in some regions. Check this before choosing it. |
| A paid certificate with cloud signing, issued to the individual | The publisher name is the person. It costs money every year. Keep it as the fallback if SignPath Foundation declines the project. |
| `signCommand` in `tauri.conf.json` | Does not fit the asynchronous flow of SignPath. |
| A `.pfx` file in a GitHub secret | Not possible with current CA rules. |

## Consequences

- Users get a signed installer. SmartScreen can still warn until the file has reputation.
- The publisher name is "SignPath Foundation". It is not the maintainer.
- Linux files have no signature. Users check them with `SHA256SUMS.txt`. We can add GPG signing later.
- The AppImage built on Ubuntu 22.04 does not run on older systems.
- One-time work outside the repository:
  - Apply to SignPath Foundation and set up the project, the signing policy, and an artifact configuration. The artifact is a ZIP file with `*.exe` and `*.msi` at its root, and both get an Authenticode signature.
  - Add the secret and the variables above in the repository settings.
  - Check the first signed release on a clean Windows machine: install, start, run `armarium-mcp`, and uninstall.
- Linux needs a real-machine check of the AppImage and the deb, including the sidecar.
