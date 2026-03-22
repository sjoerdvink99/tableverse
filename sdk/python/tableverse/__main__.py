from __future__ import annotations


def main() -> None:
    import sys

    if len(sys.argv) < 2:
        print("Usage: tableverse <path> [--port PORT] [--no-open]")
        sys.exit(1)

    path = sys.argv[1]

    import subprocess  # noqa: PLC0415
    import shutil  # noqa: PLC0415

    binary = shutil.which("tableverse")
    if binary is None:
        print("tableverse binary not found", file=sys.stderr)
        sys.exit(1)

    args = [binary, "serve", path] + sys.argv[2:]
    proc = subprocess.run(args)
    sys.exit(proc.returncode)
