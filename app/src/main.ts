import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";
import TrayPopover from "./tray/TrayPopover.svelte";

const target = document.getElementById("app");
if (!target) {
  throw new Error("index.html has no #app element to mount into");
}

// The tray popover window loads this same entry with `?tray=1` (see
// src-tauri/src/tray.rs) — same bundle, a different tiny root component,
// so there's only one Vite build to keep in sync with the Story contract.
const params = new URLSearchParams(window.location.search);
const root = params.has("tray") ? mount(TrayPopover, { target }) : mount(App, { target });

export default root;
