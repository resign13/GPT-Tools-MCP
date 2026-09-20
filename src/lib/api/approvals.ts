import { invoke } from "@tauri-apps/api/core";
import type { ApprovalStatus } from "$lib/types";

export async function listPendingApprovals(): Promise<ApprovalStatus[]> {
  return invoke<ApprovalStatus[]>("list_pending_approvals");
}

export async function decideApproval(
  approvalId: string,
  approve: boolean,
): Promise<ApprovalStatus> {
  return invoke<ApprovalStatus>("decide_approval", {
    approvalId,
    approve,
  });
}
