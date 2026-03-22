from __future__ import annotations

from typing import TYPE_CHECKING, Any

import pyarrow as pa
import pyarrow.ipc as ipc

from .exceptions import SerializationError

if TYPE_CHECKING:
    pass


def to_arrow_ipc(data: Any) -> bytes:
    table = _coerce_to_table(data)
    sink = pa.BufferOutputStream()
    writer = ipc.new_stream(sink, table.schema)
    writer.write_table(table, max_chunksize=65536)
    writer.close()
    return sink.getvalue().to_pybytes()


def _coerce_to_table(data: Any) -> pa.Table:
    type_name = type(data).__module__ + "." + type(data).__qualname__

    if isinstance(data, pa.Table):
        return data

    if isinstance(data, pa.RecordBatch):
        return pa.Table.from_batches([data])

    if "pandas" in type_name or _has_attr(data, "to_arrow"):
        try:
            if hasattr(data, "to_arrow"):
                result = data.to_arrow()
                if isinstance(result, pa.Table):
                    return result
                if isinstance(result, pa.RecordBatch):
                    return pa.Table.from_batches([result])
        except Exception as exc:
            raise SerializationError(f"to_arrow() failed: {exc}") from exc

    if "pandas" in type_name or _has_attr(data, "to_numpy"):
        try:
            import pandas as pd  # noqa: PLC0415
            if isinstance(data, pd.DataFrame):
                return pa.Table.from_pandas(data, preserve_index=False)
        except ImportError:
            pass
        except Exception as exc:
            raise SerializationError(f"pandas conversion failed: {exc}") from exc

    if "duckdb" in type_name:
        try:
            return data.arrow()
        except Exception as exc:
            raise SerializationError(f"DuckDB arrow() failed: {exc}") from exc

    if hasattr(data, "schema") and hasattr(data, "column_names"):
        try:
            return pa.Table.from_batches(list(data.to_batches()))
        except Exception as exc:
            raise SerializationError(f"RecordBatchReader conversion failed: {exc}") from exc

    raise SerializationError(
        f"Cannot serialize {type(data).__name__} to Arrow. "
        "Supported types: pandas.DataFrame, polars.DataFrame, pyarrow.Table, "
        "duckdb.DuckDBPyRelation, pyarrow.RecordBatch"
    )


def _has_attr(obj: Any, name: str) -> bool:
    try:
        return hasattr(obj, name)
    except Exception:
        return False
