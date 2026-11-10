"""Malformed upstream data is a coverage failure, not an empty mailbox."""

import pytest

from custom_components.ha_mail.client import InvalidBackendURL, normalize_backend_url
from custom_components.ha_mail.models import MailStatus


def test_counts_reject_boolean_negative_and_missing_fields() -> None:
    values = {
        "total": 2,
        "unread": 1,
        "pending_scans": 0,
        "ready_scans": 1,
        "storage_due": 0,
    }
    sync = {"initial_sync_complete": True, "error_code": None}
    assert MailStatus.parse(values, sync).unread == 1
    for bad in (True, -1, "1", None):
        with pytest.raises(ValueError):
            MailStatus.parse({**values, "unread": bad}, sync)
    with pytest.raises(ValueError):
        MailStatus.parse({}, sync)


@pytest.mark.parametrize(
    "origin",
    [
        "https://example.com/a",
        "https://user:password@example.com",
        "https://example.com?token=private",
        "https://example.com/#part",
        "file:///tmp/mail",
    ],
)
def test_backend_is_one_origin_without_credentials_or_redirect_target(
    origin: str,
) -> None:
    with pytest.raises(InvalidBackendURL):
        normalize_backend_url(origin)
