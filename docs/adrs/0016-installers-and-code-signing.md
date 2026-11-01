# ADR-0016: Distribute unsigned Windows installers and Linux packages

- Status: Proposed
- Date: 2026-11-01

## Context

ADR-0015 lets users install Armarium with `cargo install`. This needs a Rust toolchain. We also want to give ready-to-run files for each platform.

- Windows is the first target. Linux is second. macOS is not planned now.
- A Windows installer without a code signature triggers a SmartScreen warning.
- Since June 2023, CAs must keep the private key of a code signing certificate in hardware. A `.pfx` file in a GitHub secret is no longer possible.
- The maintainer is an individual. A code signing certificate or a cloud signing service costs money every year. The cost is too high for this project now.
- The application finds `armarium-mcp` next to its own executable, so the installers must include it as a sidecar.

## Decision

1. The release workflow builds these files:

   | OS | Files | Signing |
   |---|---|---|
   | Windows (x86_64) | NSIS `.exe` and `.msi` | None for now |
   | Linux (x86_64) | `.AppImage` and `.deb`, built on `ubuntu-22.04` | None |

2. The sidecar `armarium-mcp` is built for each OS and named `armarium-mcp-<target triple>`. The Linux job reads the triple from `rustc -vV`.
3. We do not sign the Windows installers for now. We add signing when the cost is acceptable.
   - The workflow has no signing step, and it needs no signing secrets.
   - The README tells users about the SmartScreen warning and how to check a file with `SHA256SUMS.txt`.
4. The `release` job writes `SHA256SUMS.txt` for all files and attaches everything to a draft GitHub Release. The crates.io publish job waits for this job, so a failed installer build blocks the crates.io release.
5. We do not build for macOS now. The Cargo install path still works there.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| Azure Artifact Signing | Easy to run, but individual developers are accepted only in some regions. Check this before choosing it. |
| A paid certificate with cloud signing, issued to the individual | The publisher name is the person. It costs money every year. Choose it when we decide that signing is worth the cost. |
| A free signing service for open source projects | Its approval is not guaranteed, and the publisher name is not the maintainer. Do not depend on it. |
| A `.pfx` file in a GitHub secret | Not possible with current CA rules. |
| MSIX for the Microsoft Store | The Store signs the package, so we need no certificate. But the MCP server must start from an external application, and the install path and the data location of an MSIX package can break this. It needs a test before we choose it. |

## Consequences

- Users see a SmartScreen warning when they run a Windows installer for the first time. They click "More info", then "Run anyway".
- Users check every file with `SHA256SUMS.txt`. We can add GPG signing later.
- We cannot submit the installers to the Microsoft Store as unpackaged applications until they are signed.
- The AppImage built on Ubuntu 22.04 does not run on older systems.
- Checks that need a real machine:
  - Windows: install, start, run `armarium-mcp`, and uninstall, on a clean machine.
  - Linux: start the AppImage and the deb, including the sidecar.
