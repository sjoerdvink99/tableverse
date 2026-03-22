from __future__ import annotations


def load_ipython_extension(ipython):
    from IPython.core.magic import register_line_magic

    from . import inspect as _inspect
    from . import show

    @register_line_magic
    def tv(line):
        line = line.strip()
        if not line:
            print("Usage: %tv <dataframe_or_path>")
            return
        try:
            obj = ipython.ev(line)
        except Exception:
            obj = line
        return show(obj, inline=True)

    @register_line_magic
    def tvinspect(line):
        line = line.strip()
        if not line:
            print("Usage: %tvinspect <dataframe_or_path>")
            return
        try:
            obj = ipython.ev(line)
        except Exception:
            obj = line
        _inspect(obj)
