//! Windows AppContainer and Job Object implementation for `exec_command`.
//!
//! The process is created suspended, assigned to a kill-on-close Job Object,
//! and only then resumed.  The AppContainer SID receives explicit ACL entries
//! for the execution root, the private temporary directory, and the resolved
//! toolchain directories.  No direct-process fallback is exposed by this
//! module.

use std::ffi::{c_void, OsStr, OsString};
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use tokio::fs::File;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, SetHandleInformation, ERROR_ALREADY_EXISTS, HANDLE,
    HANDLE_FLAG_INHERIT, HLOCAL,
};
use windows::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, ACCESS_MODE, DENY_ACCESS,
    EXPLICIT_ACCESS_W, GRANT_ACCESS, REVOKE_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID,
    TRUSTEE_IS_UNKNOWN,
};
use windows::Win32::Security::Isolation::DeriveAppContainerSidFromAppContainerName;
use windows::Win32::Security::{
    DeriveCapabilitySidsFromName, FreeSid, GetLengthSid, DACL_SECURITY_INFORMATION, PSID,
    SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT,
};
use windows::Win32::Storage::FileSystem::{
    FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_READ_ATTRIBUTES,
    FILE_TRAVERSE,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_BASIC_LIMIT_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, ResumeThread, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, INFINITE, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    STARTUPINFOW,
};

use crate::security::exec_sandbox::SandboxPolicy;

const ERROR_SUCCESS: u32 = 0;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const FILE_SCOPE_RIGHTS: u32 = FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0 | FILE_GENERIC_EXECUTE.0;
const TOOLCHAIN_READ_EXECUTE: u32 = FILE_GENERIC_READ.0 | FILE_GENERIC_EXECUTE.0;
// Git for Windows resolves every component of GIT_WORK_TREE before it honors
// GIT_DIR.  Grant only the directory traversal and attribute checks required
// for those components; the actual execution root receives the broader scope
// grant below.
const PARENT_TRAVERSE_RIGHTS: u32 = FILE_TRAVERSE.0 | FILE_READ_ATTRIBUTES.0;

/// A child process with ownership of its Job Object and temporary ACL grants.
pub(crate) struct SandboxProcess {
    // Declaration order is intentional: process/job handles close before ACL
    // cleanup, so a dropped running process cannot race ACL revocation.
    job: OwnedHandle,
    process: OwnedHandle,
    _grants: AclGrants,
    pid: u32,
}

impl std::fmt::Debug for SandboxProcess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SandboxProcess")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

pub(crate) struct SandboxChild {
    process: SandboxProcess,
    stdin: Option<File>,
    stdout: Option<File>,
    stderr: Option<File>,
}

impl SandboxChild {
    pub(crate) fn into_parts(self) -> (SandboxProcess, Option<File>, Option<File>, Option<File>) {
        (self.process, self.stdin, self.stdout, self.stderr)
    }
}

impl SandboxProcess {
    pub(crate) fn id(&self) -> Option<u32> {
        Some(self.pid)
    }

    pub(crate) fn kill_tree(&mut self) -> io::Result<()> {
        unsafe { TerminateJobObject(self.job_handle(), 1) }.map_err(win32_io_error)
    }

    pub(crate) async fn wait(&mut self) -> io::Result<std::process::ExitStatus> {
        let handle = self.process_handle();
        let handle_bits = handle.0 as usize;
        let code =
            tokio::task::spawn_blocking(move || wait_for_exit(HANDLE(handle_bits as *mut _)))
                .await
                .map_err(|error| {
                    io::Error::other(format!("sandbox wait task failed: {error}"))
                })??;
        Ok(std::os::windows::process::ExitStatusExt::from_raw(code))
    }

    pub(crate) fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        let result = unsafe { WaitForSingleObject(self.process_handle(), 0) };
        if result.0 == WAIT_TIMEOUT {
            return Ok(None);
        }
        if result.0 != WAIT_OBJECT_0 {
            return Err(last_io_error("WaitForSingleObject"));
        }
        let code = exit_code(self.process_handle())?;
        Ok(Some(std::os::windows::process::ExitStatusExt::from_raw(
            code,
        )))
    }

    fn process_handle(&self) -> HANDLE {
        HANDLE(self.process.as_raw_handle() as *mut _)
    }

    fn job_handle(&self) -> HANDLE {
        HANDLE(self.job.as_raw_handle() as *mut _)
    }
}

