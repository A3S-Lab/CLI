"use strict";

const { spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { StringDecoder } = require("node:string_decoder");
const vscode = require("vscode");

const {
  buildEditorPrompt,
  isNonNilUuid,
  parseJsonlLine,
  sanitizeForOutput,
  workspaceFileIsSafe
} = require("./core");

const MAX_JSONL_BYTES = 32 * 1024 * 1024;
const MAX_JSONL_LINE_BYTES = 2 * 1024 * 1024;
const MAX_REMOTE_DIFF_BYTES = 64 * 1024 * 1024;
const MAX_STDERR_BYTES = 1024 * 1024;
const EXEC_TIMEOUT_MS = 30 * 60 * 1000;
const REMOTE_TIMEOUT_MS = 2 * 60 * 1000;

let activeChild = null;

function configuration() {
  return vscode.workspace.getConfiguration("a3sCode");
}

function activeWorkspace() {
  if (!vscode.workspace.isTrusted) {
    throw new Error("Trust this workspace before allowing A3S Code to read files or start the configured executable.");
  }
  const editor = vscode.window.activeTextEditor;
  if (editor) {
    const folder = vscode.workspace.getWorkspaceFolder(editor.document.uri);
    if (folder) {
      return folder;
    }
  }
  const folders = vscode.workspace.workspaceFolders || [];
  if (folders.length === 1) {
    return folders[0];
  }
  throw new Error("Open a workspace, or focus a file inside one, before running A3S Code.");
}

function workspaceLabel(root, fileName) {
  return path.relative(root, fileName).split(path.sep).join("/");
}

function collectEditorDocuments(folder) {
  const root = folder.uri.fsPath;
  const realRoot = fs.realpathSync.native(root);
  const active = vscode.window.activeTextEditor;
  const documents = [];
  if (active
      && active.document.uri.scheme === "file"
      && workspaceFileIsSafe(root, realRoot, active.document.uri.fsPath)) {
    if (!active.selection.isEmpty) {
      documents.push({
        kind: "selection",
        uri: workspaceLabel(root, active.document.uri.fsPath),
        languageId: active.document.languageId,
        active: true,
        dirty: active.document.isDirty,
        content: active.document.getText(active.selection),
        selection: {
          startLine: active.selection.start.line + 1,
          startColumn: active.selection.start.character + 1,
          endLine: active.selection.end.line + 1,
          endColumn: active.selection.end.character + 1
        }
      });
    }
  }

  for (const document of vscode.workspace.textDocuments) {
    if (document.isClosed
        || document.uri.scheme !== "file"
        || !workspaceFileIsSafe(root, realRoot, document.uri.fsPath)) {
      continue;
    }
    documents.push({
      kind: "file",
      uri: workspaceLabel(root, document.uri.fsPath),
      languageId: document.languageId,
      active: Boolean(active && active.document === document),
      dirty: document.isDirty,
      content: document.getText()
    });
  }
  return documents;
}

function cliInvocation(folder, output) {
  const config = configuration();
  const executable = String(config.get("executablePath", "a3s")).trim() || "a3s";
  const args = [
    "--non-interactive",
    "--output",
    output,
    "--color",
    "never",
    "--no-progress",
    "--directory",
    folder.uri.fsPath
  ];
  const configPath = String(config.get("configPath", "")).trim();
  if (configPath) {
    args.push("--config", configPath);
  }
  return { executable, args, cwd: folder.uri.fsPath };
}

function terminateChild(child, force = false) {
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

function runProcess({ executable, args, cwd, input = "", token, timeoutMs, maxStdoutBytes, onStdout }) {
  if (activeChild) {
    throw new Error("Another A3S Code command is still running. Cancel it before starting a new one.");
  }
  return new Promise((resolve, reject) => {
    let settled = false;
    let abortError = null;
    let stdoutBytes = 0;
    let stderrBytes = 0;
    const stdoutChunks = [];
    const stderrChunks = [];
    let cancellation = { dispose() {} };
    let timer;
    let forceTimer;
    const child = spawn(executable, args, {
      cwd,
      env: process.env,
      shell: false,
      detached: process.platform !== "win32",
      windowsHide: true,
      stdio: ["pipe", "pipe", "pipe"]
    });
    activeChild = child;

    const finish = (error, result) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      clearTimeout(forceTimer);
      cancellation.dispose();
      if (activeChild === child) {
        activeChild = null;
      }
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
      terminateChild(child);
      forceTimer = setTimeout(() => {
        terminateChild(child, true);
        child.stdin.destroy();
        child.stdout.destroy();
        child.stderr.destroy();
        child.unref();
        finish(abortError);
      }, 5_000);
    };
    const failBound = (streamName) => {
      abort(new Error(`A3S ${streamName} exceeded its bounded output limit.`));
    };

    child.stdout.on("data", (chunk) => {
      stdoutBytes += chunk.length;
      if (stdoutBytes > maxStdoutBytes) {
        failBound("stdout");
        return;
      }
      if (onStdout) {
        try {
          onStdout(chunk);
        } catch (error) {
          abort(error);
        }
      } else {
        stdoutChunks.push(chunk);
      }
    });
    child.stderr.on("data", (chunk) => {
      stderrBytes += chunk.length;
      if (stderrBytes > MAX_STDERR_BYTES) {
        failBound("stderr");
        return;
      }
      stderrChunks.push(chunk);
    });
    child.once("error", (error) => finish(new Error(`Failed to start A3S: ${error.message}`)));
    child.once("close", (code, signal) => {
      const stderr = Buffer.concat(stderrChunks).toString("utf8");
      if (abortError) {
        finish(abortError);
      } else {
        finish(null, {
          code,
          signal,
          stdout: Buffer.concat(stdoutChunks),
          stderr
        });
      }
    });
    child.stdin.on("error", (error) => {
      if (error.code !== "EPIPE") {
        abort(new Error(`Failed to send the A3S prompt: ${error.message}`));
      }
    });
    child.stdin.end(input, "utf8");

    cancellation = token
      ? token.onCancellationRequested(() => {
          abort(new Error("A3S Code was cancelled."));
        })
      : { dispose() {} };
    timer = setTimeout(() => {
      abort(new Error("A3S Code exceeded its execution deadline."));
    }, timeoutMs);
  });
}

async function runCodeExec(folder, permission, prompt, outputChannel, token) {
  const invocation = cliInvocation(folder, "jsonl");
  const mode = permission === "workspace-write" ? "auto" : "plan";
  invocation.args.push("code", "exec", "--mode", mode, "--tool-policy", permission);
  const model = String(configuration().get("model", "")).trim();
  if (model) {
    invocation.args.push("--model", model);
  }

  const decoder = new StringDecoder("utf8");
  let pending = "";
  let result = null;
  let structuredError = null;
  let streamedText = false;
  let nextSequence = 1;
  let terminalRecord = false;
  const consumeLine = (line) => {
    if (Buffer.byteLength(line, "utf8") > MAX_JSONL_LINE_BYTES) {
      throw new Error("A3S emitted an oversized JSONL record.");
    }
    const presentation = parseJsonlLine(line);
    if (!presentation) {
      return;
    }
    if (presentation.sequence !== nextSequence) {
      throw new Error(`A3S JSONL sequence mismatch: expected ${nextSequence}, received ${presentation.sequence}.`);
    }
    nextSequence += 1;
    if (terminalRecord) {
      throw new Error("A3S emitted data after its terminal JSONL record.");
    }
    if (presentation.kind === "text") {
      streamedText = streamedText || presentation.text.length > 0;
      outputChannel.append(sanitizeForOutput(presentation.text));
    } else if (presentation.kind === "tool") {
      outputChannel.appendLine(`\n[tool] ${sanitizeForOutput(presentation.name, 256)}`);
    } else if (presentation.kind === "event-error") {
      outputChannel.appendLine(`\n[event error] ${sanitizeForOutput(presentation.message)}`);
    } else if (presentation.kind === "result") {
      result = presentation.data;
      terminalRecord = true;
    } else if (presentation.kind === "error") {
      structuredError = presentation;
      terminalRecord = true;
    }
  };

  const processResult = await runProcess({
    ...invocation,
    input: prompt,
    token,
    timeoutMs: EXEC_TIMEOUT_MS,
    maxStdoutBytes: MAX_JSONL_BYTES,
    onStdout(chunk) {
      pending += decoder.write(chunk);
      let newline;
      while ((newline = pending.indexOf("\n")) >= 0) {
        const line = pending.slice(0, newline).replace(/\r$/, "");
        pending = pending.slice(newline + 1);
        consumeLine(line);
      }
      if (Buffer.byteLength(pending, "utf8") > MAX_JSONL_LINE_BYTES) {
        throw new Error("A3S emitted an oversized unterminated JSONL record.");
      }
    }
  });
  pending += decoder.end();
  if (pending.trim()) {
    consumeLine(pending.replace(/\r$/, ""));
  }
  if (processResult.code !== 0 || structuredError) {
    const message = structuredError
      ? `${structuredError.code}: ${structuredError.message}`
      : processResult.stderr || `A3S exited with code ${processResult.code}`;
    throw new Error(sanitizeForOutput(message));
  }
  if (!result) {
    throw new Error("A3S closed without a terminal result record.");
  }
  if (result.toolPolicy !== permission) {
    throw new Error("A3S did not retain the requested closed tool policy.");
  }
  if (typeof result.text !== "string" || typeof result.sessionId !== "string") {
    throw new Error("A3S returned an incomplete terminal result.");
  }
  if (!streamedText && result.text) {
    outputChannel.append(sanitizeForOutput(result.text));
  }
  outputChannel.appendLine("");
  return result;
}

async function runEditorTask(permission, outputChannel) {
  const title = permission === "workspace-write" ? "Describe the workspace edit" : "Ask A3S Code";
  const request = await vscode.window.showInputBox({
    title,
    prompt: permission === "workspace-write"
      ? "A3S may edit workspace files, but cannot run processes, Git, tasks, plug-ins, or network tools."
      : "A3S receives bounded editor context and cannot mutate files or invoke external tools.",
    ignoreFocusOut: true,
    validateInput(value) {
      return value.trim() ? undefined : "Enter a request.";
    }
  });
  if (request === undefined) {
    return;
  }
  const folder = activeWorkspace();
  const maxBytes = Number(configuration().get("maxContextBytes", 262144));
  const built = buildEditorPrompt({
    request,
    permission,
    maxBytes,
    documents: collectEditorDocuments(folder)
  });

  outputChannel.clear();
  outputChannel.show(true);
  outputChannel.appendLine(`A3S Code · ${permission} · ${built.includedDocuments} editor context item(s) · ${built.bytes} bytes`);
  if (built.truncated) {
    outputChannel.appendLine("Context was truncated at the configured byte boundary.");
  }
  outputChannel.appendLine("");

  const result = await vscode.window.withProgress({
    location: vscode.ProgressLocation.Notification,
    title: permission === "workspace-write" ? "A3S Code is editing the workspace" : "A3S Code is reviewing editor context",
    cancellable: true
  }, (_progress, token) => runCodeExec(folder, permission, built.prompt, outputChannel, token));

  const session = result.sessionId ? ` Session ${sanitizeForOutput(result.sessionId, 128)}.` : "";
  if (permission === "workspace-write") {
    await vscode.commands.executeCommand("workbench.view.scm");
    void vscode.window.showInformationMessage(`A3S Code finished. Review the workspace diff before committing.${session}`);
  } else {
    void vscode.window.showInformationMessage(`A3S Code finished.${session}`);
  }
}

async function promptRemoteIdentity(context) {
  const prior = context.workspaceState.get("a3sCode.remoteIdentity", {});
  const organization = await vscode.window.showInputBox({
    title: "A3S Cloud organization",
    value: prior.organization || "",
    ignoreFocusOut: true,
    validateInput(value) {
      return isNonNilUuid(value.trim()) ? undefined : "Enter a non-nil organization UUID.";
    }
  });
  if (organization === undefined) {
    return null;
  }
  const executionId = await vscode.window.showInputBox({
    title: "A3S Cloud execution",
    value: prior.executionId || "",
    ignoreFocusOut: true,
    validateInput(value) {
      return isNonNilUuid(value.trim()) ? undefined : "Enter a non-nil execution UUID.";
    }
  });
  if (executionId === undefined) {
    return null;
  }
  const identity = { organization: organization.trim(), executionId: executionId.trim() };
  await context.workspaceState.update("a3sCode.remoteIdentity", identity);
  return identity;
}

async function runRemote(folder, subcommand, identity, token) {
  const invocation = cliInvocation(folder, "human");
  invocation.args.push(
    "code",
    "remote",
    subcommand,
    identity.executionId,
    "--organization",
    identity.organization
  );
  const result = await runProcess({
    ...invocation,
    token,
    timeoutMs: REMOTE_TIMEOUT_MS,
    maxStdoutBytes: MAX_REMOTE_DIFF_BYTES
  });
  if (result.code !== 0) {
    throw new Error(sanitizeForOutput(result.stderr || `A3S exited with code ${result.code}`));
  }
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(result.stdout);
  } catch {
    throw new Error("A3S returned remote change data that was not valid UTF-8.");
  }
}

