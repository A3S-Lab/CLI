"use strict";

const { spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const {
  boundFinalMessage,
  escapeWorkflowCommand,
  githubOutputBlock,
  loadPrompt,
  parseA3sResult,
  parseInputs,
  permissionExecution,
  permissionIsTrusted,
  resolveWorkspaceInputs,
  scrubChildEnvironment
} = require("./core");

const MAX_GITHUB_RESPONSE_BYTES = 64 * 1024;
const MAX_A3S_STDOUT_BYTES = 32 * 1024 * 1024;
const MAX_CHILD_STDERR_BYTES = 1024 * 1024;
const INSTALL_TIMEOUT_MS = 10 * 60 * 1000;

function command(name, value) {
  process.stdout.write(`::${name}::${escapeWorkflowCommand(value)}\n`);
}

function info(message) {
  process.stdout.write(`${escapeWorkflowCommand(message)}\n`);
}

function fail(error) {
  const message = error instanceof Error ? error.message : String(error);
  command("error", message.slice(0, 4096));
  process.exitCode = 1;
}

function setOutput(name, value, environment = process.env) {
  const outputPath = String(environment.GITHUB_OUTPUT || "").trim();
  if (!outputPath) {
    throw new Error("GITHUB_OUTPUT is required");
  }
  fs.appendFileSync(outputPath, githubOutputBlock(name, value), { encoding: "utf8" });
}

async function readResponseBounded(response, maxBytes) {
  if (!response.body) {
    return "";
  }
  const reader = response.body.getReader();
  const chunks = [];
  let bytes = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      bytes += value.byteLength;
      if (bytes > maxBytes) {
        throw new Error("GitHub returned an oversized permission response");
      }
      chunks.push(Buffer.from(value));
    }
  } finally {
    reader.releaseLock();
  }
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(Buffer.concat(chunks));
  } catch {
    throw new Error("GitHub returned a non-UTF-8 permission response");
  }
}

async function assertTrustedActor(inputs, environment = process.env) {
  const actor = String(environment.GITHUB_ACTOR || "").trim();
  const repository = String(environment.GITHUB_REPOSITORY || "").trim();
  if (!actor || !repository || repository.split("/").length !== 2) {
    throw new Error("GITHUB_ACTOR and GITHUB_REPOSITORY are required for the trusted-actor check");
  }
  if (inputs.allowedActors.includes(actor.toLowerCase())) {
    return;
  }
  const [owner, repo] = repository.split("/");
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 10_000);
  let response;
  try {
    response = await fetch(
      `https://api.github.com/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/collaborators/${encodeURIComponent(actor)}/permission`,
      {
        method: "GET",
        redirect: "error",
        signal: controller.signal,
        headers: {
          Accept: "application/vnd.github+json",
          Authorization: `Bearer ${inputs.githubToken}`,
          "User-Agent": "a3s-code-action",
          "X-GitHub-Api-Version": "2026-03-10"
        }
      }
    );
  } catch (error) {
    if (error && error.name === "AbortError") {
      throw new Error("GitHub actor permission lookup timed out");
    }
    throw new Error(`GitHub actor permission lookup failed: ${error.message}`);
  } finally {
    clearTimeout(timer);
  }
  const body = await readResponseBounded(response, MAX_GITHUB_RESPONSE_BYTES);
  if (!response.ok) {
    throw new Error(`GitHub actor permission lookup returned HTTP ${response.status}`);
  }
  let payload;
  try {
    payload = JSON.parse(body);
  } catch {
    throw new Error("GitHub actor permission lookup returned invalid JSON");
  }
  if (!permissionIsTrusted(payload.permission)) {
    throw new Error(`workflow actor ${actor} does not have write, maintain, or admin repository permission`);
  }
}

function terminate(child, force = false) {
  if (child && child.exitCode === null && child.signalCode === null) {
    const signal = force ? "SIGKILL" : "SIGTERM";
    if (process.platform !== "win32" && child.pid) {
      try {
        process.kill(-child.pid, signal);
        return;
      } catch {
        // Fall through to the direct child when the process group is gone.
      }
    }
    child.kill(signal);
  }
}

