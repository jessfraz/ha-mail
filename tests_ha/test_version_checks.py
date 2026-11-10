"""A slow version check cannot delay Mail counts or start a second probe."""

import asyncio
from datetime import timedelta

from aiohttp import web
from homeassistant.core import HomeAssistant
from homeassistant.helpers import entity_registry as er
from homeassistant.util import dt as dt_util
from pytest_aiohttp.plugin import AiohttpServer
from pytest_homeassistant.common import MockConfigEntry, async_fire_time_changed

from custom_components.ha_mail._app_kit.version import LOADED_VERSION
from custom_components.ha_mail.const import DOMAIN


async def test_counts_publish_while_version_endpoint_is_blocked(
    hass: HomeAssistant,
    aiohttp_server: AiohttpServer,
    socket_enabled: None,
) -> None:
    blocked = False
    blocked_reads = 0
    unread = 1
    requested_version = asyncio.Event()
    release_version = asyncio.Event()

    async def handle(request: web.Request) -> web.Response:
        nonlocal blocked_reads
        if request.path == "/v1/system":
            if blocked:
                blocked_reads += 1
                requested_version.set()
                await release_version.wait()
            return web.json_response({"version": LOADED_VERSION, "role": "integration"})
        if request.path == "/v1/summary":
            return web.json_response(
                {
                    "total": 3,
                    "unread": unread,
                    "pending_scans": 0,
                    "ready_scans": 0,
                    "storage_due": 0,
                }
            )
        if request.path == "/v1/sync":
            return web.json_response(
                {"initial_sync_complete": True, "error_code": None}
            )
        if request.path == "/v1/events/head":
            return web.json_response({"sequence": 1})
        if request.path == "/v1/events":
            return web.json_response({"items": []})
        assert request.path == "/v1/integration/presence" and request.method == "POST"
        return web.Response(status=204)

    app = web.Application()
    app.router.add_route("*", "/{path:.*}", handle)
    server = await aiohttp_server(app)
    entry = MockConfigEntry(
        domain=DOMAIN,
        data={
            "backend_url": str(server.make_url("")).rstrip("/"),
            "token": "test-integration" * 3,
        },
    )
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done(wait_background_tasks=True)
    entity = er.async_get(hass).async_get_entity_id(
        "sensor", DOMAIN, f"{entry.entry_id}_unread"
    )
    assert entity is not None and hass.states.is_state(entity, "1")

    blocked = True
    unread = 2
    async_fire_time_changed(hass, dt_util.utcnow() + timedelta(minutes=1))
    refresh = asyncio.create_task(entry.runtime_data.async_refresh())
    try:
        async with asyncio.timeout(2):
            await requested_version.wait()
            await asyncio.shield(refresh)
        assert hass.states.is_state(entity, "2")

        async with asyncio.timeout(2):
            await entry.runtime_data.async_refresh()
        assert blocked_reads == 1
        async with asyncio.timeout(2):
            assert await hass.config_entries.async_unload(entry.entry_id)
    finally:
        release_version.set()
        await refresh
        await hass.async_block_till_done(wait_background_tasks=True)
