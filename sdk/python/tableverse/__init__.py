from __future__ import annotations

from pathlib import Path
from typing import Any, Optional

from .client import delete_source, list_sources, register_source, upload_source
from .exceptions import (
    SerializationError,
    ServerNotRunningError,
    SourceNotFoundError,
    TableverseError,
    UploadError,
)
from .render import render
from .serialize import to_arrow_ipc
from .server import ServerManager

__version__ = "0.1.0"
__all__ = [
    "show",
    "open",
    "connect",
    "sources",
    "remove",
    "ServerManager",
    "TableverseError",
    "ServerNotRunningError",
    "SerializationError",
    "UploadError",
    "SourceNotFoundError",
]


def show(
    data: Any,
    name: Optional[str] = None,
    height: int = 650,
    server_url: Optional[str] = None,
) -> str:
    manager = _get_manager(server_url)
    manager.ensure_running()
    ipc_bytes = to_arrow_ipc(data)
    source = upload_source(manager.base_url, ipc_bytes, name=name)
    url = f"{manager.base_url}/view/{source['id']}"
    render(url, height=height)
    return url


def open(
    path: str | Path,
    name: Optional[str] = None,
    height: int = 650,
    server_url: Optional[str] = None,
    credentials: Optional[dict[str, str]] = None,
) -> str:
    manager = _get_manager(server_url)
    manager.ensure_running()
    source = register_source(manager.base_url, str(path), name=name, credentials=credentials)
    url = f"{manager.base_url}/view/{source['id']}"
    render(url, height=height)
    return url


def connect(server_url: str) -> None:
    ServerManager.instance().connect_to(server_url)


def sources(server_url: Optional[str] = None) -> list[dict[str, Any]]:
    manager = _get_manager(server_url)
    manager.ensure_running()
    return list_sources(manager.base_url)


def remove(source_id: str, server_url: Optional[str] = None) -> None:
    manager = _get_manager(server_url)
    manager.ensure_running()
    delete_source(manager.base_url, source_id)


def _get_manager(server_url: Optional[str]) -> ServerManager:
    if server_url is not None:
        manager = ServerManager.instance()
        manager.connect_to(server_url)
        return manager
    return ServerManager.instance()