/// Return whether an AppContainer can actually launch a process on this host.
///
/// Deriving an AppContainer SID only proves that the UserEnv API can calculate
/// a value.  The process attribute, profile registration, ACLs, and startup
/// ABI must all work before strict execution is advertised as enforced.
pub(crate) fn is_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        if std::env::var_os("CODING_TOOLS_MCP_DISABLE_APPCONTAINER").is_some() {
            return false;
        }
        probe_appcontainer()
    })
}

/// AppContainer can launch against a workspace only when its volume root
/// already grants the required traversal.  The system volume carries this
/// permission by default; changing a non-system volume root at runtime can
/// trigger a full NTFS inheritance walk, so unsupported volumes fail closed.
pub(crate) fn is_available_for_path(path: &Path) -> bool {
    is_available()
        && volume_root(path)
            .as_deref()
            .is_some_and(is_windows_system_volume)
}

fn probe_appcontainer() -> bool {
    let nonce = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(value) => value.as_nanos(),
        Err(_) => return false,
    };
    let temp_base = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| root.join("Temp"))
        .filter(|path| path.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let root = temp_base.join(format!(
        "coding-tools-mcp-appcontainer-probe-{}-{nonce}",
        std::process::id()
    ));
    if std::fs::create_dir_all(&root).is_err() {
        return false;
    }
    let policy = SandboxPolicy {
        repository_root: root.clone(),
        execution_root: root.clone(),
        startup_directory: root.clone(),
        working_directory: root.clone(),
        writable_roots: vec![root.clone()],
        readonly_roots: Vec::new(),
        temp_root: root.join("tmp"),
        network_allowed: false,
        allow_child_processes: true,
    };
    let shell =
        which::which("cmd.exe").unwrap_or_else(|_| PathBuf::from(r"C:\Windows\System32\cmd.exe"));
    let result = spawn(
        &policy,
        &shell.to_string_lossy(),
        &["/d".into(), "/c".into(), "exit 0".into()],
        &[],
    )
    .map(|child| {
        let (mut process, stdin, stdout, stderr) = child.into_parts();
        drop(stdin);
        drop(stdout);
        drop(stderr);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match process.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                _ => break false,
            }
        }
    })
    .unwrap_or(false);
    let _ = std::fs::remove_dir_all(&root);
    result
}

pub(crate) fn spawn(
    policy: &SandboxPolicy,
    program: &str,
    args: &[String],
    env: &[(String, String)],
) -> io::Result<SandboxChild> {
    std::fs::create_dir_all(&policy.temp_root)?;
    validate_policy(policy)?;

    let profile = AppContainerProfile::create(policy)?;
    let grants = AclGrants::grant(policy, &profile.sid)?;

    let pipes = PipeSet::create()?;
    let mut command_line = command_line(program, args);
    let application = wide(&display_path(program));
    let current_directory = wide(&display_path(&policy.startup_directory.to_string_lossy()));
    let environment = (!env.is_empty()).then(|| environment_block(env));
    let capabilities = profile.security_capabilities(policy.network_allowed)?;
    let handles = vec![pipes.child_stdin, pipes.child_stdout, pipes.child_stderr];
    let attribute_list = AttributeList::new(capabilities, handles)?;

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = pipes.child_stdin;
    startup.StartupInfo.hStdOutput = pipes.child_stdout;
    startup.StartupInfo.hStdError = pipes.child_stderr;
    startup.lpAttributeList = attribute_list.list;

    let mut flags = EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED;
    if environment.is_some() {
        flags |= CREATE_UNICODE_ENVIRONMENT;
    }
    let job = create_job()?;
    let mut process_info = PROCESS_INFORMATION::default();
    let created = unsafe {
        CreateProcessW(
            PCWSTR(application.as_ptr()),
            Some(PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            true,
            flags,
            environment
                .as_ref()
                .map(|value| value.as_ptr() as *const c_void),
            PCWSTR(current_directory.as_ptr()),
            &mut startup as *mut STARTUPINFOEXW as *mut STARTUPINFOW,
            &mut process_info,
        )
    };
    if let Err(error) = created {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("CreateProcessW failed: {error}"),
        ));
    }

    // The attribute list and child-side pipe handles are no longer needed in
    // the parent once CreateProcessW returns.
    drop(attribute_list);
    close_handle(pipes.child_stdin);
    close_handle(pipes.child_stdout);
    close_handle(pipes.child_stderr);

    let process = unsafe { OwnedHandle::from_raw_handle(process_info.hProcess.0 as *mut _) };
    let thread = unsafe { OwnedHandle::from_raw_handle(process_info.hThread.0 as *mut _) };
    if let Err(error) = unsafe {
        AssignProcessToJobObject(HANDLE(job.as_raw_handle() as *mut _), process_info.hProcess)
    } {
        unsafe { TerminateProcess(process_info.hProcess, 1) }.ok();
        drop(thread);
        drop(process);
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("AssignProcessToJobObject failed: {error}"),
        ));
    }
    if unsafe { ResumeThread(process_info.hThread) } == u32::MAX {
        unsafe { TerminateJobObject(HANDLE(job.as_raw_handle() as *mut _), 1) }.ok();
        drop(thread);
        drop(process);
        return Err(last_io_error("ResumeThread"));
    }
    drop(thread);

    let process = SandboxProcess {
        _grants: grants,
        job,
        process,
        pid: process_info.dwProcessId,
    };
    Ok(SandboxChild {
        process,
        stdin: Some(pipes.parent_stdin),
        stdout: Some(pipes.parent_stdout),
        stderr: Some(pipes.parent_stderr),
    })
}

