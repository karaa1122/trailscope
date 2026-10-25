# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub: **Security → Report a vulnerability** on this repository (a private security advisory). Don't open a public issue or pull request.

Include what you found, how to reproduce it, the affected version, and the impact you expect. We aim to acknowledge reports within three working days and to agree on a disclosure date with you once a fix is ready. We're happy to credit you in the advisory.

## Supported versions

Security fixes go into the latest release. Self-hosters should keep up with releases; upgrades apply migrations automatically.

## Scope

In scope: the Logbox server (ingest, API, alerts, UI), the official SDKs, the Docker image and the deployment files in this repository. Out of scope: vulnerabilities that need an already-compromised server or database, and findings in dependencies that have no impact on Logbox (please report those upstream).

How Logbox is hardened, and what operators should configure, is described in [docs/security.md](docs/security.md).
