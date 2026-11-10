"""Home Assistant adapter for Mail."""

from homeassistant.config_entries import ConfigEntry
from homeassistant.const import Platform
from homeassistant.core import HomeAssistant
from homeassistant.helpers.aiohttp_client import async_get_clientsession
from homeassistant.helpers.typing import ConfigType
from homeassistant.loader import async_get_integration

from .client import MailClient
from .const import CONF_BACKEND_URL, CONF_TOKEN
from .coordinator import MailCoordinator
from .http import MailView

PLATFORMS = [Platform.SENSOR, Platform.BINARY_SENSOR]
type MailConfigEntry = ConfigEntry[MailCoordinator]


async def async_setup(hass: HomeAssistant, config: ConfigType) -> bool:
    """Resolve the loaded entry on each request; never retain an unloaded client."""
    hass.http.register_view(MailView(hass))
    return True


async def async_setup_entry(hass: HomeAssistant, entry: MailConfigEntry) -> bool:
    """Attach one polling coordinator to this entry's lifecycle."""
    client = MailClient(
        async_get_clientsession(hass),
        entry.data[CONF_BACKEND_URL],
        entry.data[CONF_TOKEN],
    )
    integration = await async_get_integration(hass, "ha_mail")
    coordinator = MailCoordinator(hass, entry, client, str(integration.version))
    await coordinator.async_config_entry_first_refresh()
    entry.runtime_data = coordinator
    await hass.config_entries.async_forward_entry_setups(entry, PLATFORMS)
    return True


async def async_unload_entry(hass: HomeAssistant, entry: MailConfigEntry) -> bool:
    """Release entities and their entry-owned polling without closing HA's session."""
    return await hass.config_entries.async_unload_platforms(entry, PLATFORMS)
