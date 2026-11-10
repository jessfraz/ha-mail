import { mountAppShell } from "./app-kit.js";

const header = document.querySelector(".app-header");
const footer = document.getElementById("app-footer");
if (!(header instanceof HTMLElement) || !(footer instanceof HTMLElement)) {
  throw new Error("The app shell is unavailable.");
}
export const shell = mountAppShell({
  name: "Mail",
  header,
  footer,
  sourceUrl: "https://github.com/jessfraz/ha-mail",
  brand: [...header.querySelectorAll("#app-brand > img")],
  actions: [...header.querySelectorAll(":scope > button")],
});
shell.version.id = "app-version";
shell.status.id = "service-status";
