from __future__ import annotations

from typing import Any, Optional

import requests

from .exceptions import SourceNotFoundError, UploadError
from .server import ServerManager
from .serialize import to_arrow_ipc


def register_source(
    base_url: str,
    uri: str,
    name: Optional[str] = None,
    credentials: Optional[dict[str, str]] = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {"uri": uri}
    if name:
        payload["name"] = name
    if credentials:
        payload["credentials"] = credentials

    response = requests.post(
        f"{base_url}/api/v1/sources",
        json=payload,
        timeout=30,
    )
    if not response.ok:
        raise UploadError(f"Failed to register source: {response.text}")
    return response.json()  # type: ignore[no-any-return]


def upload_source(
    base_url: str,
    ipc_bytes: bytes,
    name: Optional[str] = None,
    is_parquet: bool = False,
) -> dict[str, Any]:
    headers: dict[str, str] = {
        "Content-Type": "application/x-parquet" if is_parquet else "application/octet-stream",
    }
    if name:
        headers["X-TV-Name"] = name

    response = requests.put(
        f"{base_url}/api/v1/upload",
        data=ipc_bytes,
        headers=headers,
        timeout=120,
    )
    if not response.ok:
        raise UploadError(f"Upload failed: {response.text}")
    return response.json()  # type: ignore[no-any-return]


def get_source(base_url: str, source_id: str) -> dict[str, Any]:
    response = requests.get(f"{base_url}/api/v1/sources/{source_id}", timeout=10)
    if response.status_code == 404:
        raise SourceNotFoundError(f"Source not found: {source_id}")
    response.raise_for_status()
    return response.json()  # type: ignore[no-any-return]


def list_sources(base_url: str) -> list[dict[str, Any]]:
    response = requests.get(f"{base_url}/api/v1/sources", timeout=10)
    response.raise_for_status()
    return response.json()  # type: ignore[no-any-return]


def delete_source(base_url: str, source_id: str) -> None:
    response = requests.delete(f"{base_url}/api/v1/sources/{source_id}", timeout=10)
    if response.status_code not in (200, 204):
        raise UploadError(f"Failed to delete source: {response.text}")
