from __future__ import annotations

import os


def detect_environment() -> str:
    if _in_databricks():
        return "databricks"
    if _in_colab():
        return "colab"
    if _in_jupyter():
        return "jupyter"
    return "browser"


def render(url: str, height: int = 650) -> None:
    env = detect_environment()
    if env == "databricks":
        _render_databricks(url, height)
    elif env == "colab":
        _render_colab(url, height)
    elif env == "jupyter":
        _render_jupyter(url, height)
    else:
        _render_browser(url)


def _render_jupyter(url: str, height: int) -> None:
    from IPython.display import IFrame, display  # noqa: PLC0415
    display(IFrame(url, width="100%", height=height))


def _render_databricks(url: str, height: int) -> None:
    try:
        from IPython.display import displayHTML  # noqa: PLC0415
        displayHTML(f'<iframe src="{url}" width="100%" height="{height}px" frameborder="0"></iframe>')
    except ImportError:
        _render_browser(url)


def _render_colab(url: str, height: int) -> None:
    try:
        from google.colab import output  # type: ignore[import]  # noqa: PLC0415
        output.serve_kernel_port_as_iframe(
            int(url.split(":")[-1].split("/")[0]),
            height=height,
        )
    except Exception:
        try:
            from IPython.display import IFrame, display  # noqa: PLC0415
            display(IFrame(url, width="100%", height=height))
        except ImportError:
            _render_browser(url)


def _render_browser(url: str) -> None:
    import webbrowser  # noqa: PLC0415
    webbrowser.open(url)


def _in_jupyter() -> bool:
    try:
        from IPython import get_ipython  # noqa: PLC0415
        shell = get_ipython()
        return shell is not None and "IPKernelApp" in getattr(shell, "config", {})
    except ImportError:
        return False


def _in_databricks() -> bool:
    return os.environ.get("DATABRICKS_RUNTIME_VERSION") is not None


def _in_colab() -> bool:
    try:
        import google.colab  # type: ignore[import]  # noqa: F401, PLC0415
        return True
    except ImportError:
        return False
