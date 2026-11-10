"""Diagnostics expose operational state, not mail or configuration."""

from typing import TypedDict

from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant

from .coordinator import MailCoordinator


class Diagnostics(TypedDict):
    loaded: bool
    last_update_success: bool | None
    initial_sync_complete: bool | None
    sync_problem: bool | None


async def async_get_config_entry_diagnostics(
    hass: HomeAssistant, entry: ConfigEntry[MailCoordinator]
) -> Diagnostics:
    """Read cached health only; never export tokens, origins, or location IDs."""
    coordinator = getattr(entry, "runtime_data", None)
    if not isinstance(coordinator, MailCoordinator):
        return {
            "loaded": False,
            "last_update_success": None,
            "initial_sync_complete": None,
            "sync_problem": None,
        }
    data = coordinator.data
    return {
        "loaded": True,
        "last_update_success": coordinator.last_update_success,
        "initial_sync_complete": data.initial_sync_complete if data else None,
        "sync_problem": data.error_code is not None if data else None,
    }
