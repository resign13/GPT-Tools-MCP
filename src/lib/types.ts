export type RuntimeState = "stopped" | "starting" | "running" | "stopping" | "error";
export type PermissionPreset = "default" | "full_access";
export type IsolationPolicy = "strict" | "compatibility" | "host";
export type ApprovalDecision =
  | "pending"
  | "approved"
  | "denied"
  | "expired"
  | "consumed"
  | "launch_failed"
  | "invalidated";

export interface PermissionStatus {
  permission_mode: PermissionPreset;
  configured_permission_mode: PermissionPreset;
  sandbox_enforced: boolean;
  sandbox_bypass: boolean;
  isolation_backend: string;
  isolation_failure_reason: string | null;
  isolation?: Record<string, unknown>;
  isolation_capabilities?: Record<string, unknown>;
}

export interface ApprovalStatus {
  approval_id: string;
  task: string;
  context_hash: string;
  execution_root: string;
  tool: string;
  capabilities: string[];
  operation_summary: string;
  isolation: string;
  decision: ApprovalDecision;
  remaining_seconds: number;
  created_at_epoch_seconds: number;
  expires_at_epoch_seconds: number;
}

export const DEFAULT_SERVICE_PORT = 28766;
export const DEFAULT_ACTIONS_PORT = 8787;

export interface TunnelConfig {
  type: string;
  public_url: string;
  frp_server: string;
  frp_subdomain: string;
  frp_profile_id?: string;
  frp_server_port?: number;
  cloudflare_mode: string;
  use_proxy?: boolean;
}

export interface AuthConfig {
  type: string;
  oauth_client_id: string;
  use_shared_secrets?: boolean;
}

export interface RuntimeConfig {
  permission_policy_version?: number;
  isolation_policy?: IsolationPolicy;
  local_port: number;
  tool_profile: string;
  permission_mode: string;
  runtime_command?: string;
  allowed_commands?: string;
  workspace_local_entries?: boolean;
  workspace_script_extensions?: string;
}

export interface ActionsConfig {
  permission_policy_version?: number;
  isolation_policy?: IsolationPolicy;
  public_url: string;
  tunnel_type: string;
  frp_server: string;
  frp_subdomain: string;
  frp_profile_id?: string;
  frp_server_port?: number;
  cloudflare_mode: string;
  cloudflare_token?: string;
  use_proxy?: boolean;
  local_port: number;
  permission_mode: string;
  runtime_command?: string;
  auth_type: string;
  oauth_client_id?: string;
  oauth_scopes?: string;
  allowed_commands?: string;
  max_patch_bytes?: number;
  use_shared_secrets?: boolean;
}

export interface WorkspaceProfile {
  id: string;
  name: string;
  path: string;
  tunnel: TunnelConfig;
  auth: AuthConfig;
  runtime: RuntimeConfig;
  actions?: ActionsConfig;
  gateway?: GatewayConfig;
}

export interface GatewayConfig {
  enabled: boolean;
  workspace_ids: string[];
  prompt?: string;
}

export interface RuntimeStatus {
  state: RuntimeState;
  pid: number | null;
  localMessage: string;
  publicMessage: string;
  localEndpoint: string;
  publicEndpoint: string;
}

export function actionsConfig(profile: WorkspaceProfile): ActionsConfig {
  return {
    public_url: "",
    tunnel_type: "frp",
    frp_server: "",
    frp_subdomain: "",
    cloudflare_mode: "quick",
    local_port: DEFAULT_ACTIONS_PORT,
    permission_mode: "default",
    auth_type: "api_key",
    allowed_commands:
      "pytest,python,python3,npm,npx,node,pnpm,yarn,make,mvn,mvnw,gradle,gradlew,cargo,go,ruff,mypy,eslint,tsc",
    max_patch_bytes: 200_000,
    ...profile.actions,
  };
}

export function mcpLocalEndpoint(port: number): string {
  return `http://127.0.0.1:${port}/mcp`;
}

export function actionsLocalEndpoint(port: number): string {
  return `http://127.0.0.1:${port}`;
}

export interface ActionsAuthDraft {
  authType: string;
  oauthClientId: string;
  oauthScopes: string;
  useSharedSecrets?: boolean;
}

export interface FrpProfileSummary {
  id: string;
  name: string;
  server: string;
  serverPort: number;
}

export function frpPublicUrl(
  tunnelType: string,
  frpSubdomain: string,
  frpServer: string,
  frpProfileId: string | undefined,
  profiles: FrpProfileSummary[],
  publicUrl = "",
): string {
  if (tunnelType !== "frp" || !frpSubdomain) {
    return publicUrl.replace(/\/$/, "");
  }
  const server =
    profiles.find((profile) => profile.id === frpProfileId)?.server ?? frpServer;
  if (!server) return publicUrl.replace(/\/$/, "");
  return `https://${frpSubdomain}.${server}`;
}

export function actionsPublicBaseUrl(
  profile: WorkspaceProfile,
  frpProfiles: FrpProfileSummary[] = [],
): string {
  const actions = actionsConfig(profile);
  const publicUrl = frpPublicUrl(
    actions.tunnel_type,
    actions.frp_subdomain,
    actions.frp_server,
    actions.frp_profile_id,
    frpProfiles,
    actions.public_url,
  );
  if (publicUrl) return publicUrl;
  return actionsLocalEndpoint(actions.local_port);
}

export function actionsOpenApiUrl(
  profile: WorkspaceProfile,
  frpProfiles: FrpProfileSummary[] = [],
): string {
  const base = actionsPublicBaseUrl(profile, frpProfiles);
  return base ? `${base.replace(/\/$/, "")}/openapi.json` : "";
}

export function actionsPrivacyUrl(
  profile: WorkspaceProfile,
  frpProfiles: FrpProfileSummary[] = [],
): string {
  const base = actionsPublicBaseUrl(profile, frpProfiles);
  return base ? `${base.replace(/\/$/, "")}/privacy` : "";
}

export function actionsOAuthAuthorizeUrl(
  profile: WorkspaceProfile,
  frpProfiles: FrpProfileSummary[] = [],
): string {
  const base = actionsPublicBaseUrl(profile, frpProfiles);
  return base ? `${base.replace(/\/$/, "")}/oauth/authorize` : "";
}

export function actionsOAuthTokenUrl(
  profile: WorkspaceProfile,
  frpProfiles: FrpProfileSummary[] = [],
): string {
  const base = actionsPublicBaseUrl(profile, frpProfiles);
  return base ? `${base.replace(/\/$/, "")}/oauth/token` : "";
}
