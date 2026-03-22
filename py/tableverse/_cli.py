from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

from . import _find_binary


def main():
    binary = _find_binary()
    args = sys.argv[1:]

    if not args:
        args = ["--help"]

    os.execv(binary, [binary] + args)
