"""The adapter consumes counts and sync health, never mail contents."""

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class MailStatus:
    """Validated public sensor data from the service."""

    total: int
    unread: int
    pending_scans: int
    ready_scans: int
    storage_due: int
    initial_sync_complete: bool
    error_code: str | None

    @classmethod
    def parse(cls, summary: object, sync: object) -> MailStatus:
        """Reject malformed data rather than making an empty-mail claim."""
        if not isinstance(summary, dict) or not isinstance(sync, dict):
            raise ValueError("Invalid Mail response")
        values: list[int] = []
        for field in ("total", "unread", "pending_scans", "ready_scans", "storage_due"):
            value = summary.get(field)
            if type(value) is not int or value < 0:
                raise ValueError("Invalid Mail counts")
            values.append(value)
        initial = sync.get("initial_sync_complete")
        code = sync.get("error_code")
        if not isinstance(initial, bool) or (
            code is not None and (not isinstance(code, str) or len(code) > 100)
        ):
            raise ValueError("Invalid Mail health")
        return cls(*values, initial, code)
