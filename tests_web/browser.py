"""Exercise the real dashboard through native and prefixed ingress listeners."""

from __future__ import annotations

import base64
import hashlib
import http.client
import json
import os
import sys
import threading
from datetime import UTC, datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

from playwright.sync_api import Browser, Page, Route, expect, sync_playwright


def card_hover(page: Page) -> None:
    """The whole card opens mail, while the title remains a text action."""
    title = page.locator(".mail-card .item-open").first
    title.hover()
    expect(title).to_have_css("background-color", "rgba(0, 0, 0, 0)")
    expect(title).to_have_css("background-image", "none")
    expect(title).to_have_css("text-decoration-line", "underline")
    page.mouse.move(0, 0)


def exercise(page: Page, origin: str, *, native: bool, version: str) -> None:
    page.goto(origin)
    if native:
        expect(page.get_by_role("heading", name="Connect", exact=True)).to_be_visible()
        page.get_by_label("API token", exact=True).fill("test-owner" * 4)
        page.get_by_role("button", name="Connect", exact=True).click()
    expect(page.get_by_role("heading", name="Mail & scans", exact=True)).to_be_visible()
    expect(page.locator(".mail-card")).to_have_count(1)
    expect(page.locator(".mail-card")).to_contain_text("<img src=x onerror=alert(1)>")
    expect(page.locator(".mail-card img")).to_have_count(0)
    card_hover(page)
    envelopes: list[Route] = []
    page.route("**/v1/items/mail-a/envelope", lambda route: envelopes.append(route))
    with page.expect_request("**/v1/items/mail-a/envelope"):
        page.get_by_role("button", name="Example Sender", exact=True).click()
    expect(page.get_by_role("dialog")).to_be_visible()
    expect(page.get_by_role("dialog")).to_contain_text("<img src=x onerror=alert(1)>")
    image = page.get_by_role("img", name="Envelope scan", exact=True)
    expect(image).to_be_visible()
    box = image.bounding_box()
    assert box is not None and box["width"] > 300 and box["height"] > 150
    assert len(envelopes) == 1
    envelopes[0].fulfill(
        content_type="image/png",
        body=base64.b64decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jG8sAAAAASUVORK5CYII="
        ),
    )
    expect(image).to_have_js_property("naturalWidth", 1)
    page.unroute("**/v1/items/mail-a/envelope")
    page.get_by_role("button", name="Download scan PDF", exact=True).click()
    expect(page.locator("#detail-error")).to_be_visible()
    expect(page.get_by_role("dialog")).to_be_visible()
    page.route(
        "**/v1/items/mail-a/ocr",
        lambda route: route.fulfill(json={"pages": ["Fictional OCR text"]}),
    )
    page.get_by_role("button", name="Read OCR text", exact=True).click()
    expect(page.get_by_role("heading", name="OCR text", exact=True)).to_be_focused()
    expect(page.get_by_role("dialog")).to_contain_text("Fictional OCR text")
    page.unroute("**/v1/items/mail-a/ocr")
    page.keyboard.press("Escape")
    expect(page.get_by_role("dialog")).not_to_be_visible()
    expect(
        page.get_by_role("button", name="Example Sender", exact=True)
    ).to_be_focused()
    page.locator(".wordmark").click()
    expect(page.get_by_role("heading", name="Mail & scans", exact=True)).to_be_visible()
    expect(page.locator("#login")).not_to_be_visible()
    heading = page.get_by_role("heading", name="Mail & scans", exact=True)
    before_refresh = heading.bounding_box()
    pending: list[Route] = []
    page.route("**/v1/items?**", lambda route: pending.append(route))
    page.get_by_role("button", name="Search", exact=True).click()
    expect(page.locator("#items")).to_have_attribute("aria-busy", "true")
    after_refresh = heading.bounding_box()
    assert before_refresh and after_refresh
    assert before_refresh["y"] == after_refresh["y"]
    assert len(pending) == 1
    pending[0].continue_()
    expect(page.locator("#items")).to_have_attribute("aria-busy", "false")
    page.unroute("**/v1/items?**")
    page.get_by_label("Sender or summary", exact=True).fill("not-present")
    page.get_by_role("button", name="Search", exact=True).click()
    expect(page.locator(".mail-card")).to_have_count(0)
    expect(
        page.get_by_text("No mail matches these filters.", exact=True)
    ).to_be_visible()
    page.get_by_label("Sender or summary", exact=True).fill("")
    page.get_by_role("button", name="Search", exact=True).click()
    expect(page.locator(".mail-card")).to_have_count(1)
    page.get_by_role("button", name="Settings", exact=True).click()
    expect(page.get_by_role("heading", name="Settings", exact=True)).to_be_visible()
    expect(page.locator("#binding")).to_have_text(
        "Example Street, Example mailbox, Example City, EX, 00000"
    )
    expect(page.locator("#location-form")).not_to_be_visible()
    operation_id = "00000000-0000-4000-8000-000000000001"
    completed = False

    def sync_response(route: Route) -> None:
        if route.request.method == "POST":
            route.fulfill(json={"id": operation_id})
        else:
            route.continue_()

    page.route("**/v1/sync", sync_response)
    page.route(
        f"**/v1/operations/{operation_id}",
        lambda route: route.fulfill(
            json={"state": "completed" if completed else "running"}
        ),
    )
    page.get_by_role("button", name="Sync now", exact=True).click()
    expect(page.locator("#sync-status")).to_contain_text("Sync queued")
    expect(page.locator("#notice")).to_be_empty()
    expect(page.get_by_role("button", name="Sync now", exact=True)).to_be_disabled()
    page.get_by_role("button", name="Refresh", exact=True).click()
    expect(page.locator("#notice")).not_to_have_text("Loading mail…")
    expect(page.get_by_role("button", name="Sync now", exact=True)).to_be_disabled()
    completed = True
    expect(page.get_by_role("button", name="Sync now", exact=True)).to_be_enabled()
    expect(page.locator("#notice")).to_be_empty()
    page.get_by_role("button", name="Refresh", exact=True).click()
    expect(page.locator("#notice")).not_to_have_text("Loading mail…")
    expect(page.get_by_role("button", name="Sync now", exact=True)).to_be_enabled()
    page.unroute("**/v1/sync")
    page.unroute(f"**/v1/operations/{operation_id}")
    page.get_by_role("button", name="Mail", exact=True).click()
    page.emulate_media(color_scheme="dark")
    expect(page.get_by_role("heading", name="Mail & scans", exact=True)).to_be_visible()
    page.set_viewport_size({"width": 375, "height": 812})
    assert page.evaluate("document.documentElement.scrollWidth <= window.innerWidth")
    expect(page.locator("#app-version")).to_have_text(version)
    page.route(
        "**/v1/items?**", lambda route: route.fulfill(status=503, body="unavailable")
    )
    page.get_by_role("button", name="Search", exact=True).click()
    expect(page.locator("#notice")).not_to_have_text("Loading mail…")
    expect(page.locator("#error")).to_be_visible()
    page.unroute("**/v1/items?**")
    if native:
        page.route(
            "**/v1/system", lambda route: route.fulfill(status=401, body="expired")
        )
        page.get_by_role("button", name="Refresh", exact=True).click()
        expect(page.get_by_role("heading", name="Connect", exact=True)).to_be_visible()
        expect(page.locator("#app")).not_to_be_visible()
        page.unroute("**/v1/system")


