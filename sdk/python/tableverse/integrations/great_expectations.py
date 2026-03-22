from __future__ import annotations

from typing import Any, Optional


def show_validation_failures(
    validation_result: Any,
    server_url: Optional[str] = None,
    height: int = 650,
) -> str:
    import pandas as pd  # noqa: PLC0415
    import tableverse as tv  # noqa: PLC0415

    failures = []
    for result in validation_result.results:
        if not result.success:
            failures.append(
                {
                    "expectation": str(result.expectation_config.expectation_type),
                    "column": result.expectation_config.kwargs.get("column", ""),
                    "partial_unexpected_list": str(
                        result.result.get("partial_unexpected_list", [])
                    ),
                    "element_count": result.result.get("element_count", 0),
                    "unexpected_count": result.result.get("unexpected_count", 0),
                }
            )

    if not failures:
        raise ValueError("No validation failures found")

    df = pd.DataFrame(failures)
    run_id = str(getattr(validation_result.meta, "run_id", "unknown"))
    return tv.show(df, name=f"GE Failures — {run_id}", server_url=server_url, height=height)


def show_batch(
    batch: Any,
    name: Optional[str] = None,
    server_url: Optional[str] = None,
    height: int = 650,
) -> str:
    import tableverse as tv  # noqa: PLC0415

    data = getattr(batch, "data", batch)
    return tv.show(data, name=name, server_url=server_url, height=height)
