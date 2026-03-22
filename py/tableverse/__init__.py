from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Optional, Union

__version__ = "0.1.0"
__all__ = ["show", "serve", "inspect", "Server"]


def _find_binary() -> str:
    for candidate in ("tableverse", "tv"):
        found = _which(candidate)
        if found:
            return found

    bundled = Path(__file__).parent / "_bin" / _platform_binary_name()
    if bundled.exists():
        return str(bundled)

    raise RuntimeError(
        "tableverse binary not found.\n"
        "Install via: pip install tableverse\n"
        "Or build from source: cargo install tableverse"
    )


def _which(name: str) -> Optional[str]:
    import shutil

    return shutil.which(name)


def _platform_binary_name() -> str:
    return "tableverse.exe" if sys.platform == "win32" else "tableverse"


def _is_jupyter() -> bool:
    try:
        from IPython import get_ipython

        ipy = get_ipython()
        return ipy is not None and hasattr(ipy, "kernel")
    except ImportError:
        return False


def _to_parquet(obj) -> str:
    module = type(obj).__module__

    tmp = tempfile.NamedTemporaryFile(suffix=".parquet", delete=False)
    tmp.close()
    path = tmp.name

    if module.startswith("pandas"):
        obj.to_parquet(path, index=False)
        return path

    if module.startswith("polars"):
        obj.write_parquet(path)
        return path

    try:
        import pyarrow.parquet as pq

        if hasattr(obj, "to_arrow"):
            table = obj.to_arrow()
        else:
            import pyarrow as pa

            table = pa.Table.from_pandas(obj)
        pq.write_table(table, path)
        return path
    except ImportError:
        pass

    raise TypeError(
        f"Cannot convert {type(obj).__name__} to Parquet. "
        "Supported: pandas.DataFrame, polars.DataFrame, pyarrow.Table."
    )


class Server:
    def __init__(self, process: subprocess.Popen, port: int, url: str):
        self._process = process
        self.port = port
        self.url = url

    def stop(self):
        self._process.terminate()
        try:
            self._process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self._process.kill()

    def is_running(self) -> bool:
        return self._process.poll() is None

    def __repr__(self) -> str:
        status = "running" if self.is_running() else "stopped"
        return f"Server(url={self.url!r}, status={status!r})"

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.stop()


def serve(
    source: Union[str, "os.PathLike[str]"],
    port: int = 8080,
    open_browser: bool = True,
    block: bool = False,
) -> Server:
    binary = _find_binary()
    uri = _resolve_source(source)

    cmd = [binary, "serve", uri, "--port", str(port)]
    if not open_browser:
        cmd.append("--no-open")

    process = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    url = f"http://localhost:{port}"
    server = Server(process, port, url)

    if block:
        try:
            process.wait()
        except KeyboardInterrupt:
            server.stop()

    return server


def show(
    source,
    port: int = 0,
    inline: Optional[bool] = None,
) -> Optional[Server]:
    if inline is None:
        inline = _is_jupyter()

    tmp_path: Optional[str] = None

    if isinstance(source, (str, Path)):
        uri = str(source)
    else:
        tmp_path = _to_parquet(source)
        uri = tmp_path

    if port == 0:
        port = _find_free_port()

    binary = _find_binary()
    process = subprocess.Popen(
        [binary, "serve", uri, "--port", str(port), "--no-open"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    url = f"http://localhost:{port}"
    server = Server(process, port, url)

    _wait_for_server(url, timeout=15)

    if tmp_path:
        threading.Thread(
            target=_cleanup_after_server,
            args=(process, tmp_path),
            daemon=True,
        ).start()

    if inline:
        try:
            from IPython.display import IFrame, display

            display(IFrame(src=url, width="100%", height=600))
        except ImportError:
            import webbrowser

            webbrowser.open(url)
    else:
        import webbrowser

        webbrowser.open(url)

    return server


def inspect(source) -> None:
    tmp_path: Optional[str] = None

    if isinstance(source, (str, Path)):
        uri = str(source)
    else:
        tmp_path = _to_parquet(source)
        uri = tmp_path

    subprocess.run([_find_binary(), "inspect", uri])

    if tmp_path:
        try:
            os.unlink(tmp_path)
        except OSError:
            pass


def _resolve_source(source) -> str:
    if isinstance(source, (str, Path)):
        p = Path(source)
        return str(p.resolve()) if p.exists() else str(source)
    return _to_parquet(source)


def _find_free_port() -> int:
    import socket

    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("", 0))
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        return s.getsockname()[1]


def _wait_for_server(url: str, timeout: float = 15.0):
    import urllib.error
    import urllib.request

    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            urllib.request.urlopen(f"{url}/healthz", timeout=1)
            return
        except (urllib.error.URLError, OSError):
            time.sleep(0.1)


def _cleanup_after_server(process: subprocess.Popen, tmp_path: str):
    process.wait()
    try:
        os.unlink(tmp_path)
    except OSError:
        pass


try:
    import pandas as pd

    @pd.api.extensions.register_dataframe_accessor("tv")
    class _PandasAccessor:
        def __init__(self, df):
            self._df = df

        def show(self, port: int = 0, inline: Optional[bool] = None) -> Optional[Server]:
            return show(self._df, port=port, inline=inline)

        def inspect(self) -> None:
            inspect(self._df)

except ImportError:
    pass


try:
    import polars as pl

    @pl.api.register_dataframe_namespace("tv")
    class _PolarsNamespace:
        def __init__(self, df):
            self._df = df

        def show(self, port: int = 0, inline: Optional[bool] = None) -> Optional[Server]:
            return show(self._df, port=port, inline=inline)

        def inspect(self) -> None:
            inspect(self._df)

except ImportError:
    pass