def automatic_sync(origin: str) -> None:
    """Observe real worker ingestion, then a later durable native-store update."""
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        try:
            page = browser.new_page()
            page.clock.install()
            posts = []
            page.on(
                "request",
                lambda request: (
                    posts.append(request.url)
                    if request.method == "POST" and request.url.endswith("/v1/sync")
                    else None
                ),
            )
            page.goto(origin)
            page.get_by_label("API token", exact=True).fill("test-owner" * 4)
            page.get_by_role("button", name="Connect", exact=True).click()
            page.get_by_label("Stable location", exact=True).select_option("location-a")
            expect(
                page.get_by_role("button", name="Refresh", exact=True)
            ).to_be_enabled()
            unbound_reads: list[Route] = []

            def hold_unbound_read(route: Route) -> None:
                if not unbound_reads:
                    unbound_reads.append(route)
                else:
                    route.continue_()

            page.route("**/v1/summary", hold_unbound_read)
            with page.expect_request("**/v1/summary", timeout=5000):
                page.clock.fast_forward(31000)
            page.get_by_role("button", name="Use this mailbox", exact=True).click()
            page.get_by_role("button", name="Mail", exact=True).click()
            # No Sync now or Refresh gesture: the worker and read-only UI do both jobs.
            expect(page.locator(".mail-card")).to_have_count(50, timeout=12000)
            expect(page.locator("#mail-sync-status")).to_contain_text("Up to date")
            assert len(unbound_reads) >= 1
            page.unroute("**/v1/summary")
            page.get_by_role("button", name="Load more", exact=True).click()
            expect(page.locator(".mail-card")).to_have_count(60)
            page.get_by_label("Sender or summary", exact=True).fill("Fictional")
            page.get_by_role("button", name="Search", exact=True).click()
            expect(page.locator(".mail-card")).to_have_count(50)
            page.get_by_role("button", name="Load more", exact=True).click()
            expect(page.locator(".mail-card")).to_have_count(60)
            search = page.get_by_label("Sender or summary", exact=True)
            search.fill("Unsubmitted draft")
            search.focus()
            print("update-fictional-mail", flush=True)
            assert sys.stdin.readline().strip() == "updated"
            with page.expect_response("**/v1/summary"):
                page.clock.fast_forward(31000)
            expect(
                page.get_by_role("button", name="Refresh", exact=True)
            ).to_be_enabled()
            expect(page.locator(".mail-card")).to_have_count(60)
            expect(page.locator("#items")).to_contain_text(
                "Updated by the next server sync"
            )
            expect(search).to_have_value("Unsubmitted draft")
            expect(search).to_be_focused()
            target = page.get_by_role("button", name="Fictional sender 059", exact=True)
            target.click()
            dialog = page.get_by_role("dialog")
            expect(dialog).to_contain_text("Updated by the next server sync")
            page.get_by_role("button", name="Close mail details", exact=True).focus()
            with page.expect_response("**/v1/summary"):
                page.clock.fast_forward(31000)
            expect(
                page.get_by_role("button", name="Refresh", exact=True)
            ).to_be_enabled()
            expect(dialog).to_be_visible()
            expect(
                page.get_by_role("button", name="Close mail details", exact=True)
            ).to_be_focused()
            page.keyboard.press("Escape")
            expect(target).to_be_focused()
            assert posts == []
            pending = []

            def hold_once(route: Route) -> None:
                if not pending:
                    pending.append(route)
                else:
                    route.continue_()

            page.route("**/v1/summary", hold_once)
            with page.expect_request("**/v1/summary"):
                page.clock.fast_forward(31000)
            expect(page.locator(".mail-card")).to_have_count(60)
            assert len(pending) == 1
            page.clock.fast_forward(10000)
            assert len(pending) == 1
            pending[0].continue_()
            page.unroute("**/v1/summary")
            expect(
                page.get_by_role("button", name="Refresh", exact=True)
            ).to_be_enabled()
            page.route(
                "**/v1/summary",
                lambda route: route.fulfill(status=503, body="unavailable"),
            )
            with page.expect_response("**/v1/summary"):
                page.clock.fast_forward(31000)
            expect(page.locator("#notice")).to_contain_text("Showing saved results.")
            expect(page.locator(".mail-card")).to_have_count(60)
            page.unroute("**/v1/summary")
            expect(
                page.get_by_role("button", name="Refresh", exact=True)
            ).to_be_enabled()
            with page.expect_response("**/v1/summary"):
                page.clock.fast_forward(31000)
            expect(
                page.get_by_role("button", name="Refresh", exact=True)
            ).to_be_enabled()
            expect(page.locator("#notice")).to_be_empty()
            page.route(
                "**/v1/system", lambda route: route.fulfill(status=401, body="expired")
            )
            with page.expect_request("**/v1/system"):
                page.clock.fast_forward(31000)
            expect(
                page.get_by_role("heading", name="Connect", exact=True)
            ).to_be_visible()
            expect(page.locator("#app")).not_to_be_visible()
            assert posts == []
        finally:
            browser.close()


