<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import AuthConfigForm from "$lib/components/AuthConfigForm.svelte";
  import HealthPanel from "$lib/components/HealthPanel.svelte";
  import LogViewer from "$lib/components/LogViewer.svelte";
  import RuntimePolicyForm, {
    type RuntimePolicyDraft,
  } from "$lib/components/RuntimePolicyForm.svelte";
  import ServicePanel from "$lib/components/ServicePanel.svelte";
  import Tabs from "$lib/components/Tabs.svelte";
  import TunnelConfigForm, {
    type TunnelFormConfig,
    type SaveTunnelOptions,
  } from "$lib/components/TunnelConfigForm.svelte";
  import {
    getRuntimeStatus,
    listWorkspaces,
    startRuntime,
    restartRuntime,
    stopRuntime,
    updateWorkspace,
  } from "$lib/api/workspaces";
  import { listFrpProfiles, setLastWorkspace, type FrpProfileDto } from "$lib/api/settings";
  import { restartTunnel, stopTunnel } from "$lib/api/tunnel";
  import { runServiceToggle, notifyStartFailure } from "$lib/runtime/service";
  import { showToast } from "$lib/stores/toast";
  import { promptServiceRestart } from "$lib/runtime/restart-hint";
  import { mcpRuntimeStates, workspaces } from "$lib/stores/app";
  import {
    frpPublicUrl,
    mcpLocalEndpoint,
    type AuthConfig,
    type RuntimeState,
    type WorkspaceProfile,
  } from "$lib/types";

  type SubTab = "config" | "logs" | "health";

  let profile = $state<WorkspaceProfile | null>(null);
  let mcpStatus = $state<RuntimeState>("stopped");
  let mcpStatusMessage = $state("");
  let mcpBusy = $state(false);
  let mcpLocal = $state("");
  let mcpPublic = $state("");
  let frpProfiles = $state<FrpProfileDto[]>([]);

  let mcpSubTab = $state<SubTab>("config");
  let loadGeneration = 0;

  const subTabs = [
    { value: "config", label: "配置" },
    { value: "logs", label: "日志" },
    { value: "health", label: "健康" },
  ];

  const workspaceId = $derived($page.params.id);
  const mcpTunnelForm = $derived<TunnelFormConfig>({
    type: profile?.tunnel.type ?? "none",
    public_url: profile?.tunnel.public_url ?? "",
    frp_server: profile?.tunnel.frp_server ?? "",
    frp_subdomain: profile?.tunnel.frp_subdomain ?? "",
    frp_profile_id: profile?.tunnel.frp_profile_id ?? "",
    frp_server_port: profile?.tunnel.frp_server_port ?? 7000,
    cloudflare_mode: profile?.tunnel.cloudflare_mode ?? "quick",
    use_proxy: profile?.tunnel.use_proxy ?? true,
  });

  function applyMcpRuntime(
    runtime: { state: RuntimeState; localEndpoint: string; publicEndpoint: string; localMessage?: string },
    id = workspaceId,
  ) {
    if (!id || id !== workspaceId) return;
    mcpStatus = runtime.state;
    mcpStatusMessage = runtime.localMessage ?? "";
    mcpLocal = runtime.localEndpoint;
    mcpPublic = runtime.publicEndpoint;
    mcpRuntimeStates.update((current) => ({ ...current, [id]: runtime.state }));
  }

  async function load(id = workspaceId) {
    if (!id) return;
    const generation = ++loadGeneration;
    const items = await listWorkspaces();
    if (generation !== loadGeneration || id !== workspaceId) return;
    workspaces.set(items);
    frpProfiles = await listFrpProfiles();
    if (generation !== loadGeneration || id !== workspaceId) return;
    const nextProfile = items.find((item) => item.id === id) ?? null;
    if (generation !== loadGeneration || id !== workspaceId) return;
    profile = nextProfile;
    if (nextProfile) {
      await setLastWorkspace(nextProfile.id);
    }
    if (generation !== loadGeneration || id !== workspaceId) return;
    if (!nextProfile) {
      await goto("/");
      return;
    }
    const gatewayHost = items.find((item) => item.gateway?.enabled);
    if (gatewayHost && gatewayHost.id !== nextProfile.id) {
      await goto(`/gateway?workspace=${encodeURIComponent(nextProfile.id)}`);
      return;
    }

    const mcpRuntime = await getRuntimeStatus(id);
    if (generation !== loadGeneration || id !== workspaceId) return;
    applyMcpRuntime(mcpRuntime, id);
  }

  async function refreshProfile(id = workspaceId): Promise<WorkspaceProfile | null> {
    if (!id) return null;
    const items = await listWorkspaces();
    if (id !== workspaceId) return null;
    workspaces.set(items);
    const nextProfile = items.find((item) => item.id === id) ?? null;
    profile = nextProfile;
    return nextProfile;
  }

  function tunnelConfigured(type: string | undefined): boolean {
    return type === "cloudflare" || type === "frp";
  }

  async function afterServiceStart(
    runtime: { state: RuntimeState; publicEndpoint: string },
    id: string,
  ) {
    const nextProfile = await refreshProfile(id);
    if (id !== workspaceId) return;
    const tunnelType = nextProfile?.tunnel.type;
    if (runtime.state === "running" && tunnelConfigured(tunnelType) && !runtime.publicEndpoint) {
      showToast(
        "本地服务已启动，但隧道未能自动连接。请检查代理设置与隧道配置，或查看日志。",
        { title: "隧道未连接", kind: "warning", duration: 8000 },
      );
    }
  }

  async function toggleMcp() {
    const id = workspaceId;
    if (!id || mcpBusy) return;
    const wasRunning = mcpStatus === "running";
    mcpBusy = true;
    try {
      const runtime = await runServiceToggle(
        wasRunning,
        () => startRuntime(id),
        () => stopRuntime(id),
        "MCP",
      );
      if (runtime && id === workspaceId) {
        applyMcpRuntime(runtime, id);
        if (!wasRunning) {
          if (runtime.state === "running") {
            await afterServiceStart(runtime, id);
          } else {
            notifyStartFailure("MCP", runtime);
          }
        }
      }
    } finally {
      mcpBusy = false;
    }
  }

  async function saveMcpPort(port: number) {
    if (!profile || profile.runtime.local_port === port) return;
    const next: WorkspaceProfile = {
      ...profile,
      runtime: { ...profile.runtime, local_port: port },
    };
    await updateWorkspace(next);
    profile = next;
    mcpLocal = mcpLocalEndpoint(port);
    await load();
  }

  function publicEndpointFromTunnel(config: TunnelFormConfig, suffix: string): string {
    const base = frpPublicUrl(
      config.type,
      config.frp_subdomain,
      config.frp_server,
      config.frp_profile_id,
      frpProfiles,
      config.public_url,
    );
    if (base) {
      return `${base.replace(/\/$/, "")}${suffix}`;
    }
    return "";
  }

  async function restartTunnelIfConfigured(
    targetWorkspaceId: string,
    config: TunnelFormConfig,
    service: "mcp",
  ) {
    if (config.type === "none") {
      await stopTunnel(targetWorkspaceId, service);
      return;
    }
    const status = await restartTunnel(targetWorkspaceId, service);
    if (workspaceId !== targetWorkspaceId) return;
    if (status.publicUrl) {
      mcpPublic = `${status.publicUrl.replace(/\/$/, "")}/mcp`;
    }
  }

  async function saveMcpTunnel(config: TunnelFormConfig, options?: SaveTunnelOptions) {
    if (!profile) return;
    const targetWorkspaceId = workspaceId;
    if (!targetWorkspaceId) return;
    const next: WorkspaceProfile = {
      ...profile,
      tunnel: {
        ...profile.tunnel,
        type: config.type,
        public_url: config.public_url,
        frp_server: config.frp_server,
        frp_subdomain: config.frp_subdomain,
        frp_profile_id: config.frp_profile_id,
        frp_server_port: config.frp_server_port,
        cloudflare_mode: config.cloudflare_mode,
        use_proxy: config.use_proxy,
      },
    };
    await updateWorkspace(next);
    if (!options?.skipTunnelRestart) {
      await restartTunnelIfConfigured(targetWorkspaceId, config, "mcp");
    }
    if (workspaceId !== targetWorkspaceId) return;
    profile = next;
    mcpPublic = publicEndpointFromTunnel(config, "/mcp");
    if (!options?.skipTunnelRestart && !options?.skipServicePrompt) {
      await load();
      if (workspaceId !== targetWorkspaceId) return;
    }
    if (!options?.skipServicePrompt) {
      await promptServiceRestart(mcpStatus === "running", "MCP 服务");
    }
  }

  async function saveMcpPolicy(draft: RuntimePolicyDraft) {
    if (!profile) return;
    const next: WorkspaceProfile = {
      ...profile,
      runtime: {
        ...profile.runtime,
        tool_profile: draft.toolProfile,
        permission_mode: draft.permissionMode,
        permission_policy_version: ["default", "full_access"].includes(draft.permissionMode)
          ? 1
          : (profile.runtime.permission_policy_version ?? 0),
        isolation_policy: draft.isolationPolicy,
        allowed_commands: draft.allowedCommands,
        workspace_local_entries: draft.workspaceLocalEntries,
        workspace_script_extensions: draft.workspaceScriptExtensions,
      },
    };
    await updateWorkspace(next);
    profile = next;
    await load();
    await promptServiceRestart(mcpStatus === "running", "MCP 服务");
  }

  async function saveMcpAuth(auth: AuthConfig, options?: { skipRuntimeRestart?: boolean }) {
    if (!profile || !workspaceId) return;
    const next: WorkspaceProfile = { ...profile, auth };
    await updateWorkspace(next);
    profile = next;
    if (!options?.skipRuntimeRestart && mcpStatus === "running") {
      try {
        await restartRuntime(workspaceId);
      } catch (error) {
        showToast(String(error), { title: "服务重启失败", kind: "error", duration: 8000 });
      }
    }
  }

  $effect(() => {
    const id = workspaceId;
    if (!id) return;
    profile = null;
    void load(id);

    return () => {
      loadGeneration += 1;
    };
  });
