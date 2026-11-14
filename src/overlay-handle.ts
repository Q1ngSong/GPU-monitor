import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./overlay-handle.css";

const handle = document.querySelector<HTMLButtonElement>("#overlay-handle")!;
handle.addEventListener("mousedown", event => {
  if (event.button !== 0) return;
  event.preventDefault();
  if (isTauri()) void getCurrentWindow().startDragging().catch(reason => { handle.title = String(reason); });
});
