import { invoke } from "@tauri-apps/api/core";

export interface WebviewMemorySample {
  mainMb: number;
  webviewMb: number;
  webviewProcessCount: number;
  supported: boolean;
}

/** Sample UI process memory. Does not touch MCP / Actions / FRP. */
export async function getWebviewMemorySample(): Promise<WebviewMemorySample> {
  return invoke<WebviewMemorySample>("get_webview_memory_sample");
}

/**
 * Legacy compatibility API. The native command is intentionally disabled to
 * prevent destructive UI recreation from losing the main window.
 */
export async function recreateUiWebview(): Promise<void> {
  return invoke("recreate_ui_webview");
}
