"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const DEFAULT_MAX_PROMPT_BYTES = 16 * 1024 * 1024;
const MAX_FINAL_MESSAGE_OUTPUT_BYTES = 448 * 1024;
const TRUSTED_PERMISSIONS = new Set(["admin", "maintain", "write"]);

function inputEnvironmentNames(name) {
  const canonical = `INPUT_${name.replace(/ /g, "_").toUpperCase()}`;
  return [canonical, canonical.replace(/-/g, "_")];
}

function getInput(name, environment = process.env, trim = true) {
  let value = "";
  for (const key of inputEnvironmentNames(name)) {
    if (Object.prototype.hasOwnProperty.call(environment, key)) {
      value = String(environment[key] || "");
      break;
    }
  }
  return trim ? value.trim() : value;
}

function parseAllowedActors(value) {
  if (!value.trim()) {
    return [];
  }
  const actors = value.split(",").map((actor) => actor.trim()).filter(Boolean);
  if (actors.length > 128) {
    throw new Error("allowed-actors may contain at most 128 logins");
  }
  for (const actor of actors) {
    if (actor.length > 100 || !/^[A-Za-z0-9_.\[\]-]+$/.test(actor)) {
      throw new Error(`allowed-actors contains an invalid login: ${actor}`);
    }
  }
  return [...new Set(actors.map((actor) => actor.toLowerCase()))];
}

function parseInputs(environment = process.env) {
  const githubToken = getInput("github-token", environment);
  if (!githubToken) {
    throw new Error("github-token is required for the trusted-actor check");
  }
  const prompt = getInput("prompt", environment, false);
  const promptFile = getInput("prompt-file", environment);
  if (Boolean(prompt.trim()) === Boolean(promptFile)) {
    throw new Error("exactly one of prompt or prompt-file is required");
  }
  const permissions = getInput("permissions", environment) || "read-only";
  if (!new Set(["read-only", "workspace-write"]).has(permissions)) {
    throw new Error("permissions must be read-only or workspace-write");
  }
  const timeoutText = getInput("timeout-seconds", environment) || "1800";
  if (!/^[0-9]+$/.test(timeoutText)) {
    throw new Error("timeout-seconds must be an integer from 1 through 7200");
  }
  const timeoutSeconds = Number(timeoutText);
  if (!Number.isSafeInteger(timeoutSeconds) || timeoutSeconds < 1 || timeoutSeconds > 7200) {
    throw new Error("timeout-seconds must be an integer from 1 through 7200");
  }
  let a3sVersion = getInput("a3s-version", environment) || "latest";
  if (a3sVersion !== "latest") {
    if (!/^v?[0-9]+\.[0-9]+\.[0-9]+$/.test(a3sVersion)) {
      throw new Error("a3s-version must be latest or a stable X.Y.Z/vX.Y.Z tag");
    }
    if (!a3sVersion.startsWith("v")) {
      a3sVersion = `v${a3sVersion}`;
    }
  }
  return {
    githubToken,
    prompt,
    promptFile,
    workingDirectory: getInput("working-directory", environment) || ".",
    config: getInput("config", environment),
    model: getInput("model", environment),
    permissions,
    a3sVersion,
    a3sPath: getInput("a3s-path", environment),
    allowedActors: parseAllowedActors(getInput("allowed-actors", environment)),
    timeoutSeconds
  };
}

function isPathInside(root, candidate) {
  const relative = path.relative(root, candidate);
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative));
}

function resolveExistingPath(base, requested, expectedType, boundary = base) {
  const unresolved = path.isAbsolute(requested) ? requested : path.resolve(base, requested);
  let resolved;
  try {
    resolved = fs.realpathSync(unresolved);
  } catch {
    throw new Error(`${expectedType} does not exist: ${requested}`);
  }
  if (!isPathInside(boundary, resolved)) {
    throw new Error(`${expectedType} must stay inside GITHUB_WORKSPACE: ${requested}`);
  }
  const stat = fs.statSync(resolved);
  if (expectedType === "working-directory" && !stat.isDirectory()) {
    throw new Error(`working-directory is not a directory: ${requested}`);
  }
  if (expectedType !== "working-directory" && !stat.isFile()) {
    throw new Error(`${expectedType} is not a regular file: ${requested}`);
  }
  return resolved;
}

function resolveWorkspaceInputs(inputs, environment = process.env) {
  const workspaceInput = String(environment.GITHUB_WORKSPACE || "").trim();
  if (!workspaceInput) {
    throw new Error("GITHUB_WORKSPACE is required");
  }
  let workspace;
  try {
    workspace = fs.realpathSync(workspaceInput);
  } catch {
    throw new Error("GITHUB_WORKSPACE does not exist");
  }
  if (!fs.statSync(workspace).isDirectory()) {
    throw new Error("GITHUB_WORKSPACE is not a directory");
  }
  const workingDirectory = resolveExistingPath(
    workspace,
    inputs.workingDirectory,
    "working-directory",
    workspace
  );
  const promptFile = inputs.promptFile
    ? resolveExistingPath(workspace, inputs.promptFile, "prompt-file", workspace)
    : "";
  const config = inputs.config
    ? resolveExistingPath(workspace, inputs.config, "config", workspace)
    : "";
  return { workspace, workingDirectory, promptFile, config };
}