async function reviewRemoteChanges(context) {
  const folder = activeWorkspace();
  const identity = await promptRemoteIdentity(context);
  if (!identity) {
    return;
  }
  const patch = await vscode.window.withProgress({
    location: vscode.ProgressLocation.Notification,
    title: "Downloading immutable A3S remote changes",
    cancellable: true
  }, (_progress, token) => runRemote(folder, "diff", identity, token));
  const document = await vscode.workspace.openTextDocument({ language: "diff", content: patch });
  await vscode.window.showTextDocument(document, { preview: true });
}

async function applyRemoteChanges(context) {
  const folder = activeWorkspace();
  const identity = await promptRemoteIdentity(context);
  if (!identity) {
    return;
  }
  const approval = await vscode.window.showWarningMessage(
    `Apply the immutable patch from execution ${identity.executionId} to this workspace? A3S will preflight the whole patch and will not stage or commit it.`,
    { modal: true },
    "Apply changes"
  );
  if (approval !== "Apply changes") {
    return;
  }
  const message = await vscode.window.withProgress({
    location: vscode.ProgressLocation.Notification,
    title: "Preflighting and applying A3S remote changes",
    cancellable: true
  }, (_progress, token) => runRemote(folder, "apply", identity, token));
  await vscode.commands.executeCommand("workbench.view.scm");
  void vscode.window.showInformationMessage(sanitizeForOutput(message.trim() || "A3S remote changes were applied.", 1024));
}

function reportError(error) {
  const message = error instanceof Error ? error.message : String(error);
  void vscode.window.showErrorMessage(`A3S Code: ${sanitizeForOutput(message, 2048)}`);
}

function registerSafe(context, command, handler) {
  context.subscriptions.push(vscode.commands.registerCommand(command, async () => {
    try {
      await handler();
    } catch (error) {
      reportError(error);
    }
  }));
}

function activate(context) {
  const outputChannel = vscode.window.createOutputChannel("A3S Code");
  context.subscriptions.push(outputChannel);
  registerSafe(context, "a3sCode.askWithContext", () => runEditorTask("read-only", outputChannel));
  registerSafe(context, "a3sCode.editWithContext", () => runEditorTask("workspace-write", outputChannel));
  registerSafe(context, "a3sCode.reviewRemoteChanges", () => reviewRemoteChanges(context));
  registerSafe(context, "a3sCode.applyRemoteChanges", () => applyRemoteChanges(context));
}

function deactivate() {
  terminateChild(activeChild);
}

module.exports = { activate, deactivate };