function runBoundedProcess(executable, args, options) {
  return new Promise((resolve, reject) => {
    let settled = false;
    let abortError = null;
    let stdoutBytes = 0;
    let stderrBytes = 0;
    const stdout = [];
    const stderr = [];
    let timer;
    let forceTimer;
    const child = spawn(executable, args, {
      cwd: options.cwd,
      env: options.env,
      shell: false,
      detached: process.platform !== "win32",
      windowsHide: true,
      stdio: ["pipe", "pipe", "pipe"]
    });

    const finish = (error, result) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      clearTimeout(forceTimer);
      if (error) {
        reject(error);
      } else {
        resolve(result);
      }
    };
    const abort = (error) => {
      if (abortError || settled) {
        return;
      }
      abortError = error;
      terminate(child);
      forceTimer = setTimeout(() => {
        terminate(child, true);
        child.stdin.destroy();
        child.stdout.destroy();
        child.stderr.destroy();
        child.unref();
        finish(abortError);
      }, 5_000);
    };
    const boundFailure = (stream) => {
      abort(new Error(`${options.label} ${stream} exceeded its bounded output limit`));
    };

    child.stdout.on("data", (chunk) => {
      stdoutBytes += chunk.length;
      if (stdoutBytes > options.maxStdoutBytes) {
        boundFailure("stdout");
      } else {
        stdout.push(chunk);
      }
    });
    child.stderr.on("data", (chunk) => {
      stderrBytes += chunk.length;
      if (stderrBytes > MAX_CHILD_STDERR_BYTES) {
        boundFailure("stderr");
      } else {
        stderr.push(chunk);
      }
    });
    child.once("error", (error) => finish(new Error(`${options.label} failed to start: ${error.message}`)));
    child.once("close", (code, signal) => {
      if (abortError) {
        finish(abortError);
        return;
      }
      finish(null, {
        code,
        signal,
        stdout: Buffer.concat(stdout),
        stderr: Buffer.concat(stderr).toString("utf8")
      });
    });
    child.stdin.on("error", (error) => {
      if (error.code !== "EPIPE") {
        abort(new Error(`${options.label} stdin failed: ${error.message}`));
      }
    });
    child.stdin.end(options.input || "", "utf8");
    timer = setTimeout(() => {
      abort(new Error(`${options.label} exceeded its deadline`));
    }, options.timeoutMs);
  });
}

function runnerTemp(environment) {
  const requested = String(environment.RUNNER_TEMP || "").trim();
  if (!requested) {
    throw new Error("RUNNER_TEMP is required");
  }
  let resolved;
  try {
    resolved = fs.realpathSync(requested);
  } catch {
    throw new Error("RUNNER_TEMP does not exist");
  }
  if (!fs.statSync(resolved).isDirectory()) {
    throw new Error("RUNNER_TEMP is not a directory");
  }
  return resolved;
}

async function installA3s(inputs, resolved, environment) {
  if (inputs.a3sPath) {
    const requested = path.isAbsolute(inputs.a3sPath)
      ? inputs.a3sPath
      : path.resolve(resolved.workspace, inputs.a3sPath);
    let executable;
    try {
      executable = fs.realpathSync(requested);
    } catch {
      throw new Error(`a3s-path does not exist: ${inputs.a3sPath}`);
    }
    if (!fs.statSync(executable).isFile()) {
      throw new Error(`a3s-path is not a regular file: ${inputs.a3sPath}`);
    }
    if (process.platform !== "win32") {
      fs.accessSync(executable, fs.constants.X_OK);
    }
    return executable;
  }

  const temp = runnerTemp(environment);
  const installDirectory = fs.mkdtempSync(path.join(temp, "a3s-code-action-install-"));
  const repositoryRoot = path.resolve(__dirname, "..", "..");
  const baseEnvironment = scrubChildEnvironment(environment);
  const installerEnvironment = {
    ...baseEnvironment,
    A3S_INSTALL_DIR: installDirectory,
    A3S_VERSION: inputs.a3sVersion,
    A3S_MODIFY_PATH: "0",
    A3S_GITHUB_TOKEN: inputs.githubToken
  };
  let executable;
  let args;
  if (process.platform === "win32") {
    executable = "powershell.exe";
    args = [
      "-NoLogo",
      "-NoProfile",
      "-NonInteractive",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      path.join(repositoryRoot, "install.ps1"),
      "-Version",
      inputs.a3sVersion,
      "-InstallDir",
      installDirectory
    ];
  } else {
    executable = "/bin/sh";
    args = [path.join(repositoryRoot, "install.sh")];
  }
  const installation = await runBoundedProcess(executable, args, {
    cwd: repositoryRoot,
    env: installerEnvironment,
    input: "",
    timeoutMs: INSTALL_TIMEOUT_MS,
    maxStdoutBytes: 4 * 1024 * 1024,
    label: "A3S installer"
  });
  if (installation.code !== 0) {
    throw new Error(`A3S installer exited with code ${installation.code}; installer output was not logged`);
  }
  const installed = path.join(installDirectory, process.platform === "win32" ? "a3s.exe" : "a3s");
  if (!fs.existsSync(installed) || !fs.statSync(installed).isFile()) {
    throw new Error("A3S installer completed without producing the expected executable");
  }
  return installed;
}

