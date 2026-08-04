# Security Policy

## Reporting a vulnerability

Please report security issues privately, **not** as a public issue.

Use GitHub's [private vulnerability reporting](https://github.com/sep-lab/Wit/security/advisories/new)
on this repository. We aim to acknowledge reports within 7 days.

Please include: what the issue is, how to reproduce it, and what an attacker could
achieve. If you have a suggested fix, even better.

## Scope

Wit parses untrusted binary and XML files produced by third-party applications. That is
the primary attack surface, and we treat the following as security issues:

- **Parser crashes, hangs, or unbounded allocation** on a malformed project file. A
  malicious `.als`/`.flp`/`ProjectData` should never be able to take down or exhaust the
  host.
- **Path traversal on checkout.** Project files reference sample paths. A crafted project
  must never cause Wit to write outside the working directory.
- **XML entity expansion** (billion laughs / XXE) in `.als` parsing.
- **Content-address collisions or verification bypass** in the object store — anything
  that lets one object be substituted for another.
- **Credential or token leakage** in logs, error messages, or committed metadata.

## Explicitly out of scope

- The absence of encryption-at-rest in the local object store (not yet a feature).
- Denial of service that requires the user to deliberately point Wit at a pathological
  file of their own making.
- Anything in `experiments/` — those are research prototypes, are not part of any
  release, and should never be run on untrusted input.

## A note on your music

Wit is local-first and does not transmit your projects anywhere. If that ever changes,
it will be opt-in, documented here, and announced in the changelog — not enabled by
default.

## Supported versions

Wit is pre-release. Only the `main` branch receives fixes.
