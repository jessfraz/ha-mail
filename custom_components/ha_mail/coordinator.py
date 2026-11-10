"""Entry-owned polling and content-free mail notifications."""

import json
import logging
from datetime import timedelta

from aiohttp import ClientError
from homeassistant.components import persistent_notification
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant
from homeassistant.helpers import issue_registry as ir
from homeassistant.helpers.storage import Store
from homeassistant.helpers.update_coordinator import DataUpdateCoordinator, UpdateFailed

from .client import MailClient, ResponseTooLarge
from .const import DOMAIN
from .models import MailStatus


class MailCoordinator(DataUpdateCoordinator[MailStatus]):
    """Do not expose senders, summaries, documents, or credentials in entities."""

    def __init__(
        self, hass: HomeAssistant, entry: ConfigEntry, client: MailClient, version: str
    ) -> None:
        super().__init__(
            hass,
            logging.getLogger(__name__),
            name="Mail",
            config_entry=entry,
            update_interval=timedelta(seconds=30),
        )
        self.version = version
        self.client = client
        self.entry = entry
        self._offset_store = Store[dict[str, int]](
            hass, 1, f"{DOMAIN}.{entry.entry_id}"
        )
        self._offset: int | None = None

    async def _json(self, path: str) -> object:
        response = await self.client.request("GET", path)
        if response.status != 200:
            raise UpdateFailed("Mail service is unavailable")
        return response.json()

    async def _async_update_data(self) -> MailStatus:
        try:
            result = MailStatus.parse(
                await self._json("/v1/summary"), await self._json("/v1/sync")
            )
            if result.error_code:
                ir.async_create_issue(
                    self.hass,
                    DOMAIN,
                    "sync_failed",
                    is_fixable=False,
                    severity=ir.IssueSeverity.WARNING,
                    translation_key="sync_failed",
                )
            else:
                ir.async_delete_issue(self.hass, DOMAIN, "sync_failed")
            await self._events()
            await self.client.request(
                "POST",
                "/v1/integration/presence",
                body=json.dumps(
                    {
                        "version": self.version,
                        "backup": {"status": "unavailable", "last_backup": None},
                    }
                ).encode(),
            )
            return result
        except (ClientError, TimeoutError, ValueError, ResponseTooLarge) as err:
            raise UpdateFailed("Mail service is unavailable") from err

    async def _events(self) -> None:
        if self._offset is None:
            saved = await self._offset_store.async_load()
            if saved is not None and type(saved.get("sequence")) is int:
                self._offset = saved["sequence"]
            else:
                head = await self._json("/v1/events/head")
                if not isinstance(head, dict) or type(head.get("sequence")) is not int:
                    raise ValueError("Invalid Mail events")
                self._offset = head["sequence"]
                await self._offset_store.async_save({"sequence": self._offset})
                return
        # Bounded pages preserve the offset and continue on the next poll.
        result = await self._json(f"/v1/events?after={self._offset}")
        if not isinstance(result, dict) or not isinstance(result.get("items"), list):
            raise ValueError("Invalid Mail events")
        messages = {
            "new_mail": "New physical mail arrived.",
            "scan_ready": "A mail scan is ready.",
            "scan_notice": "A mail scan needs attention.",
            "storage_due": "Physical mail is approaching its storage deadline.",
        }
        for event in result["items"]:
            if not isinstance(event, dict):
                raise ValueError("Invalid Mail event")
            sequence, kind = event.get("sequence"), event.get("kind")
            if type(sequence) is not int or sequence <= self._offset:
                raise ValueError("Invalid Mail event order")
            if kind in messages:
                persistent_notification.async_create(
                    self.hass,
                    messages[kind],
                    title="Mail",
                    notification_id=f"{DOMAIN}.{self.entry.entry_id}.{kind}",
                )
                self.hass.bus.async_fire(
                    "ha_mail_update", {"kind": kind, "sequence": sequence}
                )
            self._offset = sequence
        await self._offset_store.async_save({"sequence": self._offset})