def onboarding_discovery(origin: str, scenario: str) -> None:
    """Use real unbound native state and a controlled provider lookup boundary."""
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        try:
            page = browser.new_page()
            page.clock.install()
            lookups: list[Route] = []

            def locations(route: Route) -> None:
                lookups.append(route)
                if len(lookups) > 1:
                    route.continue_()
                elif scenario == "empty":
                    route.fulfill(json={"items": [], "next_page_token": None})
                elif scenario == "unavailable":
                    route.fulfill(status=503, body="unavailable")

            page.route("**/v1/locations", locations)
            page.goto(origin)
            page.get_by_label("API token", exact=True).fill("test-owner" * 4)
            with page.expect_request("**/v1/locations"):
                page.get_by_role("button", name="Connect", exact=True).click()
            expect(
                page.get_by_role("heading", name="Settings", exact=True)
            ).to_be_visible()
            assert len(lookups) == 1
            refresh = page.get_by_role("button", name="Refresh", exact=True)
            if scenario == "stalled":
                # The actual fetch must fail when its owned20s lifetime expires.
                with page.expect_event(
                    "requestfailed",
                    predicate=lambda request: request.url.endswith("/v1/locations"),
                    timeout=5000,
                ):
                    page.clock.fast_forward(21000)
                expect(refresh).to_be_enabled()
                expect(page.locator("#error")).to_be_visible()
            else:
                expect(refresh).to_be_enabled()
                for _ in range(2):
                    with page.expect_response("**/v1/summary"):
                        page.clock.fast_forward(31000)
                    expect(refresh).to_be_enabled()
                    assert len(lookups) == 1, (
                        "Background local refresh repeated "
                        "provider-facing location discovery"
                    )
            # Refresh is an explicit retry, including after empty,503 or timeout.
            with page.expect_request("**/v1/locations"):
                refresh.click()
            expect(
                page.get_by_label("Stable location", exact=True).locator("option")
            ).to_have_count(2)
            expect(refresh).to_be_enabled()
            assert len(lookups) == 2
            assert page.get_by_label("Stable location", exact=True).input_value() == ""
        finally:
            browser.close()


