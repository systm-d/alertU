<!-- Security issues do not belong in a public pull request — see SECURITY.md. -->

## What and why

<!-- What does this change, and why? Link the issue if there is one: Fixes #123 -->

## How it was tested

<!-- Commands you ran, hardware you tried, or the test you added. -->

## Checklist

- [ ] `cargo fmt --all --check` passes
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` is clean
- [ ] `cargo test --workspace --all-features --locked` passes
- [ ] Added a line under `## [Unreleased]` in `CHANGELOG.md`, if this is user-visible
- [ ] Any new `unsafe` carries a safety comment and a reason (see `CONTRIBUTING.md`)
