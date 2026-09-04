use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{
    OwnedRwLockReadGuard, OwnedRwLockWriteGuard, OwnedSemaphorePermit, RwLock, Semaphore,
};

use crate::data::DataStore;
use crate::mcp::server::{handle_request, SharedState};
use crate::mcp::workspace_context::WorkspaceContextPin;
use crate::tools::execution_context::GitIdentity;
use crate::tools::policy::PolicySettings;
use crate::tools::workspace::{tool_err, tool_err_code, tool_ok, Workspace};
use crate::tools::ToolContext;
use crate::workspace::WorkspaceProfile;

mod workspace_context_gateway;

const MAX_BINDINGS: usize = 256;
const BINDING_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_CONCURRENT_REQUESTS: usize = 8;
const GATEWAY_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKeySource {
    OpenAiConversation,
    McpTransport,
    ServerGenerated,
}

impl SessionKeySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiConversation => "openai_conversation",
            Self::McpTransport => "mcp_transport",
            Self::ServerGenerated => "server_generated",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionIdentifiers {
    pub openai: Option<String>,
    pub transport: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResolvedSession {
    pub key: String,
    pub transport_id: Option<String>,
    pub source: SessionKeySource,
}

#[derive(Clone)]
#[allow(dead_code)]
enum ProfileSource {
    Dynamic,
    Static(HashMap<String, WorkspaceProfile>),
}

struct Binding {
    workspace_id: String,
    fingerprint: String,
    context: SharedState,
    context_lock: Option<Arc<Mutex<WorkspaceContextPin>>>,
    lock_key: String,
    session_key_source: SessionKeySource,
    last_used: Instant,
}

struct SessionAlias {
    canonical_key: String,
    last_used: Instant,
}

#[allow(dead_code)]
enum WorkspacePermit {
    Read(OwnedRwLockReadGuard<()>),
    Write(OwnedRwLockWriteGuard<()>),
}

struct GatewayPermit {
    _global: OwnedSemaphorePermit,
    _workspace: WorkspacePermit,
}

struct RouteTarget {
    context: SharedState,
    lock_key: String,
    context_summary: Option<Value>,
}

#[derive(Clone)]
struct BindingSnapshot {
    workspace_id: String,
    fingerprint: String,
    context: SharedState,
    context_lock: Option<Arc<Mutex<WorkspaceContextPin>>>,
    lock_key: String,
    session_key_source: SessionKeySource,
}

pub struct GatewayRouter {
    host_workspace_id: String,
    source: ProfileSource,
    bindings: Mutex<HashMap<String, Binding>>,
    aliases: Mutex<HashMap<String, SessionAlias>>,
    workspace_locks: Mutex<HashMap<String, Arc<RwLock<()>>>>,
    global_gate: Arc<Semaphore>,
}

impl GatewayRouter {
    pub fn from_host(host_workspace_id: String) -> Result<Option<Arc<Self>>, String> {
        let store = DataStore::load().map_err(|error| error.to_string())?;
        let Some(host) = store.get(&host_workspace_id) else {
            return Ok(None);
        };
        if !host.gateway.enabled {
            return Ok(None);
        }
        Ok(Some(Arc::new(Self {
            host_workspace_id,
            source: ProfileSource::Dynamic,
            bindings: Mutex::new(HashMap::new()),
            aliases: Mutex::new(HashMap::new()),
            workspace_locks: Mutex::new(HashMap::new()),
            global_gate: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
        })))
    }

    #[cfg(test)]
    fn from_profiles(host: WorkspaceProfile, profiles: Vec<WorkspaceProfile>) -> Arc<Self> {
        let mut by_id = profiles
            .into_iter()
            .map(|profile| (profile.id.clone(), profile))
            .collect::<HashMap<_, _>>();
        by_id.insert(host.id.clone(), host.clone());
        Arc::new(Self {
            host_workspace_id: host.id,
            source: ProfileSource::Static(by_id),
            bindings: Mutex::new(HashMap::new()),
            aliases: Mutex::new(HashMap::new()),
            workspace_locks: Mutex::new(HashMap::new()),
            global_gate: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
        })
    }

    fn profiles(&self) -> HashMap<String, WorkspaceProfile> {
        match &self.source {
            ProfileSource::Static(profiles) => profiles.clone(),
            ProfileSource::Dynamic => DataStore::load()
                .map(|store| {
                    store
                        .list()
                        .iter()
                        .cloned()
                        .map(|profile| (profile.id.clone(), profile))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn host(
        &self,
        profiles: &HashMap<String, WorkspaceProfile>,
    ) -> Result<WorkspaceProfile, Value> {
        let host = profiles
            .get(&self.host_workspace_id)
            .cloned()
            .ok_or_else(|| {
                tool_err_code(
                    "workspace_unavailable",
                    "Gateway host workspace is unavailable.",
                    "gateway",
                )
            })?;
        if !host.gateway.enabled {
            return Err(tool_err_code(
                "gateway_disabled",
                "Gateway mode is disabled for the host workspace.",
                "gateway",
            ));
        }
        if !host
            .gateway
            .workspace_ids
            .iter()
            .any(|id| id == &self.host_workspace_id)
        {
            return Err(tool_err_code(
                "workspace_not_allowed",
                "Gateway host must be present in its own allowlist.",
                "gateway",
            ));
        }
        Ok(host)
    }

    pub fn is_enabled(&self) -> bool {
        self.host(&self.profiles()).is_ok()
    }

    pub fn host_workspace_id(&self) -> &str {
        &self.host_workspace_id
    }

    /// Resolve the logical conversation key while retaining aliases for a
    /// connector transport that may be reused across ChatGPT conversations.
    pub fn resolve_session_identity(
        &self,
        identifiers: SessionIdentifiers,
        initialize: bool,
    ) -> Result<ResolvedSession, Value> {
        let openai = normalize_session_identifier(identifiers.openai);
        let transport = normalize_session_identifier(identifiers.transport);
        if openai.is_none() && transport.is_none() && !initialize {
            return Err(tool_err_code(
                "session_required",
                "MCP session identifier is required.",
                "gateway",
            ));
        }

        let mut aliases = self.aliases.lock().expect("gateway alias lock");
        evict_expired_aliases(&mut aliases);
        let openai_canonical = openai
            .as_ref()
            .and_then(|key| aliases.get(key).map(|alias| alias.canonical_key.clone()));
        let transport_canonical = transport
            .as_ref()
            .and_then(|key| aliases.get(key).map(|alias| alias.canonical_key.clone()));

        if let (Some(openai_key), Some(transport_key)) =
            (openai_canonical.as_ref(), transport_canonical.as_ref())
        {
            if openai_key != transport_key {
                return Err(tool_err_code(
                    "session_identity_conflict",
                    "OpenAI conversation and MCP transport identifiers resolve to different sessions.",
                    "gateway",
                ));
            }
        }

        let generated = if openai.is_none() && transport.is_none() && initialize {
            Some(uuid::Uuid::new_v4().to_string())
        } else {
            None
        };
        // OpenAI's conversation id is the logical identity.  A connector can
        // reuse a transport id for a newly opened conversation, so an unknown
        // OpenAI id must win over an already-known transport alias.  Once both
        // aliases are known, the conflict check above still rejects a stale
        // pair instead of allowing an old conversation to operate on the new
        // binding.
        let canonical = openai_canonical
            .or_else(|| openai.clone())
            .or(transport_canonical)
            .or_else(|| transport.clone())
            .or(generated.clone())
            .expect("session identity must have a canonical key");
        let source = if openai.is_some() {
            SessionKeySource::OpenAiConversation
        } else if transport.is_some() {
            SessionKeySource::McpTransport
        } else {
            SessionKeySource::ServerGenerated
        };

        let now = Instant::now();
        for alias in [openai.as_ref(), transport.as_ref(), generated.as_ref()]
            .into_iter()
            .flatten()
        {
            aliases.insert(
                alias.clone(),
                SessionAlias {
                    canonical_key: canonical.clone(),
                    last_used: now,
                },
            );
        }
        trim_aliases(&mut aliases);

        Ok(ResolvedSession {
            key: canonical,
            transport_id: transport.or(generated),
            source,
        })
    }

    pub fn authorization_workspace_names(&self) -> Vec<String> {
        let profiles = self.profiles();
        let Ok(host) = self.host(&profiles) else {
            return Vec::new();
        };
        host.gateway
            .workspace_ids
            .iter()
            .filter_map(|id| profiles.get(id).map(|profile| profile.name.clone()))
            .collect()
    }

    pub fn selected_workspace_id(&self, session_key: &str) -> String {
        let profiles = self.profiles();
        self.valid_binding(session_key, &profiles)
            .map(|binding| binding.workspace_id)
            .unwrap_or_else(|| "unselected".into())
    }

    pub fn session_hash(session_key: &str) -> String {
        let mut digest = Sha256::new();
        digest.update(session_key.as_bytes());
        format!("{:x}", digest.finalize())[..16].to_string()
    }

    pub fn list_workspaces(&self) -> Value {
        let profiles = self.profiles();
        let host = match self.host(&profiles) {
            Ok(host) => host,
            Err(error) => return error,
        };
        let mut seen = std::collections::HashSet::new();
        let workspaces = host
            .gateway
            .workspace_ids
            .iter()
            .filter(|id| seen.insert((*id).clone()))
            .map(|id| {
                profiles.get(id).map(public_workspace).unwrap_or_else(|| {
                    json!({
                        "id": id,
                        "name": id,
                        "path": "",
                        "available": false
                    })
                })
            })
            .collect::<Vec<_>>();
        tool_ok(json!({
            "workspaces": workspaces,
            "host_workspace_id": self.host_workspace_id
        }))
    }

    #[allow(dead_code)]
    pub fn select_workspace(&self, session_key: &str, workspace_id: &str) -> Value {
        self.select_workspace_with_source(session_key, workspace_id, SessionKeySource::McpTransport)
    }

    fn select_workspace_with_source(
        &self,
        session_key: &str,
        workspace_id: &str,
        source: SessionKeySource,
    ) -> Value {
        let profiles = self.profiles();
        let host = match self.host(&profiles) {
            Ok(host) => host,
            Err(error) => return error,
        };
        if !host
            .gateway
            .workspace_ids
            .iter()
            .any(|id| id == workspace_id)
        {
            return tool_err_code(
                "workspace_not_allowed",
                "Workspace is not in the host allowlist.",
                "gateway",
            );
        }
        let Some(target) = profiles.get(workspace_id).cloned() else {
            return tool_err_code(
                "workspace_unavailable",
                "Workspace does not exist.",
                "gateway",
            );
        };
        if let Some(existing) = self.valid_binding(session_key, &profiles) {
            if existing.workspace_id == target.id {
                return binding_success(&target, existing.session_key_source, session_key);
            }
            return tool_err_code(
                "workspace_locked",
                "This conversation is already bound to another workspace. Start a new conversation to switch workspaces.",
                "gateway",
            );
        }
        let context = match self.build_context(&host, &target) {
            Ok(context) => context,
            Err(error) => return error,
        };
        let fingerprint = binding_fingerprint(&host, &target);
        let lock_key = workspace_lock_key(&target.path);
        let mut bindings = self.bindings.lock().expect("gateway binding lock");
        evict_expired(&mut bindings);
        // `valid_binding` runs before context construction so the gateway lock
        // is not held during filesystem/Git probing. Re-check while holding
        // the lock immediately before insertion; otherwise two concurrent
        // bind requests for one conversation could both observe an empty slot
        // and the last writer would silently replace the first workspace.
        if let Some(existing) = bindings.get(session_key) {
            if existing.workspace_id == target.id && existing.fingerprint == fingerprint {
                return binding_success(&target, existing.session_key_source, session_key);
            }
            if existing.workspace_id != target.id {
                return tool_err_code(
                    "workspace_locked",
                    "This conversation is already bound to another workspace. Start a new conversation to switch workspaces.",
                    "gateway",
                );
            }
            bindings.remove(session_key);
            self.remove_aliases_for(session_key);
        }
        if bindings.len() >= MAX_BINDINGS && !bindings.contains_key(session_key) {
            if let Some(oldest) = bindings
                .iter()
                .min_by_key(|(_, binding)| binding.last_used)
                .map(|(key, _)| key.clone())
            {
                bindings.remove(&oldest);
                self.remove_aliases_for(&oldest);
            }
        }
        bindings.insert(
            session_key.to_string(),
            Binding {
                workspace_id: target.id.clone(),
                fingerprint,
                context,
                context_lock: None,
                lock_key,
                session_key_source: source,
                last_used: Instant::now(),
            },
        );
        binding_success(&target, source, session_key)
    }

    pub fn bind_workspace(
        &self,
        session_key: &str,
        workspace_hint: &str,
        source: SessionKeySource,
    ) -> Value {
        let hint = workspace_hint.trim();
        if hint.is_empty() {
            return tool_err_code(
                "workspace_hint_required",
                "workspace_hint must contain a workspace ID, name, path, or directory name.",
                "validation",
            );
        }
        let profiles = self.profiles();
        let host = match self.host(&profiles) {
            Ok(host) => host,
            Err(error) => return error,
        };
        let candidates = host
            .gateway
            .workspace_ids
            .iter()
            .filter_map(|id| profiles.get(id))
            .filter(|profile| workspace_hint_matches(profile, hint))
            .cloned()
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [] => tool_err_code(
                "workspace_not_found",
                "No allowlisted workspace matches workspace_hint.",
                "gateway",
            ),
            [target] => self.select_workspace_with_source(session_key, &target.id, source),
            _ => {
                let mut error = tool_err_code(
                    "workspace_ambiguous",
                    "workspace_hint matches more than one allowlisted workspace.",
                    "gateway",
                );
                error["error"]["details"] = json!({
                    "candidates": candidates.iter().map(public_workspace).collect::<Vec<_>>()
                });
                error
            }
        }
    }

    pub fn selected_workspace(&self, session_key: &str) -> Value {
        let profiles = self.profiles();
        let host = match self.host(&profiles) {
            Ok(host) => host,
            Err(error) => return error,
        };
        let Some(binding) = self.valid_binding(session_key, &profiles) else {
            return tool_err_code(
                "workspace_not_selected",
                "Select a workspace for this MCP session first.",
                "gateway",
            );
        };
        if !host
            .gateway
            .workspace_ids
            .iter()
            .any(|id| id == &binding.workspace_id)
        {
            self.bindings
                .lock()
                .expect("gateway binding lock")
                .remove(session_key);
            return tool_err_code(
                "workspace_not_allowed",
                "Selected workspace is no longer in the host allowlist.",
                "gateway",
            );
        }
        let Some(profile) = profiles.get(&binding.workspace_id) else {
            self.bindings
                .lock()
                .expect("gateway binding lock")
                .remove(session_key);
            self.remove_aliases_for(session_key);
            return tool_err_code(
                "workspace_unavailable",
                "Selected workspace is unavailable.",
                "gateway",
            );
        };
        tool_ok(json!({
            "selected": true,
            "locked": true,
            "session_key_source": binding.session_key_source.as_str(),
            "workspace": public_workspace(profile),
            "session_hash": Self::session_hash(session_key)
        }))
    }

    #[allow(dead_code)]
    pub fn context_for(&self, session_key: &str, tool_name: &str) -> Result<SharedState, Value> {
        self.route_for(session_key, tool_name)
            .map(|target| target.context)
    }

    fn route_for(&self, session_key: &str, tool_name: &str) -> Result<RouteTarget, Value> {
        let profiles = self.profiles();
        let host = self.host(&profiles)?;
        let mut bindings = self.bindings.lock().expect("gateway binding lock");
        evict_expired(&mut bindings);
        let Some(binding) = bindings.get_mut(session_key) else {
            return Err(tool_err_code(
                "workspace_not_selected",
                "Select a workspace for this MCP session first.",
                "gateway",
            ));
        };
        let Some(target) = profiles.get(&binding.workspace_id) else {
            bindings.remove(session_key);
            return Err(tool_err_code(
                "workspace_unavailable",
                "Selected workspace is unavailable.",
                "gateway",
            ));
        };
        if !host
            .gateway
            .workspace_ids
            .iter()
            .any(|id| id == &binding.workspace_id)
        {
            bindings.remove(session_key);
            return Err(tool_err_code(
                "workspace_not_allowed",
                "Selected workspace is no longer in the host allowlist.",
                "gateway",
            ));
        }
        if binding_fingerprint(&host, target) != binding.fingerprint
            || !PathBuf::from(&target.path).is_dir()
        {
            bindings.remove(session_key);
            return Err(tool_err_code(
                "workspace_changed",
                "Workspace configuration changed; select it again.",
                "gateway",
            ));
        }
        let canonical = crate::tools::registry::canonical_tool_name(tool_name);
        let host_tools = crate::tools::registry::exposed_tool_names(&host.runtime.tool_profile);
        let target_tools = crate::tools::registry::exposed_tool_names(&target.runtime.tool_profile);
        if !host_tools.contains(&canonical) || !target_tools.contains(&canonical) {
            return Err(tool_err_code(
                "tool_not_allowed",
                "Tool is not allowed by both host and target profiles.",
                "policy",
            ));
        }
        binding.last_used = Instant::now();
        let context_lock = binding.context_lock.clone();
        let mut target = RouteTarget {
            context: binding.context.clone(),
            lock_key: binding.lock_key.clone(),
            context_summary: None,
        };
        drop(bindings);
        if let Some(context_lock) = context_lock {
            let mut pin = context_lock.lock().expect("workspace context lock");
            match pin.validate() {
                Ok(_) => target.context_summary = Some(pin.snapshot("valid", None)),
                Err(error) => return Err(error.tool_value(Some(&pin))),
            }
        }
        Ok(target)
    }

    fn acquire_permit(&self, lock_key: &str, mutating: bool) -> Option<GatewayPermit> {
        let deadline = Instant::now() + GATEWAY_WAIT_TIMEOUT;
        let global = loop {
            match self.global_gate.clone().try_acquire_owned() {
                Ok(permit) => break permit,
                Err(_) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return None,
            }
        };
        let lock = {
            let mut locks = self
                .workspace_locks
                .lock()
                .expect("gateway workspace lock table");
            locks
                .entry(lock_key.to_string())
                .or_insert_with(|| Arc::new(RwLock::new(())))
                .clone()
        };
        let workspace = loop {
            let permit = if mutating {
                lock.clone().try_write_owned().map(WorkspacePermit::Write)
            } else {
                lock.clone().try_read_owned().map(WorkspacePermit::Read)
            };
            if let Ok(permit) = permit {
                break permit;
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(10));
        };
        Some(GatewayPermit {
            _global: global,
            _workspace: workspace,
        })
    }

    fn remove_aliases_for(&self, canonical_key: &str) {
        self.aliases
            .lock()
            .expect("gateway alias lock")
            .retain(|_, alias| alias.canonical_key != canonical_key);
    }

    fn valid_binding(
        &self,
        session_key: &str,
        profiles: &HashMap<String, WorkspaceProfile>,
    ) -> Option<BindingView> {
        let mut bindings = self.bindings.lock().expect("gateway binding lock");
        evict_expired(&mut bindings);
        let binding = bindings.get_mut(session_key)?;
        let host = self.host(profiles).ok()?;
        if !host
            .gateway
            .workspace_ids
            .iter()
            .any(|id| id == &binding.workspace_id)
        {
            bindings.remove(session_key);
            return None;
        }
        let Some(profile) = profiles.get(&binding.workspace_id) else {
            bindings.remove(session_key);
            self.remove_aliases_for(session_key);
            return None;
        };
        if binding_fingerprint(&host, profile) != binding.fingerprint
            || !PathBuf::from(&profile.path).is_dir()
        {
            bindings.remove(session_key);
            self.remove_aliases_for(session_key);
            return None;
        }
        binding.last_used = Instant::now();
        Some(BindingView {
            workspace_id: binding.workspace_id.clone(),
            session_key_source: binding.session_key_source,
        })
    }

    fn build_context(
        &self,
        host: &WorkspaceProfile,
        target: &WorkspaceProfile,
    ) -> Result<SharedState, Value> {
        self.build_context_at_root(host, target, PathBuf::from(&target.path), None)
    }

    fn build_context_at_root(
        &self,
        host: &WorkspaceProfile,
        target: &WorkspaceProfile,
        root: PathBuf,
        identity: Option<GitIdentity>,
    ) -> Result<SharedState, Value> {
        let workspace =
            Workspace::new_with_roots(PathBuf::from(&target.path), root).map_err(tool_err)?;
        let host_policy = PolicySettings::from_runtime(&host.runtime);
        let target_policy = PolicySettings::from_runtime(&target.runtime);
        let policy = intersect_policies(&host_policy, &target_policy);
        let context = match identity {
            Some(identity) => ToolContext::try_from_workspace_with_identity(
                workspace,
                host.auth.clone(),
                policy.clone(),
                target.runtime.tool_profile.clone(),
                policy.permission_mode.clone(),
                identity,
            ),
            None => ToolContext::try_from_workspace(
                workspace,
                host.auth.clone(),
                policy.clone(),
                target.runtime.tool_profile.clone(),
                policy.permission_mode.clone(),
            ),
        }
        .map_err(tool_err)?;
        Ok(Arc::new(context))
    }

    fn binding_snapshot(
        &self,
        session_key: &str,
    ) -> Result<(WorkspaceProfile, WorkspaceProfile, BindingSnapshot), Value> {
        let profiles = self.profiles();
        let host = self.host(&profiles)?;
        let mut bindings = self.bindings.lock().expect("gateway binding lock");
        evict_expired(&mut bindings);
        let Some(binding) = bindings.get_mut(session_key) else {
            return Err(tool_err_code(
                "workspace_not_selected",
                "Select a workspace for this MCP session first.",
                "gateway",
            ));
        };
        let Some(target) = profiles.get(&binding.workspace_id).cloned() else {
            bindings.remove(session_key);
            return Err(tool_err_code(
                "workspace_unavailable",
                "Selected workspace is unavailable.",
                "gateway",
            ));
        };
        if !host
            .gateway
            .workspace_ids
            .iter()
            .any(|id| id == &binding.workspace_id)
        {
            bindings.remove(session_key);
            return Err(tool_err_code(
                "workspace_not_allowed",
                "Selected workspace is no longer in the host allowlist.",
                "gateway",
            ));
        }
        if binding_fingerprint(&host, &target) != binding.fingerprint
            || !PathBuf::from(&target.path).is_dir()
        {
            bindings.remove(session_key);
            return Err(tool_err_code(
                "workspace_changed",
                "Workspace configuration changed; select it again.",
                "gateway",
            ));
        }
        binding.last_used = Instant::now();
        let snapshot = BindingSnapshot {
            workspace_id: binding.workspace_id.clone(),
            fingerprint: binding.fingerprint.clone(),
            context: binding.context.clone(),
            context_lock: binding.context_lock.clone(),
            lock_key: binding.lock_key.clone(),
            session_key_source: binding.session_key_source,
        };
        Ok((host, target, snapshot))
    }

    #[allow(dead_code)]
    pub fn tool_result(&self, session_key: &str, name: &str, args: &Value) -> Value {
        self.tool_result_with_source(session_key, name, args, SessionKeySource::McpTransport)
    }

    pub fn tool_result_with_source(
        &self,
        session_key: &str,
        name: &str,
        args: &Value,
        source: SessionKeySource,
    ) -> Value {
        match name {
            "list_workspaces" => self.list_workspaces(),
            "bind_workspace" => args
                .get("workspace_hint")
                .and_then(Value::as_str)
                .map(|hint| self.bind_workspace(session_key, hint, source))
                .unwrap_or_else(|| {
                    tool_err_code(
                        "workspace_hint_required",
                        "workspace_hint is required.",
                        "validation",
                    )
                }),
            "select_workspace" => args
                .get("workspace_id")
                .and_then(Value::as_str)
                .map(|id| self.select_workspace_with_source(session_key, id, source))
                .unwrap_or_else(|| {
                    tool_err_code(
                        "INVALID_ARGUMENT",
                        "workspace_id is required.",
                        "validation",
                    )
                }),
            "get_selected_workspace" => self.selected_workspace(session_key),
            "pin_workspace_context" => self.pin_workspace_context(session_key, args),
            "get_workspace_context" => self.get_workspace_context(session_key),
            "unpin_workspace_context" => self.unpin_workspace_context(session_key, args),
            _ => tool_err_code("INVALID_ARGUMENT", "Unknown gateway tool.", "validation"),
        }
    }

    pub fn gateway_info(&self, session_key: Option<&str>) -> Value {
        let selected = session_key
            .map(|key| self.selected_workspace(key))
            .unwrap_or_else(|| json!({"selected": false}));
        json!({
            "enabled": self.is_enabled(),
            "host_workspace_id": self.host_workspace_id,
            "selected_workspace": selected
        })
    }
}

#[derive(Clone)]
struct BindingView {
    workspace_id: String,
    session_key_source: SessionKeySource,
}

fn same_binding_context(left: &BindingSnapshot, right: &BindingSnapshot) -> bool {
    left.workspace_id == right.workspace_id
        && left.fingerprint == right.fingerprint
        && left.lock_key == right.lock_key
        && Arc::ptr_eq(&left.context, &right.context)
}

fn normalize_session_identifier(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn evict_expired_aliases(aliases: &mut HashMap<String, SessionAlias>) {
    let now = Instant::now();
    aliases.retain(|_, alias| now.duration_since(alias.last_used) <= BINDING_TTL);
}

fn trim_aliases(aliases: &mut HashMap<String, SessionAlias>) {
    if aliases.len() <= MAX_BINDINGS {
        return;
    }
    let remove_count = aliases.len() - MAX_BINDINGS;
    let mut oldest = aliases
        .iter()
        .map(|(key, alias)| (key.clone(), alias.last_used))
        .collect::<Vec<_>>();
    oldest.sort_by_key(|(_, last_used)| *last_used);
    for (key, _) in oldest.into_iter().take(remove_count) {
        aliases.remove(&key);
    }
}

fn workspace_lock_key(path: &str) -> String {
    let canonical = PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path));
    canonical
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn normalized_path(path: &str) -> String {
    workspace_lock_key(path)
}

fn workspace_hint_matches(profile: &WorkspaceProfile, hint: &str) -> bool {
    if profile.id == hint {
        return true;
    }
    if profile.name.eq_ignore_ascii_case(hint) {
        return true;
    }
    let profile_path = PathBuf::from(&profile.path);
    let hint_path = PathBuf::from(hint);
    if normalized_path(&profile.path) == normalized_path(hint) {
        return true;
    }
    profile_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.eq_ignore_ascii_case(
                hint_path
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or(hint),
            )
        })
}

fn binding_success(
    target: &WorkspaceProfile,
    source: SessionKeySource,
    session_key: &str,
) -> Value {
    tool_ok(json!({
        "ok": true,
        "workspace": public_workspace(target),
        "binding": {
            "locked": true,
            "session_key_source": source.as_str()
        },
        "history_session_bootstrap": {
            "required": true,
            "tool": "history_session_bootstrap",
            "next": "Initialize or restore this workspace history session before project work."
        },
        "session_hash": GatewayRouter::session_hash(session_key)
    }))
}

fn evict_expired(bindings: &mut HashMap<String, Binding>) {
    let now = Instant::now();
    bindings.retain(|_, binding| now.duration_since(binding.last_used) <= BINDING_TTL);
}

fn public_workspace(profile: &WorkspaceProfile) -> Value {
    json!({
        "id": profile.id,
        "name": profile.name,
        "path": profile.path,
        "available": PathBuf::from(&profile.path).is_dir()
    })
}

fn profile_fingerprint(profile: &WorkspaceProfile) -> String {
    let path = PathBuf::from(&profile.path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&profile.path));
    let material = json!({
        "path": path.to_string_lossy(),
        "tool_profile": profile.runtime.tool_profile,
        "permission_mode": profile.runtime.permission_mode,
        "allowed_commands": profile.runtime.allowed_commands,
        "workspace_local_entries": profile.runtime.workspace_local_entries,
        "workspace_script_extensions": profile.runtime.workspace_script_extensions
    });
    let mut digest = Sha256::new();
    digest.update(material.to_string().as_bytes());
    format!("{:x}", digest.finalize())
}

fn binding_fingerprint(host: &WorkspaceProfile, target: &WorkspaceProfile) -> String {
    let material = json!({
        "host": profile_fingerprint(host),
        "target": profile_fingerprint(target)
    });
    let mut digest = Sha256::new();
    digest.update(material.to_string().as_bytes());
    format!("{:x}", digest.finalize())
}

fn intersect_policies(host: &PolicySettings, target: &PolicySettings) -> PolicySettings {
    let allowed_commands = host
        .allowed_commands
        .intersection(&target.allowed_commands)
        .cloned()
        .collect();
    let workspace_script_extensions = host
        .workspace_script_extensions
        .intersection(&target.workspace_script_extensions)
        .cloned()
        .collect();
    PolicySettings {
        allowed_commands,
        workspace_local_entries: host.workspace_local_entries && target.workspace_local_entries,
        workspace_script_extensions,
        max_patch_bytes: host.max_patch_bytes.min(target.max_patch_bytes),
        permission_mode: restrictive_permission_mode(
            &host.permission_mode,
            &target.permission_mode,
        ),
    }
}

fn restrictive_permission_mode(host: &str, target: &str) -> String {
    if host == target {
        return host.to_string();
    }
    if host == "restricted" || target == "restricted" {
        return "restricted".into();
    }
    if host == "trusted" || target == "trusted" {
        return "trusted".into();
    }
    "restricted".into()
}

pub fn gateway_tools() -> Vec<Value> {
    let mut tools = vec![
        json!({
            "name": "list_workspaces",
            "title": "List workspaces",
            "description": "List explicitly allowlisted local workspaces available to this gateway.",
            "inputSchema": {"type": "object", "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        }),
        json!({
            "name": "bind_workspace",
            "title": "Bind workspace",
            "description": "Bind this ChatGPT conversation to an allowlisted workspace by exact ID, case-insensitive name, full path, or final directory name.",
            "inputSchema": {
                "type": "object",
                "required": ["workspace_hint"],
                "properties": {"workspace_hint": {"type": "string", "minLength": 1}},
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        }),
        json!({
            "name": "select_workspace",
            "title": "Select workspace",
            "description": "Bind this MCP conversation to one explicitly allowlisted workspace.",
            "inputSchema": {"type": "object", "required": ["workspace_id"], "properties": {"workspace_id": {"type": "string", "minLength": 1}}, "additionalProperties": false},
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        }),
        json!({
            "name": "get_selected_workspace",
            "title": "Get selected workspace",
            "description": "Return the workspace bound to this MCP conversation.",
            "inputSchema": {"type": "object", "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        }),
    ];
    tools.extend(workspace_context_gateway::tools());
    tools
}

pub fn is_gateway_tool(name: &str) -> bool {
    workspace_context_gateway::is_tool(name)
        || matches!(
            name,
            "list_workspaces" | "bind_workspace" | "select_workspace" | "get_selected_workspace"
        )
}

#[allow(dead_code)]
pub fn handle_gateway_request(
    router: &GatewayRouter,
    host: &SharedState,
    body: &Value,
    session_key: Option<&str>,
) -> Value {
    handle_gateway_request_with_source(router, host, body, session_key, None)
}

pub fn handle_gateway_request_with_source(
    router: &GatewayRouter,
    host: &SharedState,
    body: &Value,
    session_key: Option<&str>,
    session_source: Option<SessionKeySource>,
) -> Value {
    if !router.is_enabled() {
        return handle_request(host, body);
    }
    let method = body.get("method").and_then(Value::as_str).unwrap_or("");
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let mut response = handle_request(host, body);
            if let Some(instructions) = response
                .get("result")
                .and_then(|result| result.get("instructions"))
                .and_then(Value::as_str)
                .map(str::to_string)
            {
                let profiles = router.profiles();
                let prompt = profiles
                    .get(router.host_workspace_id())
                    .map(|profile| profile.gateway.prompt.clone())
                    .unwrap_or_default();
                let gateway_prompt = if prompt.is_empty() {
                    "For a new conversation, inspect the user's first request for an exact workspace name, ID, full path, or final directory name and call bind_workspace first. If no workspace is explicit, call list_workspaces and ask the user to choose. After binding, the workspace is locked for this conversation; initialize its history session before project work."
                } else {
                    prompt.as_str()
                };
                let context_prompt = "For worktree development, call pin_workspace_context with the exact existing worktree root and expected branch before any project tool. While pinned, get_workspace_context is the authoritative execution root: stop on WORKSPACE_CONTEXT_MISMATCH or WORKSPACE_CONTEXT_EXPIRED and never fall back to the configured parent workspace. Before claiming completion, call get_workspace_context, git_status, and git_diff; affected_files without read-back and Git evidence is not proof that a patch reached disk.";
                if let Some(value) = response
                    .get_mut("result")
                    .and_then(|result| result.get_mut("instructions"))
                {
                    *value =
                        Value::String(format!("{gateway_prompt} {context_prompt} {instructions}"));
                }
            }
            response
        }
        "tools/list" => {
            let Some(_session_key) = session_key else {
                return rpc_error(
                    id,
                    "session_required",
                    "MCP session identifier is required.",
                );
            };
            let mut response = handle_request(host, body);
            if let Some(tools) = response
                .get_mut("result")
                .and_then(|result| result.get_mut("tools"))
                .and_then(Value::as_array_mut)
            {
                tools.extend(gateway_tools());
            }
            response
        }
        "tools/call" => {
            let name = body
                .get("params")
                .and_then(|params| params.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = body
                .get("params")
                .and_then(|params| params.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            if is_gateway_tool(name) {
                let Some(session_key) = session_key else {
                    return rpc_error(
                        id,
                        "session_required",
                        "MCP session identifier is required.",
                    );
                };
                let structured = router.tool_result_with_source(
                    session_key,
                    name,
                    &args,
                    session_source.unwrap_or(SessionKeySource::McpTransport),
                );
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": crate::tools::wrap_mcp_tool_result(name, &args, structured)
                });
            }
            let Some(session_key) = session_key else {
                return rpc_error(
                    id,
                    "session_required",
                    "MCP session identifier is required.",
                );
            };
            let target = match router.route_for(session_key, name) {
                Ok(target) => target,
                Err(structured) => {
                    return json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": crate::tools::wrap_mcp_tool_result(name, &args, structured)
                    });
                }
            };
            let mutating = crate::tools::registry::MUTATING_TOOLS
                .contains(&crate::tools::registry::canonical_tool_name(name));
            let Some(_permit) = router.acquire_permit(&target.lock_key, mutating) else {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": crate::tools::wrap_mcp_tool_result(
                        name,
                        &args,
                        tool_err_code(
                            "gateway_busy",
                            "The gateway is busy; retry this request shortly.",
                            "gateway",
                        )
                    )
                });
            };
            let target = match router.route_for(session_key, name) {
                Ok(current)
                    if current.lock_key == target.lock_key
                        && Arc::ptr_eq(&current.context, &target.context) =>
                {
                    current
                }
                Ok(_) => {
                    let structured = tool_err_code(
                        "WORKSPACE_CONTEXT_MISMATCH",
                        "The workspace context changed while this request was waiting; retry after checking the context.",
                        "workspace_context",
                    );
                    return json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": crate::tools::wrap_mcp_tool_result(name, &args, structured)
                    });
                }
                Err(structured) => {
                    return json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": crate::tools::wrap_mcp_tool_result(name, &args, structured)
                    });
                }
            };
            let request_body = body_with_session_metadata(body, session_key);
            let mut response = handle_request(&target.context, &request_body);
            if let Some(context_summary) = target.context_summary {
                if let Some(structured) = response
                    .get_mut("result")
                    .and_then(|result| result.get_mut("structuredContent"))
                    .and_then(Value::as_object_mut)
                {
                    structured.insert("workspace_context".into(), context_summary);
                }
            }
            if name == "server_info" {
                if let Some(structured) = response
                    .get_mut("result")
                    .and_then(|result| result.get_mut("structuredContent"))
                    .and_then(Value::as_object_mut)
                {
                    structured.insert("gateway".into(), router.gateway_info(Some(session_key)));
                }
            }
            response
        }
        _ => handle_request(host, body),
    }
}