fn validate_policy(policy: &SandboxPolicy) -> io::Result<()> {
    if !policy.execution_root.is_dir()
        || !policy.startup_directory.is_dir()
        || !policy.working_directory.is_dir()
    {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "sandbox execution root or working directory is unavailable",
        ));
    }
    if !path_is_within(&policy.execution_root, &policy.working_directory) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "sandbox working directory is outside execution root",
        ));
    }
    let system_temp = std::env::temp_dir();
    if !path_is_within(&policy.execution_root, &policy.startup_directory)
        && !path_is_within(&policy.temp_root, &policy.startup_directory)
        && !path_is_within(&system_temp, &policy.startup_directory)
        && !is_windows_system_path(&policy.startup_directory)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "sandbox startup directory is outside authorized roots",
        ));
    }
    if path_is_within(&policy.repository_root, &policy.execution_root)
        && path_is_within(&policy.execution_root, &policy.repository_root)
        && !path_is_same(&policy.repository_root, &policy.execution_root)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "sandbox roots are inconsistent",
        ));
    }
    Ok(())
}

fn create_job() -> io::Result<OwnedHandle> {
    let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(win32_io_error)?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
            LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &mut limits as *mut _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    }
    .map_err(win32_io_error)?;
    Ok(unsafe { OwnedHandle::from_raw_handle(job.0 as *mut _) })
}

struct PipeSet {
    child_stdin: HANDLE,
    parent_stdin: File,
    child_stdout: HANDLE,
    parent_stdout: File,
    child_stderr: HANDLE,
    parent_stderr: File,
}

impl PipeSet {
    fn create() -> io::Result<Self> {
        let mut stdin_read = HANDLE::default();
        let mut stdin_write = HANDLE::default();
        let mut stdout_read = HANDLE::default();
        let mut stdout_write = HANDLE::default();
        let mut stderr_read = HANDLE::default();
        let mut stderr_write = HANDLE::default();
        let security = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            bInheritHandle: true.into(),
            ..Default::default()
        };
        let result = unsafe {
            CreatePipe(
                &mut stdin_read,
                &mut stdin_write,
                Some(&security as *const _),
                0,
            )
            .and_then(|_| {
                CreatePipe(
                    &mut stdout_read,
                    &mut stdout_write,
                    Some(&security as *const _),
                    0,
                )
            })
            .and_then(|_| {
                CreatePipe(
                    &mut stderr_read,
                    &mut stderr_write,
                    Some(&security as *const _),
                    0,
                )
            })
        };
        if let Err(error) = result {
            for handle in [
                stdin_read,
                stdin_write,
                stdout_read,
                stdout_write,
                stderr_read,
                stderr_write,
            ] {
                close_handle(handle);
            }
            return Err(win32_io_error(error));
        }

