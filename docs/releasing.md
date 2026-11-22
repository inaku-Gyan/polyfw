# Releasing

This runbook covers stable and SemVer prerelease versions on GitHub Releases and crates.io.
Prepare a version on `main`, then push its matching tag, such as `v0.2.0` or `v0.2.0-rc.1`.
GitHub Actions publishes the crate through OIDC and completes the GitHub Release.

## Publication contract

Tagged releases run these stages:

1. Validate the tag's match with `Cargo.toml` and the committed schema's `$id`, the configuration
   example's schema link, the triggering source commit and that commit's ancestry on `main`.
2. Run reusable CI, then build, smoke-test and archive all five native binary targets.
3. Generate `SHA256SUMS` and verify the packaged crate with `cargo publish --dry-run --locked`.
4. Publish to crates.io through OIDC.
5. Create the GitHub Release with `gh release create`, attaching the archives and `SHA256SUMS`.

All jobs check out the validated source commit and use the repository's pinned Rust compiler.
Same-tag runs are serialized without cancelling an active run. The workflow stops on failure;
partial publication requires [manual recovery](#recover-a-failed-run).

Prereleases go to both channels. GitHub marks them as prereleases and never sets them as `latest`.
Every stable release becomes `latest`. When publishing an older stable version, adjust `latest`
manually afterward if a newer release should retain it.

## Prepare a version

Use the development setup and checks in [Development](development.md). In particular, Rust test
fixtures need `thumbv7em-none-eabihf`.

1. Update `[package].version` in `Cargo.toml`. Use a full stable version or a SemVer prerelease,
   for example `0.2.0-rc.1` or `0.2.0-beta.1`.
   Review `MIN_SUPPORTED_CONFIG_VERSION` and `MIN_REQUIRED_CLI_VERSION` in
   `src/config/version.rs` according to the
   [configuration compatibility policy](configuration.md#configuration-compatibility).
   Keep them unchanged for compatible releases. Advance the configuration floor when older
   contracts no longer retain their meaning, and advance the authoring minimum when newer
   configurations need newer CLI capabilities. Retained compatibility needs representative
   fixtures; a package version bump alone does not justify either compatibility bump.
2. Run `cargo check` to update the root package entry in `Cargo.lock`, then `just schema` to
   regenerate `assets/polyfw.schema.json`. Its `$id` must be
   `https://raw.githubusercontent.com/inaku-Gyan/polyfw/v<VERSION>/assets/polyfw.schema.json`, including
   any prerelease suffix. Update the `#:schema` URL in the [configuration example](configuration.md#register-the-firmware-targets)
   to the same version. Review the diffs; an ordinary version bump does not require dependency
   upgrades or a new Rust pin. Checks reject a stale schema or example link.
   Set the example's `config-version` to the release being prepared and its
   `min-polyfw-version` to the conservative authoring minimum. Update the documented
   compatibility baselines if their constants changed. Existing consumer configurations do
   not need restamping merely because a compatible CLI was released.
3. Run the checks and inspect the published package contents:

   ```sh
   just check
   cargo package --list --locked --allow-dirty
   cargo publish --dry-run --locked --allow-dirty
   git diff --check
   ```

   `--allow-dirty` permits reviewing uncommitted preparation changes. The dry run builds the
   extracted crate without uploading. The actual publication uses a clean committed tree.
   Source, README, license, embedded schema, self-contained Rust tests and the `docs/` guides are
   included. The ARM examples and their `tests/demos.rs` integration test stay in the source
   checkout and are excluded together from the crate. `Cargo.toml` defines the excluded files,
   including `docs/adr/` and `docs/research/`.
   Guide and README links use relative paths for offline reading.
4. Commit the manifest, lockfile, generated schema and accompanying changes, then merge them
   into `main` through the normal review process.

## Push the tag

Complete version preparation and confirm the trusted publisher is configured. Start from the
clean, merged commit with its pinned Rust compiler and lockfile, and substitute the actual version:

```sh
git switch main
git pull --ff-only
git status --short
cargo publish --dry-run --locked

git tag -a v0.2.0 -m 'Release v0.2.0'
git push origin v0.2.0
```

`git status --short` must print nothing. For a candidate, prepare `version = "0.2.0-rc.1"` and
use `v0.2.0-rc.1` in both tag commands. Push only the intended tag.

The workflow requires the tag version to equal the package version, the live tag to resolve to
the triggering commit at validation, and that commit to be reachable from `origin/main`. Keep the
tag unchanged. Track the `Release` run in GitHub Actions. After success, check the public GitHub
assets and crates.io version; installation commands are in [README](../README.md#install).

## Assets and build targets

| Target | Build and smoke-test environment | Archive |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | Native x86_64 Ubuntu 24.04 runner | `.tar.gz` |
| `aarch64-unknown-linux-gnu` | Native ARM64 Ubuntu 24.04 runner | `.tar.gz` |
| `x86_64-apple-darwin` | Native Intel macOS runner | `.tar.gz` |
| `aarch64-apple-darwin` | Native ARM64 macOS runner | `.tar.gz` |
| `x86_64-pc-windows-msvc` | Native x86_64 Windows runner | `.zip` |

Linux binaries are built and tested on Ubuntu 24.04; compatibility with older distributions is
not guaranteed. Each built executable must report the expected `--version`, provide `--help` and
emit a schema equal to the committed `assets/polyfw.schema.json`, with the tag's version in its `$id`.
These smoke tests need no firmware project or hardware. The full
Rust suite runs on Linux through reusable CI; the five-platform binary matrix does not claim
physical probe/debugger validation.

Archives are named `polyfw-<target>.tar.gz` or `polyfw-<target>.zip`. Each contains a directory
`polyfw-<target>/` holding `polyfw` (Windows: `polyfw.exe`), `README.md`, `LICENSE` and `docs/`
with the same guides included in the crate, excluding `docs/adr/` and `docs/research/`.
The tag selects the version. Binstall metadata in `Cargo.toml` specifies this URL and executable
layout. README and the bundled guides use relative links for offline reading. The executable can
generate its matching schema; the schema file is not bundled in the binary archive. Editors can
also fetch that schema directly from its fixed `$id` URL at the release tag. Preserve existing tags
so older versions keep their original schema link; no separate Schema version is maintained.

Download `SHA256SUMS` beside the desired archive, then verify before extracting:

| Host | Check |
| --- | --- |
| Linux | `sha256sum --ignore-missing --check SHA256SUMS` |
| macOS | `shasum -a 256 <archive>` and compare its result with the manifest entry |
| Windows PowerShell | `Get-FileHash <archive> -Algorithm SHA256` and compare with the manifest entry |

The checksum manifest covers the archives downloaded from the build jobs; the binstall metadata
provides the asset URL and layout. Build artifacts remain available for 14 days for manual recovery.

## Release notes

GitHub generates notes through its default release-notes comparison rules. The release title is
the tag. Review or edit the generated notes on GitHub after publication.

## Recover a failed run

GitHub and crates.io publish separately. The workflow publishes the crate first, then creates the
GitHub Release. It does not skip an existing crate version or resume a partially uploaded release.
Before retrying a publishing job, inspect both channels to determine what already succeeded.

| Failure point | Recovery |
| --- | --- |
| CI, binary build or package dry run | Resolve a transient runner/network problem and rerun failed jobs. Source fixes require a new commit, version and tag. |
| OIDC authentication before crate upload | Check the registered owner, repository, workflow filename and empty environment, then rerun failed jobs. |
| Crate upload or availability timeout | Inspect crates.io first: upload can succeed before Cargo reports a failure. Rerun only if the version was not published; otherwise complete GitHub publication manually. |
| Existing crate version, including a local first publish | Verify its version and source manually, then complete GitHub publication without rerunning `cargo publish`. Changed source requires a new version. |
| GitHub Release creation or asset upload after crate success | Download this run's build artifacts and complete the GitHub Release manually as described below. |
| Both channels succeeded | No recovery is needed. Do not rerun publication. |

For failures before publication, use GitHub Actions **Re-run failed jobs**, or
`gh run rerun <run-id> --failed`, after the checks above.

For a crate that is already published, use GitHub CLI from a Linux checkout to recover the
GitHub Release. Replace the example run ID and tag with those of the failed run:

```sh
release_run=123456789
release_tag=v0.2.0
gh run download "$release_run" --pattern 'binary-*' --dir dist/recovery
mkdir dist/manual-release
cp dist/recovery/binary-*/* dist/manual-release/
cd dist/manual-release
sha256sum -- *.tar.gz *.zip > SHA256SUMS
gh release create "$release_tag" ./* --verify-tag --generate-notes --title "$release_tag" --latest
```

For a prerelease, replace `--latest` with `--prerelease --latest=false`. Use the archives from the
failed run so the recovered release contains its tested binaries. If a draft or public release
already exists, inspect it first: upload only missing assets with `gh release upload`, then publish
a complete draft with `gh release edit <tag> --draft=false` and the appropriate prerelease/latest flags.
Do not replace already public archives during recovery. Restore a newer stable release as `latest`
with `gh release edit <newer-tag> --latest` when completing an older stable release.

A rerun uses the old tag's workflow and source. For a source or workflow correction, merge the
fix into `main` and prepare the next version. Registry yanking or withdrawing a release is a
separate maintainer action; consult [Cargo publishing](https://doc.rust-lang.org/cargo/reference/publishing.html)
for the registry procedure.