fn body_with_session_metadata(body: &Value, session_key: &str) -> Value {
    let method = body.get("method").and_then(Value::as_str).unwrap_or("");
    let tool_name = body
        .get("params")
        .and_then(|params| params.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if method != "tools/call" || !tool_name.starts_with("history_session_") {
        return body.clone();
    }
    let mut request = body.clone();
    let params = request
        .as_object_mut()
        .and_then(|root| root.get_mut("params"))
        .and_then(Value::as_object_mut);
    let Some(params) = params else {
        return request;
    };
    let meta = params
        .entry("_meta")
        .or_insert_with(|| json!({}))
        .as_object_mut();
    if let Some(meta) = meta {
        meta.insert(
            "openai/session".into(),
            Value::String(session_key.to_string()),
        );
    }
    request
}

pub fn rpc_error(id: Value, code: &str, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": -32001, "message": message, "data": {"code": code, "category": "gateway", "retryable": false}}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::workspace_context_tests::{git, repository};
    use crate::workspace::{GatewayConfig, WorkspaceProfile};

    fn profile(
        path: &std::path::Path,
        id: &str,
        enabled: bool,
        allowlist: Vec<String>,
    ) -> WorkspaceProfile {
        let mut profile = WorkspaceProfile::new(path.display().to_string(), Some(id.to_string()));
        profile.id = id.to_string();
        profile.gateway = GatewayConfig {
            enabled,
            workspace_ids: allowlist,
            prompt: String::new(),
        };
        profile
    }

    #[test]
    fn two_sessions_keep_distinct_contexts() {
        let first = tempfile::tempdir().expect("first");
        let second = tempfile::tempdir().expect("second");
        let host = profile(
            first.path(),
            "host",
            true,
            vec!["host".into(), "second".into()],
        );
        let target = profile(second.path(), "second", false, vec![]);
        let router = GatewayRouter::from_profiles(host, vec![target]);
        assert_eq!(router.select_workspace("session-a", "host")["ok"], true);
        assert_eq!(router.select_workspace("session-b", "second")["ok"], true);
        let a = router
            .context_for("session-a", "get_default_cwd")
            .expect("a");
        let b = router
            .context_for("session-b", "get_default_cwd")
            .expect("b");
        assert_ne!(a.workspace_path(), b.workspace_path());
    }

    #[test]
    fn missing_session_is_structured_error() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = profile(workspace.path(), "host", true, vec!["host".into()]);
        let router = GatewayRouter::from_profiles(host, vec![]);
        let response = handle_gateway_request(
            &router,
            &Arc::new(
                ToolContext::for_test(
                    workspace.path().to_path_buf(),
                    workspace.path().to_path_buf(),
                )
                .expect("context"),
            ),
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_default_cwd","arguments":{}}}),
            None,
        );
        assert_eq!(response["error"]["data"]["code"], "session_required");
    }

    #[test]
    fn missing_session_for_tools_list_is_structured_error() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = profile(workspace.path(), "host", true, vec!["host".into()]);
        let router = GatewayRouter::from_profiles(host, vec![]);
        let context = Arc::new(
            ToolContext::for_test(
                workspace.path().to_path_buf(),
                workspace.path().to_path_buf(),
            )
            .expect("context"),
        );
        let response = handle_gateway_request(
            &router,
            &context,
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
            None,
        );
        assert_eq!(response["error"]["data"]["code"], "session_required");
    }

    #[test]
    fn disabled_gateway_delegates_to_original_handler() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = profile(workspace.path(), "host", false, vec![]);
        let router = GatewayRouter::from_profiles(host, vec![]);
        let context = Arc::new(
            ToolContext::for_test(
                workspace.path().to_path_buf(),
                workspace.path().to_path_buf(),
            )
            .expect("context"),
        );
        let response = handle_gateway_request(
            &router,
            &context,
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_default_cwd","arguments":{}}}),
            None,
        );
        assert_eq!(response["result"]["structuredContent"]["ok"], true);
    }

    #[test]
    fn gateway_initialize_instructs_worktree_pin_and_final_verification() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = profile(workspace.path(), "host", true, vec!["host".into()]);
        let router = GatewayRouter::from_profiles(host, vec![]);
        let context = Arc::new(
            ToolContext::for_test(
                workspace.path().to_path_buf(),
                workspace.path().to_path_buf(),
            )
            .expect("context"),
        );
        let response = handle_gateway_request(
            &router,
            &context,
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
            Some("session"),
        );
        let instructions = response["result"]["instructions"]
            .as_str()
            .expect("instructions");
        assert!(instructions.contains("pin_workspace_context"));
        assert!(instructions.contains("get_workspace_context"));
        assert!(instructions.contains("WORKSPACE_CONTEXT_MISMATCH"));
        assert!(instructions.contains("git_status"));
        assert!(instructions.contains("affected_files"));
    }

    #[test]
    fn policy_intersection_uses_minimum_patch_limit() {
        let host = PolicySettings {
            max_patch_bytes: 10,
            ..PolicySettings::default()
        };
        let target = PolicySettings {
            max_patch_bytes: 20,
            ..PolicySettings::default()
        };
        assert_eq!(intersect_policies(&host, &target).max_patch_bytes, 10);
    }

    #[test]
    fn host_policy_change_revokes_existing_binding() {
        let host_dir = tempfile::tempdir().expect("host");
        let target_dir = tempfile::tempdir().expect("target");
        let host = profile(
            host_dir.path(),
            "host",
            true,
            vec!["host".into(), "target".into()],
        );
        let target = profile(target_dir.path(), "target", false, vec![]);
        let mut router = GatewayRouter::from_profiles(host, vec![target]);
        assert_eq!(router.select_workspace("session", "target")["ok"], true);

        let router_mut = Arc::get_mut(&mut router).expect("unique router");
        let ProfileSource::Static(profiles) = &mut router_mut.source else {
            panic!("expected static profiles");
        };
        profiles
            .get_mut("host")
            .expect("host profile")
            .runtime
            .permission_mode = "restricted".into();

        let error = match router.context_for("session", "get_default_cwd") {
            Ok(_) => panic!("stale host policy should revoke the binding"),
            Err(error) => error,
        };
        assert_eq!(error["error"]["code"], "workspace_changed");
    }

    #[test]
    fn removing_target_from_allowlist_revokes_existing_binding() {
        let host_dir = tempfile::tempdir().expect("host");
        let target_dir = tempfile::tempdir().expect("target");
        let host = profile(
            host_dir.path(),
            "host",
            true,
            vec!["host".into(), "target".into()],
        );
        let target = profile(target_dir.path(), "target", false, vec![]);
        let mut router = GatewayRouter::from_profiles(host, vec![target]);
        assert_eq!(router.select_workspace("session", "target")["ok"], true);

        let router_mut = Arc::get_mut(&mut router).expect("unique router");
        let ProfileSource::Static(profiles) = &mut router_mut.source else {
            panic!("expected static profiles");
        };
        profiles
            .get_mut("host")
            .expect("host profile")
            .gateway
            .workspace_ids = vec!["host".into()];

        let error = match router.context_for("session", "get_default_cwd") {
            Ok(_) => panic!("removed workspace should revoke the binding"),
            Err(error) => error,
        };
        assert_eq!(error["error"]["code"], "workspace_not_allowed");
    }

    #[test]
    fn removing_target_profile_clears_binding_for_reselection() {
        let host_dir = tempfile::tempdir().expect("host");
        let target_dir = tempfile::tempdir().expect("target");
        let host = profile(
            host_dir.path(),
            "host",
            true,
            vec!["host".into(), "target".into()],
        );
        let target = profile(target_dir.path(), "target", false, vec![]);
        let mut router = GatewayRouter::from_profiles(host, vec![target]);
        assert_eq!(router.select_workspace("session", "target")["ok"], true);

        let router_mut = Arc::get_mut(&mut router).expect("unique router");
        let ProfileSource::Static(profiles) = &mut router_mut.source else {
            panic!("expected static profiles");
        };
        profiles.remove("target");

        let unavailable = router.selected_workspace("session");
        assert_eq!(unavailable["error"]["code"], "workspace_not_selected");
        assert_eq!(router.select_workspace("session", "host")["ok"], true);
    }

    #[test]
    fn openai_identity_precedes_transport_and_keeps_alias() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = profile(workspace.path(), "host", true, vec!["host".into()]);
        let router = GatewayRouter::from_profiles(host, vec![]);

        let initialized = router
            .resolve_session_identity(
                SessionIdentifiers {
                    openai: Some("conversation-a".into()),
                    transport: Some("transport-1".into()),
                },
                true,
            )
            .expect("identity");
        assert_eq!(initialized.key, "conversation-a");
        assert_eq!(initialized.transport_id.as_deref(), Some("transport-1"));
        assert_eq!(initialized.source, SessionKeySource::OpenAiConversation);

        let reconnected = router
            .resolve_session_identity(
                SessionIdentifiers {
                    openai: None,
                    transport: Some("transport-1".into()),
                },
                false,
            )
            .expect("transport alias");
        assert_eq!(reconnected.key, "conversation-a");
        assert_eq!(reconnected.source, SessionKeySource::McpTransport);
    }

    #[test]
    fn conflicting_known_session_aliases_are_rejected() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = profile(workspace.path(), "host", true, vec!["host".into()]);
        let router = GatewayRouter::from_profiles(host, vec![]);
        router
            .resolve_session_identity(
                SessionIdentifiers {
                    openai: Some("conversation-a".into()),
                    transport: Some("transport-a".into()),
                },
                true,
            )
            .expect("first identity");
        router
            .resolve_session_identity(
                SessionIdentifiers {
                    openai: Some("conversation-b".into()),
                    transport: Some("transport-b".into()),
                },
                true,
            )
            .expect("second identity");

        let error = router
            .resolve_session_identity(
                SessionIdentifiers {
                    openai: Some("conversation-a".into()),
                    transport: Some("transport-b".into()),
                },
                false,
            )
            .expect_err("conflicting aliases");
        assert_eq!(error["error"]["code"], "session_identity_conflict");
    }

    #[test]
    fn new_openai_identity_wins_when_transport_alias_is_reused() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = profile(workspace.path(), "host", true, vec!["host".into()]);
        let router = GatewayRouter::from_profiles(host, vec![]);

        router
            .resolve_session_identity(
                SessionIdentifiers {
                    openai: Some("conversation-a".into()),
                    transport: Some("reused-transport".into()),
                },
                true,
            )
            .expect("first identity");
        let second = router
            .resolve_session_identity(
                SessionIdentifiers {
                    openai: Some("conversation-b".into()),
                    transport: Some("reused-transport".into()),
                },
                true,
            )
            .expect("new OpenAI identity should win");
        assert_eq!(second.key, "conversation-b");

        let stale_pair = router
            .resolve_session_identity(
                SessionIdentifiers {
                    openai: Some("conversation-a".into()),
                    transport: Some("reused-transport".into()),
                },
                false,
            )
            .expect_err("stale transport pair must be rejected");
        assert_eq!(stale_pair["error"]["code"], "session_identity_conflict");

        let transport_only = router
            .resolve_session_identity(
                SessionIdentifiers {
                    openai: None,
                    transport: Some("reused-transport".into()),
                },
                false,
            )
            .expect("transport alias should follow the new conversation");
        assert_eq!(transport_only.key, "conversation-b");
    }

    #[test]
    fn initialize_without_identifiers_generates_transport_id() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = profile(workspace.path(), "host", true, vec!["host".into()]);
        let router = GatewayRouter::from_profiles(host, vec![]);
        let resolved = router
            .resolve_session_identity(SessionIdentifiers::default(), true)
            .expect("generated identity");
        assert_eq!(resolved.source, SessionKeySource::ServerGenerated);
        assert_eq!(
            resolved.transport_id.as_deref(),
            Some(resolved.key.as_str())
        );
        assert!(!resolved.key.is_empty());
    }

    #[test]
    fn bind_workspace_matches_name_and_locks_conversation() {
        let host_dir = tempfile::tempdir().expect("host");
        let target_dir = tempfile::tempdir().expect("target");
        let host = profile(
            host_dir.path(),
            "host",
            true,
            vec!["host".into(), "target".into()],
        );
        let target = profile(target_dir.path(), "target", false, vec![]);
        let router = GatewayRouter::from_profiles(host, vec![target]);

        let bound = router.bind_workspace(
            "conversation",
            "TARGET",
            SessionKeySource::OpenAiConversation,
        );
        assert_eq!(bound["ok"], true);
        assert_eq!(bound["binding"]["locked"], true);
        assert_eq!(
            bound["binding"]["session_key_source"],
            "openai_conversation"
        );

        let locked =
            router.bind_workspace("conversation", "host", SessionKeySource::OpenAiConversation);
        assert_eq!(locked["error"]["code"], "workspace_locked");
    }

    #[test]
    fn separate_conversations_bind_by_workspace_id() {
        let first_dir = tempfile::tempdir().expect("first");
        let second_dir = tempfile::tempdir().expect("second");
        let host = profile(
            first_dir.path(),
            "host",
            true,
            vec!["host".into(), "second".into()],
        );
        let mut target = profile(second_dir.path(), "second", false, vec![]);
        target.name = "Build Task".into();
        let router = GatewayRouter::from_profiles(host, vec![target]);

        let first = router.bind_workspace(
            "conversation-a",
            "host",
            SessionKeySource::OpenAiConversation,
        );
        let second = router.bind_workspace(
            "conversation-b",
            "second",
            SessionKeySource::OpenAiConversation,
        );
        assert_eq!(first["workspace"]["id"], "host");
        assert_eq!(second["workspace"]["id"], "second");
        assert_ne!(
            router
                .context_for("conversation-a", "get_default_cwd")
                .expect("first context")
                .workspace_path(),
            router
                .context_for("conversation-b", "get_default_cwd")
                .expect("second context")
                .workspace_path()
        );
    }

    #[test]
    fn pinned_context_routes_patch_to_worktree_and_unpins_to_configured_root() {
        let (configured, _main, worktree) = repository();
        let host = profile(configured.path(), "host", true, vec!["host".into()]);
        let router = GatewayRouter::from_profiles(host, vec![]);
        assert_eq!(router.select_workspace("session", "host")["ok"], true);

        let pinned = router.tool_result(
            "session",
            "pin_workspace_context",
            &json!({
                "path": worktree,
                "expected_branch": "feature/context-lock",
                "expires_in_minutes": 5
            }),
        );
        assert_eq!(pinned["ok"], true);
        assert_eq!(pinned["locked"], true);
        let active = router
            .context_for("session", "apply_patch")
            .expect("pinned context");
        assert_eq!(
            active.workspace.root().canonicalize().expect("active root"),
            worktree.canonicalize().expect("worktree root")
        );
        assert_eq!(
            active
                .workspace
                .repository_root()
                .canonicalize()
                .expect("repository root"),
            configured.path().canonicalize().expect("configured root")
        );
        assert_eq!(
            active
                .execution_root()
                .canonicalize()
                .expect("execution root"),
            worktree.canonicalize().expect("worktree root")
        );

        let host_context = Arc::new(
            ToolContext::for_test(
                configured.path().to_path_buf(),
                configured.path().join("harness"),
            )
            .expect("host context"),
        );
        let response = handle_gateway_request(
            &router,
            &host_context,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "apply_patch",
                    "arguments": {
                        "patch": "*** Begin Patch\n*** Add File: pinned.txt\n+pinned-root\n*** End Patch\n"
                    }
                }
            }),
            Some("session"),
        );
        let structured = &response["result"]["structuredContent"];
        assert_eq!(structured["ok"], true);
        assert_eq!(structured["verified"], true);
        assert_eq!(structured["workspace_context"]["status"], "valid");
        assert!(worktree.join("pinned.txt").is_file());
        assert!(!configured.path().join("pinned.txt").exists());

        let unpinned = router.tool_result(
            "session",
            "unpin_workspace_context",
            &json!({"confirm": true}),
        );
        assert_eq!(unpinned["ok"], true);
        assert_eq!(unpinned["locked"], false);
        let restored = router
            .context_for("session", "get_default_cwd")
            .expect("restored context");
        assert_eq!(
            restored
                .workspace
                .root()
                .canonicalize()
                .expect("restored root"),
            configured.path().canonicalize().expect("configured root")
        );
    }

    #[test]
    fn pinned_context_routes_history_to_active_worktree_when_workspace_root_is_repository() {
        let (configured, _main, worktree) = repository();
        let host = profile(configured.path(), "host", true, vec!["host".into()]);
        let router = GatewayRouter::from_profiles(host, vec![]);
        assert_eq!(router.select_workspace("session", "host")["ok"], true);

        let pinned = router.tool_result(
            "session",
            "pin_workspace_context",
            &json!({
                "path": worktree,
                "expected_branch": "feature/context-lock",
                "expires_in_minutes": 5
            }),
        );
        assert_eq!(pinned["ok"], true);

        let host_context = Arc::new(
            ToolContext::for_test(
                configured.path().to_path_buf(),
                configured.path().join("harness"),
            )
            .expect("host context"),
        );
        let response = handle_gateway_request(
            &router,
            &host_context,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "history_session_bootstrap",
                    "arguments": {
                        "workspace_root": configured.path(),
                        "initial_user_input": "bootstrap pinned worktree history"
                    }
                }
            }),
            Some("session"),
        );
        let structured = &response["result"]["structuredContent"];
        assert_eq!(structured["ok"], true);
        assert_eq!(structured["current_path"], "docs/history-session/1.md");
        assert!(worktree.join("docs/history-session/1.md").is_file());
        assert!(!configured.path().join("docs/history-session/1.md").exists());
    }

    #[test]
    fn get_workspace_context_returns_failure_when_pin_has_expired() {
        let (configured, _main, worktree) = repository();
        let host = profile(configured.path(), "host", true, vec!["host".into()]);
        let router = GatewayRouter::from_profiles(host, vec![]);
        assert_eq!(router.select_workspace("session", "host")["ok"], true);
        let pinned = router.tool_result(
            "session",
            "pin_workspace_context",
            &json!({
                "path": worktree,
                "expected_branch": "feature/context-lock",
                "expires_in_minutes": 5
            }),
        );
        assert_eq!(pinned["ok"], true);

        let context_lock = router
            .bindings
            .lock()
            .expect("gateway binding lock")
            .get("session")
            .and_then(|binding| binding.context_lock.clone())
            .expect("pinned context lock");
        context_lock
            .lock()
            .expect("workspace context lock")
            .expire_for_test();

        let result = router.tool_result("session", "get_workspace_context", &json!({}));
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"]["code"], "WORKSPACE_CONTEXT_EXPIRED");
    }

    #[test]
    fn conversations_pin_distinct_worktrees_with_distinct_contexts_and_locks() {
        let (configured, main, first) = repository();
        let second = configured.path().join("second");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-b",
                "feature/second",
                second.to_str().expect("second path"),
            ],
        );
        let host = profile(configured.path(), "host", true, vec!["host".into()]);
        let router = GatewayRouter::from_profiles(host, vec![]);
        assert_eq!(router.select_workspace("one", "host")["ok"], true);
        assert_eq!(router.select_workspace("two", "host")["ok"], true);
        assert_eq!(
            router.tool_result(
                "one",
                "pin_workspace_context",
                &json!({"path": first, "expected_branch": "feature/context-lock"}),
            )["ok"],
            true
        );
        assert_eq!(
            router.tool_result(
                "two",
                "pin_workspace_context",
                &json!({"path": second, "expected_branch": "feature/second"}),
            )["ok"],
            true
        );
        let first_target = router.route_for("one", "get_default_cwd").expect("first");
        let second_target = router.route_for("two", "get_default_cwd").expect("second");
        assert_ne!(first_target.lock_key, second_target.lock_key);
        assert!(!Arc::ptr_eq(&first_target.context, &second_target.context));
        assert_ne!(
            first_target.context.workspace_path(),
            second_target.context.workspace_path()
        );
    }
}
