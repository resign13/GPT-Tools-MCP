import { invoke } from "@tauri-apps/api/core";

export interface UpdateCheckResult {
  currentVersion: string;
  latestVersion: string;
  latestTag: string;
  updateAvailable: boolean;
  releaseUrl: string;
}

export async function openUrl(url: string): Promise<void> {
  return invoke("open_url", { url });
}

export async function checkAppUpdate(): Promise<UpdateCheckResult> {
  return invoke<UpdateCheckResult>("check_app_update");
}