def visual_review(browser: Browser, origin: str, output: Path) -> None:
    """Render the actual UI with fictional mail at its API boundary."""
    output.mkdir(parents=True, exist_ok=True)
    fixture_response = browser.new_context()
    try:
        response = fixture_response.request.get(
            origin + "/v1/items/mail-a",
            headers={"Authorization": "Bearer " + "test-owner" * 4},
        )
        assert response.ok
        template = response.json()
    finally:
        fixture_response.close()
    now = datetime(2026, 11, 11, 12, tzinfo=UTC)
    rows = []
    for index, (sender, summary, scan, deadline, shredded) in enumerate(
        [
            (
                "Maple Studio",
                "Your fabric samples are ready to review.",
                "completed",
                "2023-06-01T12:00:00Z",
                False,
            ),
            (
                "Meadow Workshop",
                "A guide to caring for your new tools.",
                "completed",
                "2026-11-13T12:00:00Z",
                False,
            ),
            (
                "Marigold Water",
                "Your monthly statement is ready.",
                "completed",
                None,
                False,
            ),
            ("Northstar Insurance", None, "processing", "2026-12-01T12:00:00Z", False),
            ("Cedar Library", None, "none", None, False),
            (
                "Harbor Press",
                "The autumn catalog.",
                "completed",
                "2023-06-01T12:00:00Z",
                True,
            ),
        ]
    ):
        item = dict(template)
        item.update(
            id=f"fictional-{index}",
            sender=sender,
            summary=summary,
            scan_status=scan,
            received_at="2026-11-09T12:00:00Z",
            clear_at=deadline,
            shred_status="completed" if shredded else None,
            has_envelope=False,
            has_ocr=False,
            has_pdf=scan == "completed",
            read_at=None if index < 2 else "2026-11-10T12:00:00Z",
        )
        rows.append(item)
    for device, width, height, touch in [
        ("iphone", 390, 844, True),
        ("ipad", 820, 1180, True),
        ("desktop", 1440, 1000, False),
    ]:
        visual_device(browser, origin, output, rows, now, device, width, height, touch)


