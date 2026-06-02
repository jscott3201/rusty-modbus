#!/usr/bin/env python3
"""Guard against the 'path-only inter-crate dependency' publish failure.

`cargo publish` strips `path` and rejects any normal/build dependency on another
workspace crate that lacks a `version` requirement, with:

    all dependencies must have a version requirement specified when publishing

Because the crates publish one at a time in dependency order, a single missing
version field aborts the run mid-way and leaves a half-published registry. This
script catches the regression offline (no registry access, no first-publish
chicken-and-egg) so it can gate both the release PR and the publish job.

Exit 0 if clean, 1 (with the offending edges) otherwise.
"""

import json
import subprocess
import sys


def main() -> int:
    meta = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"]
        )
    )
    members = {pkg["name"] for pkg in meta["packages"]}
    violations = []
    for pkg in meta["packages"]:
        # publish == null  -> publishable; a list (e.g. []) -> restricted.
        if pkg.get("publish") is not None:
            continue
        for dep in pkg["dependencies"]:
            if dep["name"] not in members:
                continue  # external crate, not our concern
            if dep.get("kind") == "dev":
                continue  # dev-deps are dropped on publish; version optional
            if dep["req"] == "*":  # path-only, no version requirement
                violations.append(f"{pkg['name']} -> {dep['name']}")

    if violations:
        print(
            "Publishable crates with version-less inter-crate dependencies:",
            file=sys.stderr,
        )
        for edge in sorted(violations):
            print(f"  {edge}", file=sys.stderr)
        print(
            '\nAdd `version = "<workspace version>"` alongside `path = ...` so '
            "`cargo publish` accepts them.",
            file=sys.stderr,
        )
        return 1

    print("OK: every publishable crate pins a version on its inter-crate deps")
    return 0


if __name__ == "__main__":
    sys.exit(main())
