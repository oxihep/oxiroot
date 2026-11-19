# Releasing the oxihep crates

oxiroot and its five sister repositories ([oxihist], [oxiplot], [oxifit],
[oxistats], [oxiparticle]) are not on crates.io yet. They depend on each other
through git dependencies pinned to release tags:

```toml
oxistats = { git = "https://github.com/oxihep/oxistats", tag = "v0.1.0", version = "0.1.0" }
```

`version` is there so that publishing later only means deleting `git` and `tag`.

## The dependency order

```text
oxistats, oxiparticle, oxifit-formula        (no oxihep dependencies)
oxifit       -> oxistats, oxifit-formula
oxiplot      -> [oxifit]
oxihist      -> [oxiplot], [oxifit]
oxiroot      -> oxistats, oxiparticle, oxifit, oxifit-formula, oxihist, [oxiplot]
```

`[x]` is an optional (feature) dependency.

## Rule 1: tags never move

A tag names one commit forever. To ship a fix, cut a **new** tag (`v0.1.1`),
never delete and recreate an old one. Someone's `Cargo.lock` records the commit
a tag pointed to when they built, and moving the tag makes their build
irreproducible and silently different from yours. Tags are protected on GitHub
for this reason.

## Rule 2: every repository names the same tag

When oxiroot depends on `oxistats` directly and through `oxifit`, both
manifests must name the same `oxistats` tag. Otherwise Cargo builds two copies
of oxistats, and `oxifit::StatError` is no longer `oxistats::StatError`. The
error shows up far from the cause, as a type mismatch.

The `oxihep crates resolve once` CI job fails as soon as any oxihep crate
resolves to more than one source.

## Cutting a release

1. Merge the change into the upstream repository's `main`, with CI green.
2. In that repository, bump `version` in its `Cargo.toml`. For a workspace that
   is `[workspace.package]` plus the internal `version =` entries. Add the
   release heading to its `CHANGELOG.md`, merge, and tag the merge commit:
   ```sh
   git tag -a v0.1.1 -m "oxistats 0.1.1" && git push origin v0.1.1
   ```
   Point the install instructions at the new tag in the same PR: every
   README's dependency line, and for oxiroot the `oxiroot = { git … }` lines
   in `README.md` and `docs/`, and the `cargo install --git … --tag` line for
   `oxroot`.
3. Walk **down** the dependency order above. In every repository that names
   the upstream crate, change `tag` (and `version`) to the new tag in **one PR
   per repository**, then refresh the lockfile:
   ```sh
   cargo update -p oxistats
   ```
   CI's lockstep job confirms nothing resolves twice.
4. If that dependent now also changes, it is the next upstream: repeat from
   step 2 for it.

A change to oxistats can therefore mean tagging oxistats, then oxifit (which
re-exports its error type), then bumping oxiplot, oxihist and oxiroot. Do it
in that order, in one sitting.

## The Rust toolchain

CI runs a pinned stable toolchain (`RUST_STABLE` in `.github/workflows/ci.yml`)
because `-D warnings` turns every new clippy lint into a red `main`. To move
to a new Rust:

1. Bump `RUST_STABLE` in a PR.
2. Fix what the new lints and rustfmt report in the same PR.

The minimum supported Rust version (1.100, `rust-version` in every manifest) is
checked by the `msrv` job.

## When `main` is red

- **Don't merge on red.** Branch protection requires the CI checks. If you
  must bypass it (an outage, a broken third-party action), say why in the PR.
- **Find the cause first:** your change, or the world moving? A world-moving
  example is a new toolchain, a new ROOT image in the interop job, or a yanked
  crate. Re-run the last green commit's CI to tell them apart.
- **The weekly run** (Mondays, also startable by hand) tests the latest
  stable Rust and the latest ROOT image, and fuzzes each target for two
  minutes. It never gates a PR. When it fails, bump `RUST_STABLE` or the
  pinned ROOT image in a PR that fixes what broke, or file the fuzz crash
  (its input is kept as the run's `fuzz-artifacts`).
- **Fix forward in a PR,** or revert the offending merge with `git revert`.
  Never force-push `main`.

## When a release is wrong

- Cut the next patch tag with the fix and bump dependents as above. Don't move
  or delete the bad tag; say in the CHANGELOG what was wrong with it.
- If the bad release is a security problem, note it in the CHANGELOG, and once
  the crates are on crates.io, yank the version.

[oxihist]: https://github.com/oxihep/oxihist
[oxiplot]: https://github.com/oxihep/oxiplot
[oxifit]: https://github.com/oxihep/oxifit
[oxistats]: https://github.com/oxihep/oxistats
[oxiparticle]: https://github.com/oxihep/oxiparticle
