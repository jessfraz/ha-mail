"""Assertions cross the actual Core HTTP bridge and Rust service."""

from homeassistant.core import HomeAssistant
from homeassistant.helpers import issue_registry as ir
from pytest_homeassistant.common import MockConfigEntry
from pytest_homeassistant.typing import ClientSessionGenerator

from custom_components.ha_mail._app_kit.version import LOADED_VERSION
from custom_components.ha_mail.const import DOMAIN
from custom_components.ha_mail.repairs import VERSION_REPAIR


async def test_real_backend_version_matches_loaded_companion(
    hass: HomeAssistant, entry: MockConfigEntry
) -> None:
    assert str(await VERSION_REPAIR.probe(hass, entry)) == LOADED_VERSION
    assert (
        ir.async_get(hass).async_get_issue(DOMAIN, f"{entry.entry_id}_version_mismatch")
        is None
    )


async def test_admin_can_read_mail_through_real_bridge(
    hass: HomeAssistant,
    hass_client: ClientSessionGenerator,
    entry: MockConfigEntry,
) -> None:
    client = await hass_client()
    response = await client.get("/api/ha_mail/v1/items/mail-a")
    assert response.status == 200
    item = await response.json()
    assert item["sender"] == "Example Sender"
    assert item["scan_status"] == "completed"
    assert response.headers["Cache-Control"] == "no-store"
    assert (
        await client.get("/api/ha_mail/v1/items?url=https://example.com")
    ).status == 400
    assert (await client.get("/api/ha_mail/v1/locations")).status == 404
    assert (
        await client.post("/api/ha_mail/v1/items/mail-a/shred", json={})
    ).status == 404


async def test_nonadmin_cannot_forge_admin_claims_or_read_contents(
    hass_client: ClientSessionGenerator,
    hass_user_token: str,
    entry: MockConfigEntry,
) -> None:
    client = await hass_client(hass_user_token)
    headers = {"X-Mail-User-Is-Admin": "1", "X-Mail-User-Id": "forged"}
    assert (
        await client.get("/api/ha_mail/v1/items/mail-a", headers=headers)
    ).status == 403
    assert (
        await client.get("/api/ha_mail/v1/items/mail-a/pdf", headers=headers)
    ).status == 403
    assert (
        await client.post("/api/ha_mail/v1/sync", json={"idempotency_key": "forged"})
    ).status == 403
    response = await client.get("/api/ha_mail/v1/summary")
    assert response.status == 200
    assert (await response.json())["ready_scans"] == 1


async def test_anonymous_and_unloaded_entry_cannot_reach_backend(
    hass: HomeAssistant,
    hass_client: ClientSessionGenerator,
    entry: MockConfigEntry,
) -> None:
    anonymous = await hass_client(None)
    assert (await anonymous.get("/api/ha_mail/v1/summary")).status == 401
    assert await hass.config_entries.async_unload(entry.entry_id)
    client = await hass_client()
    assert (await client.get("/api/ha_mail/v1/summary")).status == 503


async def test_entities_and_diagnostics_do_not_contain_mail_contents(
    hass: HomeAssistant,
    entry: MockConfigEntry,
) -> None:
    from custom_components.ha_mail.diagnostics import async_get_config_entry_diagnostics

    states = [
        state
        for state in hass.states.async_all()
        if state.entity_id.startswith(("sensor.mail_", "binary_sensor.mail_"))
    ]
    assert len(states) == 6
    for state in states:
        assert "Example Sender" not in str(state.as_dict())
        assert "Example summary" not in str(state.as_dict())
        assert "location-a" not in str(state.as_dict())
    report = await async_get_config_entry_diagnostics(hass, entry)
    assert report == {
        "loaded": True,
        "last_update_success": True,
        "initial_sync_complete": True,
        "sync_problem": False,
    }
