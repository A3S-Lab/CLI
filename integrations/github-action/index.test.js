"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  assertTrustedActor,
  main,
  runBoundedProcess,
  structuredFailure,
  verifyA3sAutomationSupport
} = require("./index");

test("trusted actor check accepts only repository write authority", async (context) => {
  const originalFetch = global.fetch;
  context.after(() => {
    global.fetch = originalFetch;
  });
  let request;
  global.fetch = async (url, options) => {
    request = { url, options };
    return new Response(JSON.stringify({ permission: "write", role_name: "maintain" }), {
      status: 200,
      headers: { "content-type": "application/json" }
    });
  };
  await assertTrustedActor(
    { githubToken: "secret", allowedActors: [] },
    { GITHUB_ACTOR: "octocat", GITHUB_REPOSITORY: "A3S-Lab/CLI" }
  );
  assert.match(request.url, /A3S-Lab\/CLI\/collaborators\/octocat\/permission$/);
  assert.equal(request.options.redirect, "error");
  assert.equal(request.options.headers.Authorization, "Bearer secret");

  global.fetch = async () => new Response(JSON.stringify({ permission: "read" }), { status: 200 });
  await assert.rejects(
    assertTrustedActor(
      { githubToken: "secret", allowedActors: [] },
      { GITHUB_ACTOR: "reader", GITHUB_REPOSITORY: "A3S-Lab/CLI" }
    ),
    /does not have write/
  );
});

test("explicit actor allowlist avoids the permission request", async (context) => {
  const originalFetch = global.fetch;
  context.after(() => {
    global.fetch = originalFetch;
  });
  global.fetch = async () => {
    throw new Error("fetch should not run");
  };
  await assertTrustedActor(
    { githubToken: "secret", allowedActors: ["dependabot[bot]"] },
    { GITHUB_ACTOR: "dependabot[bot]", GITHUB_REPOSITORY: "A3S-Lab/CLI" }
  );
});

test("bounded process passes stdin without shell interpolation", async () => {
  const input = "$(echo should-not-run)\n::warning::still-data";
  const result = await runBoundedProcess(
    process.execPath,
    ["-e", "process.stdin.pipe(process.stdout)"],
    {
      cwd: process.cwd(),
      env: process.env,
      input,
      timeoutMs: 10_000,
      maxStdoutBytes: 1024,
      label: "fixture"
    }
  );
  assert.equal(result.code, 0);
  assert.equal(result.stdout.toString("utf8"), input);
});

test("bounded process terminates its process group at the deadline", async () => {
  await assert.rejects(
    runBoundedProcess(
      process.execPath,
      ["-e", "setInterval(() => {}, 1000)"],
      {
        cwd: process.cwd(),
        env: process.env,
        input: "",
        timeoutMs: 100,
        maxStdoutBytes: 1024,
        label: "deadline fixture"
      }
    ),
    /exceeded its deadline/
  );
});

test("structured failures expose only the bounded CLI error code", () => {
  const result = {
    code: 1,
    stdout: Buffer.from(JSON.stringify({
      schemaVersion: 1,
      ok: false,
      error: { code: "approval.required", message: "review needed" }
    })),
    stderr: "fallback"
  };
  assert.equal(
    structuredFailure(result),
    "A3S Code failed (approval.required); child output was not logged"
  );
});

test("compatibility check requires the closed tool-policy CLI contract", async () => {
  const runner = async () => ({
    code: 0,
    signal: null,
    stdout: Buffer.from("--tool-policy <TOOL_POLICY>\nworkspace-write"),
    stderr: ""
  });
  await verifyA3sAutomationSupport("a3s", process.cwd(), process.env, runner);
  await assert.rejects(
    verifyA3sAutomationSupport("old-a3s", process.cwd(), process.env, async () => ({
      code: 0,
      signal: null,
      stdout: Buffer.from("legacy help"),
      stderr: ""
    })),
    /predates closed automation profiles/
  );
});

test("action orchestration keeps prompt off argv and scrubs the model process", async () => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "a3s-action-index-"));
  try {
    const outputPath = path.join(fixture, "github-output.txt");
    fs.writeFileSync(outputPath, "", "utf8");
    let invocation;
    const environment = {
      "INPUT_GITHUB-TOKEN": "github-secret",
      INPUT_PROMPT: "Review without shell interpolation: $(whoami)",
      INPUT_PERMISSIONS: "workspace-write",
      INPUT_TIMEOUT_SECONDS: "60",
      GITHUB_WORKSPACE: fixture,
      GITHUB_OUTPUT: outputPath,
      GITHUB_ACTOR: "maintainer",
      GITHUB_REPOSITORY: "A3S-Lab/CLI",
      RUNNER_TEMP: fixture,
      OPENAI_API_KEY: "provider-secret",
      PATH: process.env.PATH
    };
    const result = Buffer.from(JSON.stringify({
      schemaVersion: 1,
      command: "code.exec",
      ok: true,
      data: {
        text: "review complete",
        sessionId: "session-1",
        usage: { totalTokens: 2 },
        toolPolicy: "workspace-write"
      }
    }));
    await main(environment, {
      command() {},
      info() {},
      async assertTrustedActor() {},
      async installA3s() {
        return "fixture-a3s";
      },
      async verifyA3sAutomationSupport() {},
      async runBoundedProcess(executable, args, options) {
        invocation = { executable, args, options };
        return { code: 0, signal: null, stdout: result, stderr: "" };
      }
    });

    assert.equal(invocation.executable, "fixture-a3s");
    assert.equal(invocation.options.input, environment.INPUT_PROMPT);
    assert.equal(invocation.args.includes(environment.INPUT_PROMPT), false);
    assert.deepEqual(
      invocation.args.slice(invocation.args.indexOf("code")),
      ["code", "exec", "--mode", "auto", "--tool-policy", "workspace-write"]
    );
    assert.equal(invocation.options.env.OPENAI_API_KEY, "provider-secret");
    assert.equal(invocation.options.env.GITHUB_OUTPUT, undefined);
    assert.equal(invocation.options.env["INPUT_GITHUB-TOKEN"], undefined);
    assert.equal(invocation.options.env.GITHUB_REPOSITORY, undefined);

    const outputs = fs.readFileSync(outputPath, "utf8");
    assert.match(outputs, /final-message<<[^\n]+\nreview complete\n/);
    assert.match(outputs, /final-message-truncated<<[^\n]+\nfalse\n/);
    assert.match(outputs, /session-id<<[^\n]+\nsession-1\n/);
    assert.equal(outputs.includes("github-secret"), false);
  } finally {
    fs.rmSync(fixture, { recursive: true, force: true });
  }
});
