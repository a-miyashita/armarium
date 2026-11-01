# ADR-0016: Distribute an unsigned MSI and `cargo install` for now

- Status: Accepted
- Date: 2026-11-01

## Context

ADR-0015 lets users install Armarium with `cargo install`. This needs a Rust toolchain and LLVM. We also want to give ready-to-run files.

- Windows is the first target. Linux is second. macOS is not planned now.
- A Windows installer without a code signature triggers a SmartScreen warning.
- Since June 2023, CAs must keep the private key of a code signing certificate in hardware. A `.pfx` file in a GitHub secret is no longer possible.
- The maintainer is an individual. We applied to [SignPath Foundation](https://signpath.org/), which signs open source projects for free. The application was rejected. The program requires a verifiable reputation for end-user programs, and Armarium is a new project with no wide usage yet.
- The paid options (a certificate for an individual, or a cloud signing service) cost money every year. We do not want this cost for a project with no users yet.
- The application finds `armarium-mcp` next to its own executable, so the installers must include it as a sidecar.

## Decision

1. For now, the official ways to install Armarium are:
   - the unsigned **MSI** (Windows) from GitHub Releases, and
   - `cargo install armarium` (ADR-0015).
2. The release workflow still builds more files, and attaches them to the draft release with `SHA256SUMS.txt`: the NSIS `.exe` (Windows), and the `.AppImage` and `.deb` (Linux, built on `ubuntu-22.04`). We give them as they are, without support, and the README does not list them.
3. The sidecar `armarium-mcp` is built for each OS and named `armarium-mcp-<target triple>`.
4. The README tells Windows users how to get past SmartScreen ("More info", then "Run anyway") and how to check the download with `SHA256SUMS.txt`.
5. The `sign-windows` job stays in `release.yml`, but it does nothing. It runs only when the repository variable `SIGNPATH_ORGANIZATION_ID` is set. When we get a signing service, the job is the place to connect it.
   - SignPath signs a finished file in a separate, asynchronous step, so we use a GitHub Action and not `signCommand` of Tauri.
   - The `publish` and `smoke` jobs use explicit status checks, because the implicit `success()` would skip them when `sign-windows` is skipped.
6. We do not build for macOS now.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| SignPath Foundation | Free, but rejected for the reason in Context. We can apply again when the project has more users, articles, or other signs of reputation. |
| Azure Artifact Signing | Easy to run, but individual developers are accepted only in some regions. |
| A paid certificate with cloud signing, issued to the individual | The publisher name is the person. It costs money every year. Reconsider it when there are users who need it. |
| `signCommand` in `tauri.conf.json` | Does not fit the asynchronous flow of SignPath. |
| A `.pfx` file in a GitHub secret | Not possible with current CA rules. |

## Consequences

- Windows shows a SmartScreen warning when a user runs the MSI. Some users and some companies will not install it.
- Users can check a file only by its SHA-256 checksum. Nothing proves who built it, except that the file is in the GitHub Release of this repository.
- `cargo install` has no signature problem, but it needs Rust and LLVM.
- The Linux files and the NSIS installer have no support promise, so the README does not say they are an official way to install.
- To start signing later, apply to a signing service, set the variables and the secret for the `sign-windows` job, and write a new ADR that supersedes this one.
