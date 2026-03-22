from __future__ import annotations

from pathlib import Path
from typing import Optional


def show(
    path: str | Path,
    server_url: Optional[str] = None,
    height: int = 650,
) -> str:
    import subprocess  # noqa: PLC0415
    import sys  # noqa: PLC0415

    import tableverse as tv  # noqa: PLC0415

    result = subprocess.run(
        ["dvc", "pull", str(path)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"dvc pull failed: {result.stderr}")

    return tv.open(path, server_url=server_url, height=height)


def show_current(
    path: str | Path,
    server_url: Optional[str] = None,
    height: int = 650,
) -> str:
    import tableverse as tv  # noqa: PLC0415

    resolved = Path(path)
    if not resolved.exists():
        return show(path, server_url=server_url, height=height)
    return tv.open(resolved, server_url=server_url, height=height)
