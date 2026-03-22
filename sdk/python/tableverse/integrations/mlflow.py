from __future__ import annotations

from pathlib import Path
from typing import Any, Optional


def show_artifact(
    run_id: str,
    artifact_path: str,
    server_url: Optional[str] = None,
    height: int = 650,
) -> str:
    import mlflow  # noqa: PLC0415
    import tableverse as tv  # noqa: PLC0415

    client = mlflow.MlflowClient()
    local_path = client.download_artifacts(run_id, artifact_path)
    return tv.open(local_path, server_url=server_url, height=height)


def log_and_show(
    data: Any,
    artifact_name: str = "data.parquet",
    server_url: Optional[str] = None,
    height: int = 650,
) -> str:
    import tempfile  # noqa: PLC0415

    import mlflow  # noqa: PLC0415
    import pyarrow.parquet as pq  # noqa: PLC0415
    import tableverse as tv  # noqa: PLC0415
    from tableverse.serialize import _coerce_to_table  # noqa: PLC0415

    table = _coerce_to_table(data)
    with tempfile.NamedTemporaryFile(suffix=".parquet", delete=False) as f:
        tmp_path = f.name

    pq.write_table(table, tmp_path)
    mlflow.log_artifact(tmp_path, artifact_name)
    return tv.show(data, server_url=server_url, height=height)