        // Only the child-side handles remain inheritable.
        for handle in [stdin_write, stdout_read, stderr_read] {
            if let Err(error) =
                unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, Default::default()) }
            {
                for remaining in [
                    stdin_read,
                    stdin_write,
                    stdout_read,
                    stdout_write,
                    stderr_read,
                    stderr_write,
                ] {
                    close_handle(remaining);
                }
                return Err(win32_io_error(error));
            }
        }

        let parent_stdin = match file_from_handle(stdin_write) {
            Ok(file) => file,
            Err(error) => {
                close_handle(stdin_read);
                close_handle(stdin_write);
                close_handle(stdout_read);
                close_handle(stdout_write);
                close_handle(stderr_read);
                close_handle(stderr_write);
                return Err(error);
            }
        };
        let parent_stdout = match file_from_handle(stdout_read) {
            Ok(file) => file,
            Err(error) => {
                close_handle(stdin_read);
                close_handle(stdout_read);
                close_handle(stdout_write);
                close_handle(stderr_read);
                close_handle(stderr_write);
                return Err(error);
            }
        };
        let parent_stderr = match file_from_handle(stderr_read) {
            Ok(file) => file,
            Err(error) => {
                close_handle(stdin_read);
                close_handle(stderr_read);
                close_handle(stdout_write);
                close_handle(stderr_write);
                return Err(error);
            }
        };

        Ok(Self {
            child_stdin: stdin_read,
            parent_stdin,
            child_stdout: stdout_write,
            parent_stdout,
            child_stderr: stderr_write,
            parent_stderr,
        })
    }
}

struct AppContainerProfile {
    sid: Vec<u8>,
}

impl AppContainerProfile {
    fn create(policy: &SandboxPolicy) -> io::Result<Self> {
        use sha2::{Digest, Sha256};
        let mut digest = Sha256::new();
        digest.update(policy.execution_root.to_string_lossy().as_bytes());
        digest.update(std::process::id().to_le_bytes());
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        digest.update(nonce.to_le_bytes());
        let hash = digest.finalize();
        let suffix = hash
            .iter()
            .take(12)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let name = wide(&format!("CodingToolsMCP-{suffix}"));
        // Create the profile before deriving its SID.  Derive... intentionally
        // returns a deterministic SID even when no profile exists, but
        // CreateProcess requires the profile to be registered with UserEnv.
        let raw = match create_profile_sid(&name) {
            Ok(sid) => sid,
            Err(error)
                if error.code() == windows::core::HRESULT::from_win32(ERROR_ALREADY_EXISTS.0) =>
            {
                unsafe { DeriveAppContainerSidFromAppContainerName(PCWSTR(name.as_ptr())) }
                    .map_err(win32_io_error)?
            }
            Err(error) => return Err(win32_io_error(error)),
        };
        let length = unsafe { GetLengthSid(raw) } as usize;
        if length == 0 {
            unsafe { FreeSid(raw) };
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid AppContainer SID",
            ));
        }
        let sid = unsafe { std::slice::from_raw_parts(raw.0 as *const u8, length) }.to_vec();
        unsafe { FreeSid(raw) };
        Ok(Self { sid })
    }

    fn security_capabilities(&self, network_allowed: bool) -> io::Result<SecurityCapabilities> {
        let mut capability_sids = Vec::new();
        if network_allowed {
            capability_sids.push(derive_capability_sid("internetClient")?);
        }
        Ok(SecurityCapabilities {
            app_sid: self.sid.clone(),
            capability_sids,
            attributes: Vec::new(),
        })
    }
}

struct SecurityCapabilities {
    // Own every allocation referenced by SECURITY_CAPABILITIES.  The raw
    // structure is passed to UpdateProcThreadAttribute only while this value
    // is held by AttributeList.
    app_sid: Vec<u8>,
    capability_sids: Vec<Vec<u8>>,
    attributes: Vec<SID_AND_ATTRIBUTES>,
}

