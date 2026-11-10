"""Load the real HA integration against the Rust service owned by the test."""

import os

import pytest
from homeassistant.auth.const import GROUP_ID_USER
from homeassistant.auth.providers import AuthProvider
from homeassistant.core import HomeAssistant
from pytest_homeassistant.common import MockConfigEntry, MockUser

from custom_components.ha_mail.const import DOMAIN


@pytest.fixture(autouse=True)
def custom_integrations(enable_custom_integrations: None) -> None:
    """Make the checked-out integration available to Core."""


@pytest.fixture
async def entry(hass: HomeAssistant, socket_enabled: None) -> MockConfigEntry:
    origin = os.environ.get("MAIL_TEST_ORIGIN")
    if not origin:
        pytest.skip("Run the Rust ha_bridge integration test for a real backend")
    entry = MockConfigEntry(
        domain=DOMAIN,
        title="Mail",
        data={"backend_url": origin, "token": "test-integration" * 3},
    )
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    return entry


@pytest.fixture
async def hass_user(hass: HomeAssistant, local_auth: AuthProvider) -> MockUser:
    """An authenticated user without administrator privileges."""
    group = await hass.auth.async_get_group(GROUP_ID_USER)
    assert group is not None
    return MockUser(groups=[group]).add_to_hass(hass)


@pytest.fixture
async def hass_user_token(hass: HomeAssistant, hass_user: MockUser) -> str:
    refresh_token = await hass.auth.async_create_refresh_token(
        hass_user, client_id="https://example.invalid"
    )
    return hass.auth.async_create_access_token(refresh_token)
