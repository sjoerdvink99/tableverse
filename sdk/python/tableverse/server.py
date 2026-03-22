from __future__ import annotations

import atexit
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import ClassVar, Optional

import requests

from .exceptions import ServerNotRunningError

_DEFAULT_PORT = 8080
_HEALTH_TIMEOUT = 15.0
_HEALTH_INTERVAL = 0.2


class ServerManager:
    _instance: ClassVar[Optional["ServerManager"]] = None
    _process: Optional[subprocess.Popen[bytes]]
    _port: int
    _base_url: str

    def __init__(self, port: int = _DEFAULT_PORT) -> None:
        self._process = None
        self._port = port
        self._base_url = f"http://localhost:{port}"

    @classmethod
    def instance(cls, port: int = _DEFAULT_PORT) -> "ServerManager":
        if cls._instance is None:
            cls._instance = cls(port)
        return cls._instance

    @property
    def base_url(self) -> str:
        return self._base_url

    @property
    def port(self) -> int:
        return self._port

    def is_running(self) -> bool:
        try:
            response = requests.get(f"{self._base_url}/healthz", timeout=1.0)
            return response.status_code == 200
        except Exception:
            return False

    def ensure_running(self) -> None:
        if self.is_running():
            return
        self._start()

    def _find_binary(self) -> Optional[str]:
        import shutil

        if binary := shutil.which("tableverse"):
            return binary

        candidates = [
            Path(sys.prefix) / "bin" / "tableverse",
            Path(sys.prefix) / "Scripts" / "tableverse.exe",
        ]
        for path in candidates:
            if path.exists():
                return str(path)

        return None

    def _start(self) -> None:
        binary = self._find_binary()
        if binary is None:
            raise ServerNotRunningError(
                "tableverse binary not found. Install with: pip install tableverse[server] "
                "or run 'tableverse serve --port PORT' manually and call "
                "tv.connect('http://localhost:PORT')."
            )

        self._process = subprocess.Popen(
            [binary, "serve", "--port", str(self._port), "--no-open"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        atexit.register(self.stop)
        self._wait_ready()

    def _wait_ready(self) -> None:
        deadline = time.monotonic() + _HEALTH_TIMEOUT
        while time.monotonic() < deadline:
            if self.is_running():
                return
            time.sleep(_HEALTH_INTERVAL)
        raise ServerNotRunningError(
            f"Tableverse server did not start within {_HEALTH_TIMEOUT}s on port {self._port}"
        )

    def stop(self) -> None:
        if self._process is not None:
            self._process.terminate()
            try:
                self._process.wait(timeout=5.0)
            except subprocess.TimeoutExpired:
                self._process.kill()
            self._process = None

    def connect_to(self, url: str) -> None:
        self._base_url = url.rstrip("/")
        self._port = int(url.split(":")[-1]) if ":" in url.split("//")[-1] else 80
        if not self.is_running():
            raise ServerNotRunningError(f"No Tableverse server at {url}")
