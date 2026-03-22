from __future__ import annotations

from typing import TYPE_CHECKING, Any, Optional

if TYPE_CHECKING:
    from airflow.utils.context import Context


class TableverseOperator:
    def __init__(
        self,
        task_id: str,
        source_path: str,
        server_url: str = "http://localhost:8080",
        name: Optional[str] = None,
        **kwargs: Any,
    ) -> None:
        try:
            from airflow.models import BaseOperator  # noqa: PLC0415
            super().__init__(task_id=task_id, **kwargs)  # type: ignore[call-arg]
        except ImportError as exc:
            raise ImportError("airflow not installed: pip install tableverse[airflow]") from exc

        self.source_path = source_path
        self.server_url = server_url
        self.name = name

    def execute(self, context: "Context") -> str:
        import tableverse as tv  # noqa: PLC0415

        name = self.name or context["task_instance"].task_id
        return tv.open(self.source_path, name=name, server_url=self.server_url)
