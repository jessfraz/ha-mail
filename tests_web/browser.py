"""Exercise the real dashboard through native and prefixed ingress listeners."""

from __future__ import annotations

import base64
import http.client
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

from playwright.sync_api import Page, Route, expect, sync_playwright


def exercise(page: Page, origin: str, *, native: bool) -> None:
    page.goto(origin)
    if native:
        expect(page.get_by_role("heading", name="Connect", exact=True)).to_be_visible()
        page.get_by_label("API token", exact=True).fill("test-owner" * 4)
        page.get_by_role("button", name="Connect", exact=True).click()
    expect(page.get_by_role("heading", name="Mailbox", exact=True)).to_be_visible()
    expect(page.locator(".mail-card")).to_have_count(1)
    expect(page.locator(".mail-card")).to_contain_text("<img src=x onerror=alert(1)>")
    expect(page.locator(".mail-card img")).to_have_count(0)
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
    expect(page.get_by_role("heading", name="Mailbox", exact=True)).to_be_visible()
    expect(page.locator("#login")).not_to_be_visible()
    heading = page.get_by_role("heading", name="Mailbox", exact=True)
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
    expect(page.locator("#notice")).to_have_text("Sync queued.")
    expect(page.get_by_role("button", name="Sync now", exact=True)).to_be_disabled()
    page.get_by_role("button", name="Refresh", exact=True).click()
    expect(page.locator("#notice")).not_to_have_text("Loading mail…")
    expect(page.get_by_role("button", name="Sync now", exact=True)).to_be_disabled()
    completed = True
    expect(page.get_by_role("button", name="Sync now", exact=True)).to_be_enabled()
    page.get_by_role("button", name="Refresh", exact=True).click()
    expect(page.locator("#notice")).not_to_have_text("Loading mail…")
    expect(page.get_by_role("button", name="Sync now", exact=True)).to_be_enabled()
    page.unroute("**/v1/sync")
    page.unroute(f"**/v1/operations/{operation_id}")
    page.get_by_role("button", name="Mail", exact=True).click()
    page.emulate_media(color_scheme="dark")
    expect(page.get_by_role("heading", name="Mailbox", exact=True)).to_be_visible()
    page.set_viewport_size({"width": 375, "height": 812})
    assert page.evaluate("document.documentElement.scrollWidth <= window.innerWidth")
    expect(page.locator("#app-version")).to_have_text("0.1.1")
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


def main() -> None:
    native, ingress = sys.argv[1:3]
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
                exercise(browser.new_page(), native, native=True)
                exercise(
                    browser.new_page(),
                    f"http://127.0.0.1:{proxy.server_port}{prefix}/",
                    native=False,
                )
            finally:
                browser.close()
    finally:
        proxy.shutdown()
        proxy.server_close()
        thread.join(timeout=10)


if __name__ == "__main__":
    main()
