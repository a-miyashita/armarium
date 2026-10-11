# ADR-0015: Publish to crates.io so that `cargo install armarium` works

- Status: Accepted
- Date: 2026-10-11

## Context

We want users to install Armarium with `cargo install armarium` (Windows; Linux is unofficial).

- `cargo install` builds the crate from the crates.io package. It does not run the Tauri CLI or `npm`.
- The frontend (`ui/`) is built by Vite. The build output must be in the package, because the package can contain only files inside the crate directory.
- Without the `custom-protocol` feature, a Tauri release build loads the dev server URL, not the embedded frontend.
- The application depends on `armarium-core`, so the core must be on crates.io too.

## Decision

1. Publish three crates: `armarium-core`, `armarium-mcp`, and `armarium` (the application in `src-tauri`).
2. Vite writes the build to `src-tauri/dist`, and `frontendDist` is `./dist`. The directory is not committed. The `include` list in `src-tauri/Cargo.toml` puts it in the package.
3. `custom-protocol` is a default feature of `armarium`. Development uses `tauri dev -- --no-default-features`.
4. The release workflow publishes the crates in order (core, mcp, application) when a `v*` tag is pushed.:
   - A `verify` job fails when the tag is not `v` + the crate version, when an `armarium-core` dependency or `src-tauri/tauri.conf.json` has another version, when the commit is not on `main`, or when `Cargo.lock` is out of date.
   - The `publish` job uses the GitHub environment `release`, which has a required reviewer. A tag alone never publishes, because crates.io versions cannot be deleted.
   - Authentication is crates.io trusted publishing (OIDC). No long-lived token is stored.
   - The job skips a crate that is already on crates.io, so a re-run continues after a partial failure.
   - `armarium` is published with `--allow-dirty`, because `cargo` treats the ignored `src-tauri/dist` as an uncommitted change.
   - `armarium` is published with `--no-verify`. The build script of Tauri writes `src-tauri/gen`, and the verification step of `cargo publish` rejects this. Instead, the job checks that the package file list has the required files, and a `smoke` job runs `cargo install armarium armarium-mcp` from crates.io on Windows after the publish.
5. CI builds the frontend before the Rust steps, because the application now embeds `src-tauri/dist`.
6. `armarium-mcp` is installed separately. The application finds it next to its own executable, so both go to the same Cargo bin directory.

## Alternatives considered

| Alternative | Evaluation |
|---|---|
| Commit the built frontend | `cargo install` works from git too, but the diffs of generated files are noisy. |
| Build the frontend in `build.rs` | Needs Node.js on every user machine. |
| Make `armarium` install `armarium-mcp` too | One crate can have only its own binaries. A second binary in the application crate would pull in the `rmcp` and `tokio` dependencies. |

## Consequences

- Users need a Rust toolchain, and the first build takes a long time.
- A crates.io release cannot be changed. A mistake needs a new version.
- `cargo build`, `cargo test`, and `cargo clippy` need `npm --prefix ui run build` first.
- The crate versions must match across the workspace and `src-tauri/tauri.conf.json`. The `verify` job checks this.
- Trusted publishing works per crate, so the first release is done by hand. Publish the three crates with a personal token (build the frontend first), set a trusted publisher on each crate (repository `a-miyashita/armarium`, workflow `release.yml`, environment `release`), and then push the tag. Before the manual publish, run `cargo package -p armarium --no-verify --allow-dirty --list` and check that `dist/index.html` is in the list. The workflow skips the published crates and runs the `smoke` job.
- One-time settings outside the repository: the `release` environment with a required reviewer, restricted to `v*` tags.

## Amendments

### 2026-10-11: LLVM is a build requirement

`cargo install armarium` failed on a machine without LLVM: `Unable to find libclang`.

- The `session` feature of `rusqlite` (used for undo, see [ADR-0006](0006-working-copy-edit-model.md)) turns on `buildtime_bindgen` in `libsqlite3-sys`. The pre-generated bindings do not have the session API, so `bindgen` runs at build time and needs `libclang`. The `bundled` feature does not remove this need.
- CI and the `smoke` job did not find this, because the GitHub runners have LLVM installed.
- The README now lists LLVM as a requirement (Windows: `winget install LLVM.LLVM` and `LIBCLANG_PATH`; Linux: `libclang-dev`).
- We could remove the requirement by not using `session`. This needs a new undo design and is not in this change.

