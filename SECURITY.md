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

> ⚠️ **Read this before reporting a parser bug.** The classes above describe the *product*
> parser, which does not exist yet. The only parsers in this repository today are the
> research prototypes in `experiments/`, and they **demonstrably fail several of those
> checks** — this is known, documented, and deliberately not treated as a vulnerability.
>
> The test suite already proves it, as `xfail` cases with reproductions:
> billion-laughs expansion (a 218-byte `.als` → 100,000 characters), unbounded tree
> allocation (8.8 KB → 66 MB), a quadratic varint DoS (200 KB of `0xFF` → 3.6 s), a leaked
> file descriptor, and unchecked bounds surfacing as bare `IndexError`.
>
> These are tracked in the open as
> [issue #10](https://github.com/sep-lab/Wit/issues/10). Please do not file a private
> advisory for them — you would be reporting something already public. **Fixes are very
> welcome**; the tests are written and waiting.

## Explicitly out of scope

- The absence of encryption-at-rest in the local object store (not yet a feature).
- Denial of service that requires the user to deliberately point Wit at a pathological
  file of their own making.
- Anything in `experiments/` — those are research prototypes, are not part of any
  release, and should never be run on untrusted input. See the note above.

## A note on your music

Wit is local-first and does not transmit your projects anywhere. If that ever changes,
it will be opt-in, documented here, and announced in the changelog — not enabled by
default.

## Supported versions

Wit is pre-release. Only the `main` branch receives fixes.