function decodeUtf8(buffer, label) {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(buffer);
  } catch {
    throw new Error(`${label} must contain valid UTF-8`);
  }
}

function loadPrompt(inputs, resolved, maxBytes = DEFAULT_MAX_PROMPT_BYTES) {
  let buffer;
  if (resolved.promptFile) {
    const stat = fs.statSync(resolved.promptFile);
    if (stat.size > maxBytes) {
      throw new Error(`prompt-file exceeds the ${maxBytes}-byte limit`);
    }
    buffer = fs.readFileSync(resolved.promptFile);
  } else {
    buffer = Buffer.from(inputs.prompt, "utf8");
  }
  if (buffer.length > maxBytes) {
    throw new Error(`prompt exceeds the ${maxBytes}-byte limit`);
  }
  const prompt = decodeUtf8(buffer, resolved.promptFile ? "prompt-file" : "prompt");
  if (!prompt.trim()) {
    throw new Error("prompt must not be empty");
  }
  return prompt;
}

function permissionExecution(profile) {
  if (profile === "read-only") {
    return { mode: "plan", toolPolicy: "read-only" };
  }
  if (profile === "workspace-write") {
    return { mode: "auto", toolPolicy: "workspace-write" };
  }
  throw new Error("unknown permission profile");
}

function permissionIsTrusted(permission) {
  return TRUSTED_PERMISSIONS.has(String(permission || "").toLowerCase());
}

function parseA3sResult(stdout, expectedToolPolicy) {
  const text = decodeUtf8(Buffer.isBuffer(stdout) ? stdout : Buffer.from(stdout), "A3S result");
  let result;
  try {
    result = JSON.parse(text);
  } catch (error) {
    throw new Error(`A3S returned invalid JSON: ${error.message}`);
  }
  if (!result || result.schemaVersion !== 1 || result.ok !== true || !result.data || typeof result.data !== "object") {
    throw new Error("A3S did not return a successful schema-v1 result");
  }
  if (result.data.toolPolicy !== expectedToolPolicy) {
    throw new Error("A3S did not retain the requested closed tool policy");
  }
  if (typeof result.data.text !== "string"
      || typeof result.data.sessionId !== "string"
      || !result.data.sessionId
      || Buffer.byteLength(result.data.sessionId, "utf8") > 256) {
    throw new Error("A3S result is missing text or sessionId");
  }
  return result;
}

function scrubChildEnvironment(environment) {
  const scrubbed = {};
  const exact = new Set([
    "GH_TOKEN",
    "A3S_GITHUB_TOKEN"
  ]);
  for (const [key, value] of Object.entries(environment)) {
    const upper = key.toUpperCase();
    if (upper.startsWith("INPUT_")
        || upper.startsWith("GITHUB_")
        || (upper.startsWith("ACTIONS_") && (upper.includes("TOKEN") || upper.includes("URL")))
        || exact.has(upper)
        || upper.includes("GITHUB_TOKEN")) {
      continue;
    }
    scrubbed[key] = value;
  }
  return scrubbed;
}

function githubOutputBlock(name, value, randomId = () => crypto.randomUUID().replace(/-/g, "")) {
  if (!/^[A-Za-z_][A-Za-z0-9_-]*$/.test(name)) {
    throw new Error("invalid GitHub output name");
  }
  const text = String(value);
  let delimiter;
  do {
    delimiter = `a3s_${randomId()}`;
  } while (text.split(/\r?\n/).includes(delimiter));
  return `${name}<<${delimiter}\n${text}\n${delimiter}\n`;
}

function boundFinalMessage(value, maxBytes = MAX_FINAL_MESSAGE_OUTPUT_BYTES) {
  const encoded = Buffer.from(String(value), "utf8");
  if (encoded.length <= maxBytes) {
    return { value: encoded.toString("utf8"), truncated: false };
  }
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let end = maxBytes;
  while (end > 0) {
    try {
      return { value: decoder.decode(encoded.subarray(0, end)), truncated: true };
    } catch {
      end -= 1;
    }
  }
  return { value: "", truncated: true };
}

function escapeWorkflowCommand(value) {
  return String(value)
    .replace(/\u001b\][^\u0007]*(?:\u0007|\u001b\\)/g, "")
    .replace(/\u001b\[[0-?]*[ -/]*[@-~]/g, "")
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f\u202a-\u202e\u2066-\u2069]/gi, "�")
    .replace(/%/g, "%25")
    .replace(/\r/g, "%0D")
    .replace(/\n/g, "%0A");
}

module.exports = {
  DEFAULT_MAX_PROMPT_BYTES,
  MAX_FINAL_MESSAGE_OUTPUT_BYTES,
  boundFinalMessage,
  escapeWorkflowCommand,
  getInput,
  githubOutputBlock,
  isPathInside,
  loadPrompt,
  parseA3sResult,
  parseInputs,
  permissionExecution,
  permissionIsTrusted,
  resolveWorkspaceInputs,
  scrubChildEnvironment
};
