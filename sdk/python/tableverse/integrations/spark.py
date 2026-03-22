from __future__ import annotations

import tempfile
import uuid
from typing import TYPE_CHECKING, Any, Optional

if TYPE_CHECKING:
    from pyspark.sql import DataFrame as SparkDataFrame

_SAMPLE_THRESHOLD_ROWS = 1_000_000
_PARQUET_THRESHOLD_ROWS = 50_000_000


def show(
    df: "SparkDataFrame",
    sample: Optional[int] = None,
    name: Optional[str] = None,
    server_url: Optional[str] = None,
    height: int = 650,
) -> str:
    import tableverse as tv  # noqa: PLC0415

    n_rows = df.count()

    if sample is not None:
        fraction = min(sample, n_rows) / max(n_rows, 1)
        arrow_table = df.sample(fraction=fraction, seed=42).toArrow()
        return tv.show(arrow_table, name=name, server_url=server_url, height=height)

    if n_rows <= _SAMPLE_THRESHOLD_ROWS:
        return tv.show(df.toArrow(), name=name, server_url=server_url, height=height)

    if n_rows <= _PARQUET_THRESHOLD_ROWS:
        fraction = _SAMPLE_THRESHOLD_ROWS / n_rows
        arrow_table = df.sample(fraction=fraction, seed=42).toArrow()
        return tv.show(
            arrow_table,
            name=f"{name} (sampled {_SAMPLE_THRESHOLD_ROWS:,})" if name else f"sampled {_SAMPLE_THRESHOLD_ROWS:,} rows",
            server_url=server_url,
            height=height,
        )

    tmp_path = f"{tempfile.gettempdir()}/tableverse_{uuid.uuid4().hex}.parquet"
    df.write.parquet(tmp_path, mode="overwrite")
    return tv.open(tmp_path, name=name, server_url=server_url, height=height)