impl SecurityCapabilities {
    fn as_raw(&mut self) -> SECURITY_CAPABILITIES {
        self.attributes = self
            .capability_sids
            .iter()
            .map(|sid| SID_AND_ATTRIBUTES {
                Sid: PSID(sid.as_ptr() as *mut _),
                Attributes: 0x0000_0004,
            })
            .collect::<Vec<_>>();
        let capabilities = if self.attributes.is_empty() {
            null_mut()
        } else {
            self.attributes.as_mut_ptr()
        };
        SECURITY_CAPABILITIES {
            AppContainerSid: PSID(self.app_sid.as_ptr() as *mut _),
            Capabilities: capabilities,
            CapabilityCount: self.capability_sids.len() as u32,
            Reserved: 0,
        }
    }
}

struct AttributeList {
    list: windows::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST,
    _storage: Vec<usize>,
    // Keep the backing SIDs and SID_AND_ATTRIBUTES array alive until after
    // CreateProcessW returns.  The attribute API copies the outer structure,
    // but retains pointers to these inner allocations.
    _capabilities: SecurityCapabilities,
    capabilities: Box<SECURITY_CAPABILITIES>,
    _handle_list: Vec<HANDLE>,
}

impl AttributeList {
    fn new(mut capabilities: SecurityCapabilities, handle_list: Vec<HANDLE>) -> io::Result<Self> {
        let mut size = 0usize;
        let attribute_count = 1 + u32::from(!handle_list.is_empty());
        let _ =
            unsafe { InitializeProcThreadAttributeList(None, attribute_count, None, &mut size) };
        if size == 0 {
            return Err(last_io_error("InitializeProcThreadAttributeList(size)"));
        }
        // The attribute list requires pointer alignment.  A Vec<u8> only has
        // byte alignment in Rust even though most allocators happen to return
        // a wider-aligned address.
        let words = size.div_ceil(size_of::<usize>());
        let mut storage = vec![0usize; words];
        let list = windows::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST(
            storage.as_mut_ptr() as *mut _,
        );
        unsafe { InitializeProcThreadAttributeList(Some(list), attribute_count, None, &mut size) }
            .map_err(win32_io_error)?;
        let raw_capabilities = capabilities.as_raw();
        let attribute_list = Self {
            list,
            _storage: storage,
            _capabilities: capabilities,
            capabilities: Box::new(raw_capabilities),
            _handle_list: handle_list,
        };
        unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                Some(attribute_list.capabilities.as_ref() as *const _ as *const _),
                size_of::<SECURITY_CAPABILITIES>(),
                None,
                None,
            )
        }
        .map_err(win32_io_error)?;
        if !attribute_list._handle_list.is_empty() {
            let bytes = size_of::<HANDLE>() * attribute_list._handle_list.len();
            unsafe {
                UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    Some(attribute_list._handle_list.as_ptr() as *const c_void),
                    bytes,
                    None,
                    None,
                )
            }
            .map_err(win32_io_error)?;
        }
        Ok(attribute_list)
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.list) };
    }
}

#[derive(Default)]
struct AclGrant {
    path: PathBuf,
    inheritance: windows::Win32::Security::ACE_FLAGS,
}

#[derive(Default)]
struct AclGrants {
    entries: Vec<AclGrant>,
    sid: Vec<u8>,
    // Only full allow grants participate in root coalescing. Parent
    // traversal entries intentionally have no inheritance and must not mask a
    // later read/execute grant for a child metadata directory.
    allow_roots: Vec<PathBuf>,
}

