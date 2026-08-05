#!/usr/bin/env python3
"""
Guardrail: experiments/ imports nothing but the standard library.

WHY THIS IS A REAL CONSTRAINT, NOT A PREFERENCE
    AGENTS.md and experiments/README.md both state it: the prototypes are Python
    3.9+, standard library only, "so a musician with a stock Mac can run them
    against their own session with no setup".

    That constraint is the reason anyone outside this repo can reproduce a
    published number at all. The moment `import numpy` appears, "run this against
    your own project" becomes "install a toolchain first", and the contributions
    the project most needs — measurements from other people's sessions — stop
    arriving. A linter cannot see that; it needs saying explicitly.

    It is also easy to break by accident, because the import works fine on the
    machine of whoever added it.

USAGE
    python3 check_stdlib_only.py [DIR ...]     (default: experiments/)
    Needs Python 3.10+ for sys.stdlib_module_names; skips loudly on older.
"""

from __future__ import annotations

import ast
import os
import sys

DEFAULT_DIRS = ["experiments"]

# Modules that ship with CPython but are not in stdlib_module_names on every
# version, plus the prototypes themselves importing each other.
ALWAYS_OK = {"__future__", "typing_extensions"}


def local_modules(dirs) -> set:
    names = set()
    for d in dirs:
        for name in os.listdir(d) if os.path.isdir(d) else []:
            if name.endswith(".py"):
                names.add(name[:-3])
    return names


def main() -> int:
    dirs = sys.argv[1:] or DEFAULT_DIRS
    dirs = [d for d in dirs if os.path.isdir(d)]
    if not dirs:
        print("nothing to check")
        return 0

    stdlib = getattr(sys, "stdlib_module_names", None)
    if stdlib is None:
        print("::warning::sys.stdlib_module_names needs Python 3.10+ — stdlib-only check skipped")
        return 0

    allowed = set(stdlib) | ALWAYS_OK | local_modules(dirs)
    bad = []

    for d in dirs:
        for root, _dirs, files in os.walk(d):
            _dirs[:] = [x for x in _dirs if x != "__pycache__"]
            for name in sorted(files):
                if not name.endswith(".py"):
                    continue
                path = os.path.join(root, name)
                with open(path, encoding="utf-8") as fh:
                    tree = ast.parse(fh.read(), filename=path)
                for node in ast.walk(tree):
                    if isinstance(node, ast.Import):
                        mods = [(a.name, node.lineno) for a in node.names]
                    elif isinstance(node, ast.ImportFrom):
                        if node.level:      # relative import, local by definition
                            continue
                        mods = [(node.module or "", node.lineno)]
                    else:
                        continue
                    for mod, lineno in mods:
                        top = mod.split(".")[0]
                        if top and top not in allowed:
                            bad.append((path, lineno, top))

    for path, lineno, mod in bad:
        print(f"::error file={path},line={lineno}::'{mod}' is not in the standard library.")
        print(f"  {path}:{lineno}  imports '{mod}'")

    if bad:
        print(
            "\n"
            "  experiments/ is standard-library-only on purpose (AGENTS.md,\n"
            "  experiments/README.md). A stock macOS Python has to be able to run these\n"
            "  against a real session with no install step — that is what makes the\n"
            "  published numbers reproducible by someone who is not us, and reproducing\n"
            "  them on other people's material is the contribution this project most needs.\n"
            "\n"
            "  If a dependency is genuinely unavoidable, it belongs in a new directory\n"
            "  with its own requirements file, not in experiments/.\n"
        )
        return 1

    print(f"clean: {', '.join(dirs)} imports only the standard library")
    return 0


if __name__ == "__main__":
    sys.exit(main())
