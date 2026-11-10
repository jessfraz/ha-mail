"""Report transport and provider-sync failures separately from mail counts."""

from typing import override

from homeassistant.components.binary_sensor import (
    BinarySensorDeviceClass,
    BinarySensorEntity,
)
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers.entity import DeviceInfo, EntityCategory
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from .const import DOMAIN
from .coordinator import MailCoordinator


async def async_setup_entry(
    hass: HomeAssistant,
    entry: ConfigEntry[MailCoordinator],
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    """A failed poll remains visible as a problem."""
    async_add_entities([MailProblem(entry.runtime_data, entry.entry_id)])


class MailProblem(CoordinatorEntity[MailCoordinator], BinarySensorEntity):
    """Health never includes raw exceptions, senders, or document data."""

    _attr_has_entity_name = True
    _attr_translation_key = "connection_problem"
    _attr_device_class = BinarySensorDeviceClass.PROBLEM
    _attr_entity_category = EntityCategory.DIAGNOSTIC

    def __init__(self, coordinator: MailCoordinator, entry_id: str) -> None:
        super().__init__(coordinator)
        self._attr_unique_id = f"{entry_id}_connection_problem"
        self._attr_device_info = DeviceInfo(
            identifiers={(DOMAIN, entry_id)}, name="Mail", manufacturer="Mail"
        )
        self._update_health()

    @property
    def available(self) -> bool:
        return True

    def _update_health(self) -> None:
        self._attr_is_on = not self.coordinator.last_update_success or (
            self.coordinator.data is not None
            and self.coordinator.data.error_code is not None
        )

    @callback
    @override
    def _handle_coordinator_update(self) -> None:
        self._update_health()
        super()._handle_coordinator_update()
