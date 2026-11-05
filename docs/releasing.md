# Releasing

Versions follow [Semantic Versioning](https://semver.org). The tag and the version in `Cargo.toml` must match, the pipeline checks this.

1. Set `version` in `Cargo.toml` and run `cargo check` so that `Cargo.lock` follows.
2. Move the entries under `[Unreleased]` in `CHANGELOG.md` into a new section `[X.Y.Z] - YYYY-MM-DD`.
3. Commit and wait for CI to pass on `main`.
4. Tag and push:

   ```bash
   git tag -a vX.Y.Z -m "vX.Y.Z"
   git push origin vX.Y.Z
   ```

The `Release` workflow then builds all packages, tests each of them on a clean system and publishes the GitHub release with the changelog section as its notes. A tag with a suffix such as `v1.2.0-rc.1` is published as a pre-release. Its notes come from the section of the base version `1.2.0`.

## What the pipeline produces

| Target | Files | Tested on |
|---|---|---|
| Windows x86-64 | `.zip` | Windows runner |
| Linux x86-64 | `.tar.gz`, `.deb`, `.rpm` | Ubuntu, Debian, Fedora, Alpine |
| Linux ARM64 | `.tar.gz`, `.deb`, `.rpm` | Emulated with QEMU |

Plus `SHA256SUMS`. In public repositories, build provenance is attested and can be checked with `gh attestation verify`.

Release binaries are built only by the pipeline. Local builds contain paths from your machine in panic messages, so do not upload them.

## Before making the repository public

Check the whole history, not just the current state:

```bash
git log --all --format='%an <%ae>%n%cn <%ce>' | sort -u
```

```bash
git log --all -p | grep -inE 'your-name|your-email|/home/your-user|C:\\\\Users\\\\your-user'
```

Then enable under Settings: private vulnerability reporting (Security), Discussions if wanted, and a branch protection rule for `main` that requires CI.