def visual_device(
    browser: Browser,
    origin: str,
    output: Path,
    rows: list[dict[str, object]],
    now: datetime,
    device: str,
    width: int,
    height: int,
    touch: bool,
) -> None:
    context = browser.new_context(
        viewport={"width": width, "height": height},
        is_mobile=touch,
        has_touch=touch,
        locale="en-US",
        timezone_id="UTC",
    )
    try:
        page = context.new_page()
        page.clock.install(time=now)
        status = {
            "last_success_at": int(now.timestamp()),
            "last_attempt_at": int(now.timestamp()),
            "initial_sync_complete": True,
            "error_code": None,
        }
        operation = {
            "id": "00000000-0000-4000-8000-000000000002",
            "state": "queued",
            "error_code": None,
            "created_at": int(now.timestamp()),
            "completed_at": None,
        }
        first = False

        def sync_result(route: Route) -> None:
            if route.request.method == "POST":
                route.fulfill(json=operation)
            else:
                route.fulfill(json=status)

        page.route("**/v1/sync", sync_result)
        page.route(
            "**/v1/summary",
            lambda route: route.fulfill(
                json={
                    "total": 0 if first else len(rows),
                    "unread": 0 if first else 2,
                    "pending_scans": 0 if first else 1,
                    "ready_scans": 0 if first else 4,
                    "storage_due": 2,
                }
            ),
        )
        page.route(
            "**/v1/items?**",
            lambda route: route.fulfill(
                json={"items": [] if first else rows, "next_page_token": None}
            ),
        )

        def mail_detail(route: Route) -> None:
            item_id = route.request.url.rsplit("/", 1)[-1]
            route.fulfill(json=next(item for item in rows if item["id"] == item_id))

        page.route("**/v1/items/fictional-*", mail_detail)
        page.route(
            "**/v1/operations/" + str(operation["id"]),
            lambda route: route.fulfill(json=operation),
        )
        page.goto(origin)
        page.get_by_label("API token", exact=True).fill("test-owner" * 4)
        page.get_by_role("button", name="Connect", exact=True).click()
        expect(page.locator(".mail-card")).to_have_count(len(rows))
        assets = {}
        for name in ["app.js", "style.css", "index.html"]:
            route = "/" if name == "index.html" else "/" + name
            native_asset = context.request.get(origin + route)
            data = native_asset.body()
            source = (Path(__file__).resolve().parents[1] / "web" / name).read_bytes()
            if name != "index.html":
                assert data == source
            assets[name] = hashlib.sha256(data).hexdigest()
        (output / f"{device}-assets.json").write_text(
            json.dumps(assets, indent=2) + "\n"
        )

        def capture(name: str) -> None:
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
            page.screenshot(
                path=str(output / f"{device}-{name}.png"),
                full_page=not page.get_by_role("dialog").is_visible(),
            )

        capture("mailbox")
        for sender, name in [
            ("Maple Studio", "past-storage"),
            ("Meadow Workshop", "due-soon"),
        ]:
            page.get_by_role("button", name=sender, exact=True).click()
            expect(page.get_by_role("dialog")).to_be_visible()
            if name == "past-storage":
                expect(page.get_by_role("dialog")).to_contain_text(
                    "Stable has not reported this item shredded."
                )
                expect(page.get_by_role("dialog")).not_to_contain_text(
                    "Physical mail shredded"
                )
            capture(name)
            page.get_by_role("button", name="Close mail details", exact=True).click()
        page.get_by_role("button", name="Settings", exact=True).click()
        capture("settings")
        page.get_by_role("button", name="Sync now", exact=True).click()
        expect(page.locator("#sync-status")).to_contain_text("Sync queued")
        expect(page.locator("#notice")).to_be_empty()
        capture("queued")
        page.get_by_role("button", name="Mail", exact=True).click()
        capture("mail-queued")
        page.get_by_role("button", name="Settings", exact=True).click()
        operation["state"] = "running"
        page.clock.fast_forward(2100)
        expect(page.locator("#sync-status")).to_contain_text("Syncing")
        capture("syncing")
        page.get_by_role("button", name="Mail", exact=True).click()
        capture("mail-syncing")
        page.get_by_role("button", name="Settings", exact=True).click()
        operation["state"] = "completed"
        status["last_success_at"] = int(now.timestamp()) + 60
        page.clock.fast_forward(61000)
        expect(page.get_by_role("button", name="Sync now", exact=True)).to_be_enabled()
        expect(page.locator("#notice")).to_be_empty()
        expect(page.locator("#sync-status")).to_contain_text("12:01 PM")
        capture("complete")
        status["error_code"] = "provider_rate_limited"
        page.get_by_role("button", name="Refresh", exact=True).click()
        expect(page.locator("#sync-status")).to_contain_text("Sync needs attention")
        capture("failure")
        page.get_by_role("button", name="Mail", exact=True).click()
        capture("mail-failure")
        page.get_by_role("button", name="Settings", exact=True).click()
        status.update(
            last_success_at=None, initial_sync_complete=False, error_code=None
        )
        first = True
        page.get_by_role("button", name="Refresh", exact=True).click()
        page.get_by_role("button", name="Mail", exact=True).click()
        expect(
            page.get_by_role("heading", name="Getting your mail", exact=True)
        ).to_be_visible()
        capture("first-sync")
        first = False
        status.update(last_success_at=int(now.timestamp()), initial_sync_complete=True)
        page.get_by_role("button", name="Refresh", exact=True).click()
        expect(page.locator(".mail-card")).to_have_count(len(rows))
        page.emulate_media(color_scheme="dark")
        capture("mailbox-dark")
        page.get_by_role("button", name="Maple Studio", exact=True).click()
        expect(page.get_by_role("dialog")).to_be_visible()
        capture("past-storage-dark")
        page.get_by_role("button", name="Close mail details", exact=True).click()
        page.get_by_role("button", name="Settings", exact=True).click()
        capture("settings-dark")
    finally:
        context.close()