function structuredFailure(result) {
  try {
    const parsed = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(result.stdout));
    if (parsed && parsed.error) {
      const candidate = String(parsed.error.code || "code.exec.failed");
      const code = /^[a-z0-9._-]{1,128}$/i.test(candidate) ? candidate : "code.exec.failed";
      return `A3S Code failed (${code}); child output was not logged`;
    }
  } catch {
    // Fall back to bounded stderr without exposing stdout/model content.
  }
  return `A3S Code exited with code ${result.code}; child output was not logged`;
}

async function verifyA3sAutomationSupport(executable, cwd, environment, runner = runBoundedProcess) {
  const result = await runner(executable, ["--color", "never", "code", "exec", "--help"], {
    cwd,
    env: scrubChildEnvironment(environment),
    input: "",
    timeoutMs: 15_000,
    maxStdoutBytes: 256 * 1024,
    label: "A3S compatibility check"
  });
  let help = "";
  try {
    help = new TextDecoder("utf-8", { fatal: true }).decode(result.stdout);
  } catch {
    throw new Error("A3S compatibility check returned non-UTF-8 output");
  }
  if (result.code !== 0 || !help.includes("--tool-policy") || !help.includes("workspace-write")) {
    throw new Error(
      "the selected A3S executable predates closed automation profiles; use a3s-path with a compatible build or select a newer release"
    );
  }
}

function writePrivateResult(resultBytes, environment) {
  const directory = fs.mkdtempSync(path.join(runnerTemp(environment), "a3s-code-action-result-"));
  const resultPath = path.join(directory, "result.json");
  fs.writeFileSync(resultPath, resultBytes, { flag: "wx", mode: 0o600 });
  return resultPath;
}

async function main(environment = process.env, dependencies = {}) {
  const trustedActorCheck = dependencies.assertTrustedActor || assertTrustedActor;
  const executableInstaller = dependencies.installA3s || installA3s;
  const processRunner = dependencies.runBoundedProcess || runBoundedProcess;
  const supportVerifier = dependencies.verifyA3sAutomationSupport
    || ((executable, cwd, childEnvironment) => {
      return verifyA3sAutomationSupport(executable, cwd, childEnvironment, processRunner);
    });
  const emitCommand = dependencies.command || command;
  const logInfo = dependencies.info || info;
  const inputs = parseInputs(environment);
  emitCommand("add-mask", inputs.githubToken);
  await trustedActorCheck(inputs, environment);
  const resolved = resolveWorkspaceInputs(inputs, environment);
  const prompt = loadPrompt(inputs, resolved);
  const execution = permissionExecution(inputs.permissions);
  const executable = await executableInstaller(inputs, resolved, environment);
  const childEnvironment = {
    ...scrubChildEnvironment(environment),
    NO_COLOR: "1"
  };
  await supportVerifier(executable, resolved.workingDirectory, childEnvironment);

  const args = [
    "--output",
    "json",
    "--non-interactive",
    "--color",
    "never",
    "--no-progress",
    "--directory",
    resolved.workingDirectory
  ];
  if (resolved.config) {
    args.push("--config", resolved.config);
  }
  args.push(
    "code",
    "exec",
    "--mode",
    execution.mode,
    "--tool-policy",
    execution.toolPolicy
  );
  if (inputs.model) {
    args.push("--model", inputs.model);
  }

  const run = await processRunner(executable, args, {
    cwd: resolved.workingDirectory,
    env: childEnvironment,
    input: prompt,
    timeoutMs: inputs.timeoutSeconds * 1000,
    maxStdoutBytes: MAX_A3S_STDOUT_BYTES,
    label: "A3S Code"
  });
  if (run.code !== 0) {
    if (run.stdout.length > 0) {
      const failurePath = writePrivateResult(run.stdout, environment);
      setOutput("result-file", failurePath, environment);
    }
    throw new Error(structuredFailure(run).slice(0, 4096));
  }

  const result = parseA3sResult(run.stdout, execution.toolPolicy);
  const resultPath = writePrivateResult(run.stdout, environment);
  const finalMessage = boundFinalMessage(result.data.text);
  const usageJson = JSON.stringify(result.data.usage ?? null);
  if (Buffer.byteLength(usageJson, "utf8") > 32 * 1024) {
    throw new Error("A3S usage metadata exceeded the bounded GitHub output limit");
  }
  setOutput("final-message", finalMessage.value, environment);
  setOutput("final-message-truncated", String(finalMessage.truncated), environment);
  setOutput("session-id", result.data.sessionId, environment);
  setOutput("usage-json", usageJson, environment);
  setOutput("result-file", resultPath, environment);
  logInfo(`A3S Code completed with the ${execution.toolPolicy} profile; model output was retained in action outputs.`);
}

if (require.main === module) {
  main().catch(fail);
}

module.exports = {
  assertTrustedActor,
  main,
  runBoundedProcess,
  structuredFailure,
  verifyA3sAutomationSupport
};
