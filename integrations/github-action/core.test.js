"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

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

function baseEnvironment(workspace) {
  return {
    "INPUT_GITHUB-TOKEN": "token",
    INPUT_PROMPT: "Review this change",
    INPUT_PERMISSIONS: "read-only",
    INPUT_TIMEOUT_SECONDS: "30",
    GITHUB_WORKSPACE: workspace
  };
}

test("inputs require one prompt and map closed permission profiles", () => {
  const environment = baseEnvironment("C:/workspace");
  const parsed = parseInputs(environment);
  assert.equal(parsed.githubToken, "token");
  assert.deepEqual(permissionExecution(parsed.permissions), { mode: "plan", toolPolicy: "read-only" });

  assert.throws(() => parseInputs({ ...environment, INPUT_PROMPT_FILE: "task.md" }), /exactly one/);
  assert.throws(() => parseInputs({ ...environment, INPUT_PERMISSIONS: "danger-full-access" }), /permissions/);
  assert.deepEqual(permissionExecution("workspace-write"), {
    mode: "auto",
    toolPolicy: "workspace-write"
  });
});

test("workspace inputs cannot escape through relative paths", () => {
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "a3s-action-core-"));
  try {
    fs.mkdirSync(path.join(workspace, "project"));
    fs.writeFileSync(path.join(workspace, "task.md"), "Review safely", "utf8");
    const inputs = parseInputs({
      ...baseEnvironment(workspace),
      INPUT_PROMPT: "",
      INPUT_PROMPT_FILE: "task.md",
      INPUT_WORKING_DIRECTORY: "project"
    });
    const resolved = resolveWorkspaceInputs(inputs, { GITHUB_WORKSPACE: workspace });
    assert.equal(resolved.workingDirectory, fs.realpathSync(path.join(workspace, "project")));
    assert.equal(loadPrompt(inputs, resolved), "Review safely");

    const escaping = { ...inputs, promptFile: path.resolve(workspace, "..", "outside.md") };
    assert.throws(() => resolveWorkspaceInputs(escaping, { GITHUB_WORKSPACE: workspace }), /inside GITHUB_WORKSPACE|does not exist/);
  } finally {
    fs.rmSync(workspace, { recursive: true, force: true });
  }
});

test("prompt loading enforces UTF-8 and exact byte limits", () => {
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "a3s-action-prompt-"));
  try {
    const promptFile = path.join(workspace, "task.md");
    fs.writeFileSync(promptFile, Buffer.from([0xff, 0xfe]));
    const inputs = { prompt: "", promptFile: "task.md", workingDirectory: ".", config: "" };
    const resolved = resolveWorkspaceInputs(inputs, { GITHUB_WORKSPACE: workspace });
    assert.throws(() => loadPrompt(inputs, resolved, 8), /valid UTF-8/);
    fs.writeFileSync(promptFile, "🙂🙂🙂", "utf8");
    assert.throws(() => loadPrompt(inputs, resolved, 8), /exceeds/);
  } finally {
    fs.rmSync(workspace, { recursive: true, force: true });
  }
});

test("structured result must echo the requested policy", () => {
  const result = parseA3sResult(Buffer.from(JSON.stringify({
    schemaVersion: 1,
    command: "code.exec",
    ok: true,
    data: { text: "done", sessionId: "session", usage: {}, toolPolicy: "read-only" }
  })), "read-only");
  assert.equal(result.data.text, "done");
  assert.throws(() => parseA3sResult(Buffer.from(JSON.stringify({
    schemaVersion: 1,
    ok: true,
    data: { text: "done", sessionId: "session", toolPolicy: "standard" }
  })), "read-only"), /retain/);
});

test("child environment removes GitHub capabilities but preserves provider configuration", () => {
  const scrubbed = scrubChildEnvironment({
    PATH: "/bin",
    OPENAI_API_KEY: "provider",
    GITHUB_TOKEN: "github",
    GITHUB_REPOSITORY: "A3S-Lab/CLI",
    "INPUT_GITHUB-TOKEN": "input",
    GITHUB_OUTPUT: "/tmp/output",
    ACTIONS_ID_TOKEN_REQUEST_TOKEN: "oidc"
  });
  assert.deepEqual(scrubbed, { PATH: "/bin", OPENAI_API_KEY: "provider" });
});

test("trusted permissions and multiline outputs are closed and injection-safe", () => {
  assert.equal(permissionIsTrusted("write"), true);
  assert.equal(permissionIsTrusted("maintain"), true);
  assert.equal(permissionIsTrusted("read"), false);
  const values = ["collision", "safe"];
  const block = githubOutputBlock("final-message", "first\na3s_collision\nlast", () => values.shift());
  assert.match(block, /^final-message<<a3s_safe\n/);
  assert.equal(escapeWorkflowCommand("bad%\n::warning::x"), "bad%25%0A::warning::x");
  assert.equal(escapeWorkflowCommand("\u001b[31mred\u001b[0m\u202e"), "red�");
});

test("final message output is UTF-8 bounded without splitting a character", () => {
  const result = boundFinalMessage("a🙂b", 4);
  assert.deepEqual(result, { value: "a", truncated: true });
  assert.deepEqual(boundFinalMessage("done", 4), { value: "done", truncated: false });
});
