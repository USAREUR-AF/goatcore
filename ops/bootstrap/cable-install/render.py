#!/usr/bin/env python3
"""render.py <template> — fill a cable-install template from the environment, to stdout.

HOST, USER_NAME and HASH are required; LUKS is required only by templates that name it.
The SSH keys come from ./authorized_keys (one public key per line, # comments allowed).
"""
import os
import sys


def main() -> int:
    template = open(sys.argv[1]).read()
    keys = [k.strip() for k in open("authorized_keys") if k.strip() and not k.startswith("#")]
    if not keys:
        print("authorized_keys holds no key", file=sys.stderr)
        return 1
    values = {
        "__HOST__": os.environ["HOST"],
        "__USER__": os.environ["USER_NAME"],
        "__HASH__": os.environ["HASH"],
        "__KEYS_YAML__": "\n".join(f'      - "{k}"' for k in keys),
    }
    if "__LUKS__" in template:
        values["__LUKS__"] = os.environ["LUKS"]
    for name, value in values.items():
        template = template.replace(name, value)
    sys.stdout.write(template)
    return 0


if __name__ == "__main__":
    sys.exit(main())