def main() -> None:
    if sys.argv[1] == "--onboarding":
        onboarding_discovery(sys.argv[2], sys.argv[3])
        return
    if sys.argv[1] == "--automatic":
        automatic_sync(sys.argv[2])
        return
    native, ingress, version = sys.argv[1:]
    upstream = urlsplit(ingress)
    hostname = upstream.hostname
    assert hostname is not None
    prefix = "/api/hassio_ingress/fixture"

    class Proxy(BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            if not self.path.startswith(prefix + "/"):
                self.send_error(404)
                return
            connection = http.client.HTTPConnection(hostname, upstream.port, timeout=10)
            try:
                connection.request(
                    "GET",
                    self.path[len(prefix) :],
                    headers={
                        "X-Remote-User-Id": "fixture-user",
                        "X-Ingress-Path": prefix,
                    },
                )
                response = connection.getresponse()
                payload = response.read()
                self.send_response(response.status)
                for key, value in response.getheaders():
                    if key.lower() in {
                        "content-type",
                        "content-security-policy",
                        "cache-control",
                    }:
                        self.send_header(key, value)
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
            finally:
                connection.close()

        def log_message(self, format: str, *args: object) -> None:
            return

    proxy = ThreadingHTTPServer(("127.0.0.1", 0), Proxy)
    thread = threading.Thread(target=proxy.serve_forever, daemon=True)
    thread.start()
    try:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch()
            try:
                capture = os.environ.get("MAIL_UI_CAPTURE_DIR")
                if capture:
                    visual_review(browser, native, Path(capture))
                exercise(browser.new_page(), native, native=True, version=version)
                exercise(
                    browser.new_page(),
                    f"http://127.0.0.1:{proxy.server_port}{prefix}/",
                    native=False,
                    version=version,
                )
            finally:
                browser.close()
    finally:
        proxy.shutdown()
        proxy.server_close()
        thread.join(timeout=10)


if __name__ == "__main__":
    main()
