from __future__ import annotations

from pathlib import Path
from typing import Optional


def show_artifact(
    artifact_name: str,
    version: str = "latest",
    parquet_glob: str = "*.parquet",
    server_url: Optional[str] = None,
    height: int = 650,
) -> str:
    import wandb  # noqa: PLC0415
    import tableverse as tv  # noqa: PLC0415

    artifact = wandb.use_artifact(f"{artifact_name}:{version}")
    local_dir = artifact.download()

    parquet_files = sorted(Path(local_dir).glob(parquet_glob))
    if parquet_files:
        return tv.open(parquet_files[0], name=artifact_name, server_url=server_url, height=height)

    arrow_files = sorted(Path(local_dir).glob("*.arrow")) + sorted(Path(local_dir).glob("*.ipc"))
    if arrow_files:
        return tv.open(arrow_files[0], name=artifact_name, server_url=server_url, height=height)

    raise ValueError(f"No Parquet or Arrow files found in artifact {artifact_name}:{version}")
