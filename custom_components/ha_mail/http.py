"""Fixed-destination, allowlisted bridge using Home Assistant authentication."""

import re
from urllib.parse import urlencode

from aiohttp import ClientError, web
from homeassistant.config_entries import ConfigEntryState
from homeassistant.core import HomeAssistant
from homeassistant.helpers.http import HomeAssistantView

from .client import Actor, ResponseTooLarge
from .const import DOMAIN
from .coordinator import MailCoordinator

_ID = r"[a-zA-Z0-9_-]{1,200}"
_ROUTES = (
    (re.compile(r"health|summary|system|sync"), frozenset({"GET"})),
    (re.compile(r"sync"), frozenset({"POST"})),
    (re.compile(r"items"), frozenset({"GET"})),
    (re.compile(rf"items/{_ID}(?:/(?:pdf|envelope|ocr|events))?"), frozenset({"GET"})),
    (re.compile(rf"operations/{_ID}"), frozenset({"GET"})),
    (re.compile(r"events"), frozenset({"GET"})),
)


class MailView(HomeAssistantView):
    """Never accept an upstream origin, token, or arbitrary path from a caller."""

    url = "/api/ha_mail/v1/{path:.*}"
    name = "api:ha_mail:v1"
    requires_auth = True

    def __init__(self, hass: HomeAssistant) -> None:
        self._hass = hass

    async def _proxy(self, request: web.Request, path: str) -> web.Response:
        route = next(
            (
                route
                for route in _ROUTES
                if route[0].fullmatch(path) and request.method in route[1]
            ),
            None,
        )
        if route is None:
            raise web.HTTPNotFound
        user = request["hass_user"]
        if not user.is_admin and path not in {"health", "summary", "sync"}:
            raise web.HTTPForbidden
        if request.method != "GET" and not user.is_admin:
            raise web.HTTPForbidden
        allowed = (
            {"query", "status", "archived", "unread", "limit", "page_token"}
            if path == "items"
            else {"after"}
            if path == "events" or path.endswith("/events")
            else set()
        )
        if len(request.query_string) > 4096 or request.query.keys() - allowed:
            raise web.HTTPBadRequest
        coordinator: MailCoordinator | None = None
        for entry in self._hass.config_entries.async_entries(DOMAIN):
            if entry.state is ConfigEntryState.LOADED:
                coordinator = entry.runtime_data
                break
        if coordinator is None:
            raise web.HTTPServiceUnavailable
        payload = bytearray()
        async for chunk in request.content.iter_chunked(4096):
            payload.extend(chunk)
            if len(payload) > 16 * 1024:
                raise web.HTTPRequestEntityTooLarge(
                    max_size=16 * 1024, actual_size=len(payload)
                )
        backend_path = "/health" if path == "health" else f"/v1/{path}"
        if request.query:
            backend_path += "?" + urlencode(list(request.query.items()))
        try:
            response = await coordinator.client.request(
                request.method,
                backend_path,
                body=bytes(payload) if payload else None,
                actor=Actor(user_id=user.id, admin=user.is_admin),
            )
        except (ClientError, TimeoutError, ResponseTooLarge) as err:
            raise web.HTTPBadGateway from err
        return web.Response(
            body=response.body,
            status=response.status,
            headers={
                **response.headers,
                "Cache-Control": "no-store",
                "X-Content-Type-Options": "nosniff",
            },
        )

    async def get(self, request: web.Request, path: str) -> web.Response:
        """Read an allowlisted resource."""
        return await self._proxy(request, path)

    async def post(self, request: web.Request, path: str) -> web.Response:
        """Schedule GET-only synchronization for an administrator."""
        return await self._proxy(request, path)