impl AclGrants {
    fn grant(policy: &SandboxPolicy, sid: &[u8]) -> io::Result<Self> {
        let mut grants = Self {
            entries: Vec::new(),
            sid: sid.to_vec(),
            allow_roots: Vec::new(),
        };
        let result = (|| {
            // AppContainer needs a non-inherited traverse grant on the
            // immediate parent of each scope.  A linked worktree has an
            // additional repository parent, which is handled explicitly so
            // Git can resolve the worktree metadata without granting access to
            // the repository tree itself.
            grant_parent_traversal(&policy.execution_root, &mut grants)?;
            if !path_is_same(&policy.repository_root, &policy.execution_root) {
                grant_parent_traversal(&policy.repository_root, &mut grants)?;
            }
            grant_parent_traversal(&policy.temp_root, &mut grants)?;
            for root in &policy.readonly_roots {
                grant_parent_traversal(root, &mut grants)?;
            }

            // A repository root can contain sibling worktrees.  A linked
            // worktree's shared Git metadata lives under the configured
            // repository root, so that one root must remain reachable for
            // read-only metadata access.  Other sibling worktrees are denied
            // explicitly before their paths receive any inherited grants.
            for sibling in discover_sibling_worktrees(policy) {
                if !path_is_same(&sibling, &policy.execution_root)
                    && !path_is_same(&sibling, &policy.repository_root)
                {
                    grant_acl(&sibling, &grants.sid, FILE_SCOPE_RIGHTS, DENY_ACCESS)?;
                    grants.record(sibling, SUB_CONTAINERS_AND_OBJECTS_INHERIT);
                }
            }
            grant_acl(
                &policy.execution_root,
                &grants.sid,
                FILE_SCOPE_RIGHTS,
                GRANT_ACCESS,
            )?;
            grants.record_allow(
                policy.execution_root.clone(),
                SUB_CONTAINERS_AND_OBJECTS_INHERIT,
            );

            if !path_is_same(&policy.temp_root, &policy.execution_root) {
                grant_acl(
                    &policy.temp_root,
                    &grants.sid,
                    FILE_SCOPE_RIGHTS,
                    GRANT_ACCESS,
                )?;
                grants.record_allow(policy.temp_root.clone(), SUB_CONTAINERS_AND_OBJECTS_INHERIT);
            }
            for root in &policy.readonly_roots {
                // Windows system binaries already carry the AppContainer read/
                // execute ACE. Attempting to rewrite the protected Windows tree
                // would fail with ACCESS_DENIED and is unnecessary.
                // Parent traversal entries are deliberately non-inherited and
                // therefore do not satisfy a child read grant. Coalesce only
                // against an existing full allow root, never a traversal
                // entry.
                if root.exists()
                    && !is_windows_system_path(root)
                    && !grants
                        .allow_roots
                        .iter()
                        .any(|ancestor| path_is_within(ancestor, root))
                {
                    grant_acl(root, &grants.sid, TOOLCHAIN_READ_EXECUTE, GRANT_ACCESS)?;
                    grants.record_allow(root.clone(), SUB_CONTAINERS_AND_OBJECTS_INHERIT);
                }
            }
            Ok::<(), io::Error>(())
        })();
        result.map(|()| grants)
    }

    fn record(&mut self, path: PathBuf, inheritance: windows::Win32::Security::ACE_FLAGS) {
        self.entries.push(AclGrant { path, inheritance });
    }

    fn record_allow(&mut self, path: PathBuf, inheritance: windows::Win32::Security::ACE_FLAGS) {
        self.allow_roots.push(path.clone());
        self.record(path, inheritance);
    }
}

impl Drop for AclGrants {
    fn drop(&mut self) {
        for entry in self.entries.iter().rev() {
            let _ = revoke_acl_with_inheritance(&entry.path, &self.sid, entry.inheritance);
        }
    }
}

fn grant_parent_traversal(path: &Path, grants: &mut AclGrants) -> io::Result<()> {
    // AppContainer path checks traverse every ancestor.  User-profile temp
    // directories and linked worktrees commonly sit several levels below a
    // protected root, so granting only the immediate parent still produces a
    // misleading "Invalid path ... Permission denied" from Git.  Walk up to
    // the volume root and grant only non-inherited traverse/attribute rights
    // on each component.  The volume root is required for non-system drives
    // such as D:\; the Windows system volume already exposes the required
    // AppContainer traversal and is deliberately left untouched.
    let mut current = path.parent();
    while let Some(parent) = current {
        if !is_windows_system_path(parent)
            && !(parent.parent().is_none() && is_windows_system_volume(parent))
            && !grants
                .entries
                .iter()
                .any(|entry| path_is_same(&entry.path, parent))
        {
            let inheritance = windows::Win32::Security::ACE_FLAGS(0);
            grant_acl_with_inheritance(
                parent,
                &grants.sid,
                PARENT_TRAVERSE_RIGHTS,
                GRANT_ACCESS,
                inheritance,
            )?;
            grants.record(parent.to_path_buf(), inheritance);
        }
        let Some(next) = parent.parent() else {
            break;
        };
        if path_is_same(next, parent) {
            break;
        }
        current = Some(next);
    }
    Ok(())
}

fn grant_acl(path: &Path, sid: &[u8], rights: u32, mode: ACCESS_MODE) -> io::Result<()> {
    grant_acl_with_inheritance(path, sid, rights, mode, SUB_CONTAINERS_AND_OBJECTS_INHERIT)
}

