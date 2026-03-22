from __future__ import annotations


class TableverseError(Exception):
    pass


class ServerNotRunningError(TableverseError):
    pass


class SerializationError(TableverseError):
    pass


class UploadError(TableverseError):
    pass


class SourceNotFoundError(TableverseError):
    pass
