# Publishing checklist

The repository is private and nothing has been published. On the day:

1. Run `tools/check_public_text.sh` and read `cargo package -p fbx-rs --list`;
   the package must contain only `src/`, the README, the changelog, the
   licences and `THIRD-PARTY.md`.
2. Re-run the release fuzz on the final source (rule: corpus seeds; each
   target runs until coverage has been flat for 10 minutes, at least 10 and
   at most 120 CPU-minutes; zero crashes) and record the receipt.
3. Final review of the README, `docs/VERIFICATION.md`, the rustdoc and the
   licences.
4. In `.github/workflows/ci.yml` enable `push` and `pull_request` triggers; in
   `release.yml` enable the `push: tags: ['v*']` trigger.
5. Add the `CARGO_REGISTRY_TOKEN` repository secret (a crates.io token scoped
   to publishing `fbx-rs`).
6. Move the `[Unreleased]` changelog entries under `## [0.1.0] - <date>`,
   check `version` in `Cargo.toml`, commit, and push the tag `v0.1.0`.
   The release workflow checks that the tag matches `Cargo.toml`, runs
   `cargo publish --dry-run`, publishes, and creates the GitHub release from
   the changelog section.
7. Make the GitHub repository public.