fn grant_acl_with_inheritance(
    path: &Path,
    sid: &[u8],
    rights: u32,
    mode: ACCESS_MODE,
    inheritance: windows::Win32::Security::ACE_FLAGS,
) -> io::Result<()> {
    let path = wide(&path.to_string_lossy());
    let mut old_dacl = null_mut();
    let mut descriptor = windows::Win32::Security::PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(path.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut old_dacl as *mut _),
            None,
            &mut descriptor,
        )
    };
    if status.0 != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }

    let trustee = windows::Win32::Security::Authorization::TRUSTEE_W {
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_UNKNOWN,
        ptstrName: PWSTR(sid.as_ptr() as *mut _),
        ..Default::default()
    };
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: rights,
        grfAccessMode: mode,
        grfInheritance: inheritance,
        Trustee: trustee,
    };
    let mut new_acl = null_mut();
    let acl_status = unsafe { SetEntriesInAclW(Some(&[entry]), Some(old_dacl), &mut new_acl) };
    if !descriptor.is_invalid() {
        unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    }
    if acl_status.0 != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(acl_status.0 as i32));
    }
    let set_status = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(path.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(new_acl),
            None,
        )
    };
    unsafe { LocalFree(Some(HLOCAL(new_acl as *mut _))) };
    if set_status.0 != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(set_status.0 as i32));
    }
    Ok(())
}

fn revoke_acl_with_inheritance(
    path: &Path,
    sid: &[u8],
    inheritance: windows::Win32::Security::ACE_FLAGS,
) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    grant_acl_with_inheritance(path, sid, 0, REVOKE_ACCESS, inheritance)
}

fn is_windows_system_path(path: &Path) -> bool {
    let Some(system_root) = std::env::var_os("SystemRoot") else {
        return false;
    };
    path_is_within(Path::new(&system_root), path)
}

pub(crate) fn is_windows_system_volume(path: &Path) -> bool {
    let Some(system_root) = std::env::var_os("SystemRoot") else {
        return false;
    };
    let path = display_path(&path.to_string_lossy()).to_ascii_lowercase();
    let system_root =
        display_path(&PathBuf::from(system_root).to_string_lossy()).to_ascii_lowercase();
    path.get(..2) == system_root.get(..2)
}

