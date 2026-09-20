import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const projectRoot = dirname(fileURLToPath(import.meta.url));
const tauriCli = resolve(projectRoot, "node_modules", "@tauri-apps", "cli", "tauri.js");
const userArgs = process.argv.slice(2);
const tauriArgs = ["dev", "--no-watch", ...userArgs];

// A single Rust compiler keeps Windows debug builds below the resource
// pressure that previously produced repeated rustc BEX64 failures.
const env = {
  ...process.env,
  CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS || "1",
  CARGO_INCREMENTAL: process.env.CARGO_INCREMENTAL || "0",
};

function runTauri() {
  return new Promise((resolveExit) => {
    const child = spawn(process.execPath, [tauriCli, ...tauriArgs], {
      cwd: projectRoot,
      env,
      stdio: "inherit",
      windowsHide: false,
    });

    const forwardSignal = (signal) => {
      if (!child.killed) child.kill(signal);
    };
    const onSigint = () => forwardSignal("SIGINT");
    const onSigterm = () => forwardSignal("SIGTERM");
    process.once("SIGINT", onSigint);
    process.once("SIGTERM", onSigterm);

    child.once("error", (error) => {
      console.error(`[desktop] failed to start Tauri: ${error.message}`);
      resolveExit({ code: 1, signal: null });
    });
    child.once("exit", (code, signal) => {
      process.removeListener("SIGINT", onSigint);
      process.removeListener("SIGTERM", onSigterm);
      if (signal) {
        console.error(`[desktop] Tauri stopped by ${signal}`);
      }
      resolveExit({ code: code ?? (signal ? 0 : 1), signal });
    });
  });
}

const MAX_RETRIES = 2;
let attempt = 0;
let exitCode = 1;
while (attempt <= MAX_RETRIES) {
  const result = await runTauri();
  exitCode = result.code;
  // Tauri returns zero for a normal close (including the explicit quit action).
  // Retry only failed launches/builds, which covers transient rustc/host exits.
  if (result.signal || exitCode === 0 || attempt === MAX_RETRIES) break;
  attempt += 1;
  console.error(`[desktop] Tauri exited with code ${exitCode}; retrying (${attempt}/${MAX_RETRIES})`);
}

process.exitCode = exitCode;
