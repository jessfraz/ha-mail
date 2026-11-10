"""Content-free mail counts for dashboards and explicitly exposed Assist entities."""

from typing import override

from homeassistant.components.sensor import SensorEntity, SensorEntityDescription
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers.entity import DeviceInfo
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from .const import DOMAIN
from .coordinator import MailCoordinator

KEYS = ("total", "unread", "pending_scans", "ready_scans", "storage_due")


async def async_setup_entry(
    hass: HomeAssistant,
    entry: ConfigEntry[MailCoordinator],
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    """Add backend-owned counts without copying any mail contents."""
    async_add_entities(
        MailCount(entry.runtime_data, entry.entry_id, key) for key in KEYS
    )


class MailCount(CoordinatorEntity[MailCoordinator], SensorEntity):
    """A count is unavailable before the first complete provider sync."""

    _attr_has_entity_name = True

    def __init__(self, coordinator: MailCoordinator, entry_id: str, key: str) -> None:
        super().__init__(coordinator)
        self.entity_description = SensorEntityDescription(
            key=key, translation_key=key, icon="mdi:mailbox-outline"
        )
        self._attr_unique_id = f"{entry_id}_{key}"
        self._attr_device_info = DeviceInfo(
            identifiers={(DOMAIN, entry_id)}, name="Mail", manufacturer="Mail"
        )
        self._update_value()

    @property
    def available(self) -> bool:
        """Partial or failed ingestion must not look like an empty mailbox."""
        return (
            super().available
            and self.coordinator.data.initial_sync_complete
            and self.coordinator.data.error_code is None
        )

    def _update_value(self) -> None:
        self._attr_native_value = getattr(
            self.coordinator.data, self.entity_description.key
        )

    @callback
    @override
    def _handle_coordinator_update(self) -> None:
        self._update_value()
        super()._handle_coordinator_update()