</script>

{#if profile}
  <section class="page-scroll">
    <div class="page-body">
      <div class="mt-4 flex flex-col gap-3">
        <ServicePanel
          title="MCP"
          subtitle="Streamable HTTP · 工具运行时"
          status={mcpStatus}
          statusMessage={mcpStatusMessage}
          port={profile.runtime.local_port}
          portEditable={true}
          busy={mcpBusy}
          tunnelType={profile.tunnel.type}
          localEndpoint={mcpLocal || mcpLocalEndpoint(profile.runtime.local_port)}
          publicEndpoint={mcpPublic}
          publicLabel="公网 MCP"
          onToggle={toggleMcp}
          onPortChange={saveMcpPort}
        />
      </div>

        <div class="mt-5">
          <Tabs
            items={subTabs}
            value={mcpSubTab}
            onchange={(v) => {
              mcpSubTab = v as SubTab;
            }}
          />
        </div>

        {#if mcpSubTab === "config"}
          <div class="tx-card mt-4 grid gap-6 p-5">
            <div>
              <p class="tx-section-label">隧道</p>
              <TunnelConfigForm
                workspaceId={workspaceId!}
                service="mcp"
                config={mcpTunnelForm}
                onSave={saveMcpTunnel}
              />
            </div>
            <div>
              <p class="tx-section-label">认证</p>
              <AuthConfigForm
                workspaceId={workspaceId!}
                auth={profile.auth}
                onSaveProfile={saveMcpAuth}
              />
            </div>
            <div>
              <p class="tx-section-label">策略</p>
              <RuntimePolicyForm
                workspaceId={workspaceId ?? undefined}
                toolProfile={profile.runtime.tool_profile}
                permissionMode={profile.runtime.permission_mode}
                isolationPolicy={profile.runtime.isolation_policy ?? "strict"}
                allowedCommands={profile.runtime.allowed_commands ?? ""}
                workspaceLocalEntries={profile.runtime.workspace_local_entries ?? true}
                workspaceScriptExtensions={profile.runtime.workspace_script_extensions ?? ".exe,.bat,.cmd,.ps1"}
                onSave={saveMcpPolicy}
              />
            </div>
          </div>
        {:else if mcpSubTab === "logs"}
          <div class="mt-4">
            <LogViewer workspaceId={workspaceId!} service="mcp" />
          </div>
        {:else}
          <div class="mt-4">
            <HealthPanel workspaceId={workspaceId!} />
          </div>
        {/if}
    </div>

    <footer class="border-t border-[var(--color-border)] px-8 py-4 text-xs text-[var(--color-text-muted)]">
      MCP 默认端口 28766；隧道、认证和策略配置位于下方。
    </footer>
  </section>
{/if}
