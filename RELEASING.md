# Releasing

This is the operational runbook for pagh-api (`libpagh`). The branch model is in
`BRANCHING.md`; the version/feature rules are in `README.md`. Nothing here
releases automatically except the tag-driven `release` workflow
(`.github/workflows/release.yml`).

## Version sources

| Artifact | Version source |
|---|---|
| `libpagh` SDK crate | `libpagh/Cargo.toml` (`[package].version`, currently `0.9.0`) |
| Locked dependency set | `Cargo.lock` (regenerated and committed with the bump) |

The root `Cargo.toml` is a virtual workspace manifest and carries no version.
`dev` carries the **next release** version: the first feature after a release
bumps MINOR, the first fix bumps PATCH. `main` keeps the last **released**
version. A stable tag `vX.Y.Z` must equal `libpagh/Cargo.toml` on `main`, and
`Cargo.lock` must agree — regenerate it (`cargo build --release --examples
--target riscv64gc-unknown-none-elf`, or `cargo update -p libpagh`) before
tagging.

## Pre-release (release candidate) — from `dev` only

1. Freeze `dev`: after `vX.Y.Z-rc.1`, only fixes for this release are merged
   until the release lands. Everything else waits.
2. Pre-flight on the exact `dev` commit:
   - CI green (`.github/workflows/ci.yml`, required check `ci / build`:
     `./build.sh`, the full example build and `tools/check_abi.py`);
   - `./build.sh` green (builds `libpagh` and the `hello` ELF);
   - `cargo build --release --examples --target riscv64gc-unknown-none-elf`
     green for every example;
   - `python3 tools/check_abi.py` prints `ABI check OK: …` — the contract in
     `docs/abi.md` still matches the syscall/errno/flag/struct wrappers;
   - live evidence on the released kernel (the pinned pagh-riscv 0.21.2 used by
     the current transcripts): `docs/live-spawn.txt` and
     `docs/live-canonicalize.txt` reproduced for the examples the release
     touches, plus the live transcripts in `pagh-riscv/docs/` for the wrapped
     syscalls;
   - release notes: the README sections describing the release are written;
   - version source equals `X.Y.Z` (no `-rc` in `libpagh/Cargo.toml`, lock
     updated);
   - no open blocker, no known regression in the release scope.
3. Tag and push `vX.Y.Z-rc.N`:
   ```sh
   git checkout dev && git pull --ff-only
   git tag -a vX.Y.Z-rc.1 -m "pagh-api X.Y.Z-rc.1"
   git push origin vX.Y.Z-rc.1
   ```
4. Watch the `release` workflow: it refuses the tag unless it is reachable
   from `dev` **and not** from `main`, checks that the tag base equals the
   `libpagh` version, writes the source archive
   (`pagh-api-vX.Y.Z-rc.N.tar.gz`) with a `.sha256`, and publishes a GitHub
   **Pre-release**. A rejected tag stays in the repository but produces **no**
   release entry: delete it (`git push origin :refs/tags/<tag>`) before
   retrying.
5. Test the pre-release source archive. Bugs: fix on `dev`, tag `-rc.2` (never
   move the `-rc.1` tag).

## Stable release — from `main`

1. Merge `dev` into `main` (merge commit; no code changes at this step):
   ```sh
   git checkout main && git pull --ff-only
   git merge --no-ff dev
   git push origin main
   ```
2. Wait for CI on `main` to be green.
3. Tag and push `vX.Y.Z` on the merge commit; the workflow checks that the
   commit is reachable from `main` and that the `libpagh` version matches.
4. Verify in **Releases**: the new entry is a full release with generated notes
   and the source archive (`pagh-api-vX.Y.Z.tar.gz`) plus its matching
   `.sha256`. There is no binary artifact: pagh-api ships SDK source, and
   consumers build it against their pinned toolchain. GitHub marks the newest
   stable as **Latest**.
5. `dev` continues with the next version (first change bumps
   `libpagh/Cargo.toml` and `Cargo.lock`).

## Hotfix — straight to `main`, no RC

Hotfixes are limited to **critical/high verified security findings** or a
release-blocking regression in a released version. Everything else rides `dev`.

1. Branch from `main`: `hotfix/<short-name>`.
2. PR to `main` (never to `dev`), full CI plus a targeted regression test/PoC
   for the bug. Get it reviewed.
3. Merge to `main`, then tag the PATCH release `vX.Y.Z+1` directly (no RC) and
   verify the release entry.
4. Merge `main` back into `dev` with `--no-ff`:
   ```sh
   git checkout dev && git pull --ff-only
   git merge --no-ff main
   ```
   Resolve version conflicts in favour of the larger version (dev stays ahead),
   keeping `libpagh/Cargo.toml` and `Cargo.lock` in sync. README sections merge
   additively: keep both release sections.
5. CI on `dev` must be green after the merge-back.

## Cross-repository ABI change order

When a change touches the C/ABI boundary, release in dependency order and only
then update dependants:

1. `pagh-api` / `pagh-libc` (whichever changed; if both, `pagh-api` first when
   the SDK pins the libc header).
2. `pagh-rust` / `pagh-python` (they consume `pagh-api`/`pagh-libc`).
3. `pagh-programs` (apps built against the new SDK), `pagh-repo` (published
   packages), and finally Pagh-OS / pagh-riscv images that ship the new
   runtime/ABI.
4. Update the pinned sibling versions (`Cargo.toml` path/registry deps,
   `Cargo.lock`) in the same release train, not later.

Never tag a dependant before its dependency is tagged; the release workflow
only checks branch/version, not cross-repo compatibility.

## Backfill and "Latest"

GitHub marks **Latest** by release creation time, not by version number. When
backfilling already-released versions, create tags in ascending version order
(oldest first), so the newest version is created last and becomes Latest.

## Rollback / mistakes

- Wrong tag on the wrong commit: delete the release (`gh release delete <tag>
  --yes`) and the tag (`git push origin :refs/tags/<tag>`), fix, retag.
- Workflow rejected the tag: delete the tag; no release exists to clean up.
- Bad stable release: prefer a PATCH hotfix over moving/deleting a published
  tag; if the release is minutes old and nothing depends on it, delete
  release + tag, then re-tag the fixed commit.
- Never move an existing tag; immutable tags are enforced by the tag ruleset.

## Release checklist (paste into the release PR / notes)

- [ ] `libpagh/Cargo.toml` (and `Cargo.lock`) updated and equal to the tag base
- [ ] README release sections written
- [ ] CI green on the exact commit
- [ ] Live evidence run and linked (`docs/live-spawn.txt` / `docs/live-canonicalize.txt`)
- [ ] Freeze respected (no feature merges after `-rc.1`)
- [ ] Source archive + `.sha256` downloaded and inspected
- [ ] `main`/`dev` synchronized per the flow above
- [ ] Release entry has the right type (Pre-release vs Release)