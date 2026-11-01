# ADR-0016: Distribute an unsigned MSI and `cargo install` for now

- Status: Accepted
- Date: 2026-11-01

## Context

ADR-0015 lets users install Armarium with `cargo install`. This needs a Rust toolchain and LLVM. We also want to give ready-to-run files.

- Windows is the first target. Linux is second. macOS is not planned now.
- A Windows installer without a code signature triggers a SmartScreen warning.
- Since June 2023, CAs must keep the private key of a code signing certificate in hardware. A `.pfx` file in a GitHub secret is no longer possible.
- The maintainer is an individual. It is hard to get a code signing certificate at a low and steady cost. A paid certificate or a cloud signing service costs money every year, and Armarium has no users yet.
- The application finds `armarium-mcp` next to its own executable, so the installers must include it as a sidecar.

## Decision

1. For now, the official ways to install Armarium are:
   - the unsigned **MSI** (Windows) from GitHub Releases, and
   - `cargo install armarium` (ADR-0015).
2. The release workflow still builds more files, and attaches them to the draft release with `SHA256SUMS.txt`: the NSIS `.exe` (Windows), and the `.AppImage` and `.deb` (Linux, built on `ubuntu-22.04`). We give them as they are, without support, and the README does not list them.
3. The sidecar `armarium-mcp` is built for each OS and named `armarium-mcp-<target triple>`.
4. The README tells Windows users how to get past SmartScreen ("More info", then "Run anyway") and how to check the download with `SHA256SUMS.txt`.
5. We do not sign the installers. We do not build for macOS.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| A paid code signing certificate with cloud signing, issued to the individual | The publisher name is the person. It costs money every year. Reconsider it when there are users who need it. |
| A `.pfx` file in a GitHub secret | Not possible with current CA rules. |

## Consequences

- Windows shows a SmartScreen warning when a user runs the MSI. Some users and some companies will not install it.
- Users can check a file only by its SHA-256 checksum. Nothing proves who built it, except that the file is in the GitHub Release of this repository.
- `cargo install` has no signature problem, but it needs Rust and LLVM.
- The Linux files and the NSIS installer have no support promise, so the README does not say they are an official way to install.
- To start signing later, choose a signing method, add a signing step to `release.yml`, and write a new ADR that supersedes this one.
