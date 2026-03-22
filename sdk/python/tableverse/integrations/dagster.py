from __future__ import annotations

from typing import TYPE_CHECKING, Any, Optional

if TYPE_CHECKING:
    from dagster import InputContext, OutputContext


class TableverseIOManager:
    def __init__(self, server_url: str = "http://localhost:8080") -> None:
        self._server_url = server_url

    def handle_output(self, context: "OutputContext", obj: Any) -> None:
        import tableverse as tv  # noqa: PLC0415
        from dagster import MetadataValue  # noqa: PLC0415

        asset_name = context.asset_key.path[-1] if context.asset_key else context.name
        url = tv.show(obj, name=asset_name, server_url=self._server_url)

        context.add_output_metadata(
            {
                "tableverse_url": MetadataValue.url(url),
                "tableverse_rows": MetadataValue.int(_row_count(obj)),
            }
        )

    def load_input(self, context: "InputContext") -> Any:
        raise NotImplementedError("TableverseIOManager is write-only")


def _row_count(data: Any) -> int:
    try:
        return len(data)
    except Exception:
        return -1
