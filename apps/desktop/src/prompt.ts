import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/**
 * The offer to take notes, shown when a call starts.
 *
 * It withdraws itself after a while: a prompt that sits over the call for its
 * whole length would be in the way of the thing it is offering to help with.
 */
const SHOW_MS = 30_000;

const title = document.getElementById("title")!;
const accept = document.getElementById("accept") as HTMLButtonElement;
const later = document.getElementById("later") as HTMLButtonElement;

let timer: number | undefined;

void listen<string>("prompt:call", (e) => {
  title.textContent = `${e.payload} call started`;
  accept.disabled = false;
  window.clearTimeout(timer);
  timer = window.setTimeout(() => void invoke("meeting_prompt_dismiss"), SHOW_MS);
});

accept.addEventListener("click", async () => {
  window.clearTimeout(timer);
  accept.disabled = true;
  try {
    await invoke("meeting_prompt_accept");
  } catch (e) {
    title.textContent = "Could not start notes";
    accept.disabled = false;
    console.error(e);
  }
});

later.addEventListener("click", () => {
  window.clearTimeout(timer);
  void invoke("meeting_prompt_dismiss");
});
