# Changesets

Version bumps and changelogs for the Node SDKs (`@trailscope/node`, `@trailscope/express`, `@trailscope/fastify`, `@trailscope/nestjs`).

When a pull request changes an SDK, add a changeset:

```bash
pnpm changeset
```

Pick the packages, the bump (patch, minor, major) and write one line for the changelog. On `main`, a "Version Packages" pull request collects the pending changesets; merging it bumps the versions. Pushing a `v*` tag then publishes the new versions to npm (see `.github/workflows/release.yml`).

The server and the Python SDK are versioned by hand (`Cargo.toml`, `sdks/python/pyproject.toml` and `sdks/python/src/trailscope/_version.py`).