fn volume_root(path: &Path) -> Option<PathBuf> {
    let value = display_path(&path.to_string_lossy());
    if value.len() >= 2 && value.as_bytes()[1] == b':' {
        return Some(PathBuf::from(format!("{}\\", &value[..2])));
    }
    let mut components = value.trim_start_matches(['\\', '/']).split(['\\', '/']);
    let server = components.next()?;
    let share = components.next()?;
    Some(PathBuf::from(format!(r"\\{server}\{share}\")))
}

fn discover_sibling_worktrees(policy: &SandboxPolicy) -> Vec<PathBuf> {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            &policy.repository_root.to_string_lossy(),
            "worktree",
            "list",
            "--porcelain",
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .filter_map(|path| path.canonicalize().ok())
        .filter(|path| !path_is_same(path, &policy.execution_root))
        .collect()
}

fn derive_capability_sid(name: &str) -> io::Result<Vec<u8>> {
    let name = wide(name);
    let mut groups: *mut PSID = null_mut();
    let mut group_count = 0u32;
    let mut capabilities: *mut PSID = null_mut();
    let mut capability_count = 0u32;
    unsafe {
        DeriveCapabilitySidsFromName(
            PCWSTR(name.as_ptr()),
            &mut groups,
            &mut group_count,
            &mut capabilities,
            &mut capability_count,
        )
    }
    .map_err(win32_io_error)?;
    let sid = if capability_count == 0 || capabilities.is_null() {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "capability SID not found",
        ))
    } else {
        let raw = unsafe { *capabilities };
        let length = unsafe { GetLengthSid(raw) } as usize;
        Ok(unsafe { std::slice::from_raw_parts(raw.0 as *const u8, length) }.to_vec())
    };
    if !groups.is_null() {
        unsafe { LocalFree(Some(HLOCAL(groups as *mut _))) };
    }
    if !capabilities.is_null() {
        unsafe { LocalFree(Some(HLOCAL(capabilities as *mut _))) };
    }
    sid
}

fn create_profile_sid(name: &[u16]) -> windows::core::Result<PSID> {
    let display_name = wide("Coding Tools MCP sandbox");
    let description = wide("Per-command Coding Tools MCP AppContainer");
    unsafe {
        windows::Win32::Security::Isolation::CreateAppContainerProfile(
            PCWSTR(name.as_ptr()),
            PCWSTR(display_name.as_ptr()),
            PCWSTR(description.as_ptr()),
            None,
        )
    }
}

fn file_from_handle(handle: HANDLE) -> io::Result<File> {
    if handle.is_invalid() {
        return Err(last_io_error("invalid pipe handle"));
    }
    let file = unsafe { std::fs::File::from_raw_handle(handle.0 as *mut _) };
    Ok(File::from_std(file))
}

fn close_handle(handle: HANDLE) {
    if !handle.is_invalid() {
        unsafe { CloseHandle(handle) }.ok();
    }
}

fn wait_for_exit(handle: HANDLE) -> io::Result<u32> {
    let wait = unsafe { WaitForSingleObject(handle, INFINITE) };
    if wait.0 != WAIT_OBJECT_0 {
        return Err(last_io_error("WaitForSingleObject"));
    }
    exit_code(handle)
}

fn exit_code(handle: HANDLE) -> io::Result<u32> {
    let mut code = 0u32;
    unsafe { GetExitCodeProcess(handle, &mut code) }.map_err(win32_io_error)?;
    Ok(code)
}

fn environment_block(overrides: &[(String, String)]) -> Vec<u16> {
    let mut values = std::env::vars_os().collect::<Vec<(OsString, OsString)>>();
    let key_matches =
        |left: &OsString, right: &str| left.to_string_lossy().eq_ignore_ascii_case(right);
    // Environment names are case-insensitive on Windows. Remove inherited
    // entries before appending overrides so the block has one value per key.
    for (key, value) in overrides {
        values.retain(|(existing, _)| !key_matches(existing, key));
        values.push((OsString::from(key), OsString::from(value)));
    }
    values.sort_by(|left, right| {
        left.0
            .to_string_lossy()
            .to_ascii_lowercase()
            .cmp(&right.0.to_string_lossy().to_ascii_lowercase())
    });
    let mut block = Vec::new();
    for (key, value) in values {
        block.extend(OsStr::new(&key).encode_wide());
        block.push('=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

fn command_line(program: &str, args: &[String]) -> Vec<u16> {
    let mut value = windows_quote(program);
    for arg in args {
        value.push(' ');
        value.push_str(&windows_quote(arg));
    }
    wide(&value)
}

fn windows_quote(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".into();
    }
    if !value.chars().any(|ch| ch.is_whitespace() || ch == '"') {
        return value.into();
    }
    let mut quoted = String::from("\"");
    let mut slashes = 0usize;
    for ch in value.chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        if ch == '"' {
            quoted.push_str(&"\\".repeat(slashes * 2 + 1));
            quoted.push('"');
        } else {
            quoted.push_str(&"\\".repeat(slashes));
            quoted.push(ch);
        }
        slashes = 0;
    }
    quoted.push_str(&"\\".repeat(slashes * 2));
    quoted.push('"');
    quoted
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn display_path(value: &str) -> String {
    value
        .strip_prefix("\\\\?\\UNC\\")
        .map(|path| format!("\\\\{path}"))
        .or_else(|| value.strip_prefix("\\\\?\\").map(str::to_string))
        .unwrap_or_else(|| value.to_string())
}

fn path_is_within(root: &Path, candidate: &Path) -> bool {
    let root = normalize_path(root);
    let candidate = normalize_path(candidate);
    candidate == root || candidate.starts_with(&(root + "\\"))
}

fn path_is_same(left: &Path, right: &Path) -> bool {
    normalize_path(left) == normalize_path(right)
}

fn normalize_path(path: &Path) -> String {
    display_path(&path.to_string_lossy())
        .trim_end_matches(['\\', '/'])
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn win32_io_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

fn last_io_error(operation: &str) -> io::Error {
    io::Error::other(format!("{operation}: {}", std::io::Error::last_os_error()))
}
