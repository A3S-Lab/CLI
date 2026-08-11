# A3S Code for VS Code-compatible editors

This zero-runtime-dependency extension connects VS Code, Cursor, and Windsurf to
the installed `a3s` executable. It deliberately reuses `a3s code exec` and
`a3s code remote`; it does not embed a second agent runtime or retain provider
credentials.

Commands are disabled until the editor workspace is trusted. Open-document
context is also resolved through the real workspace boundary, so a file reached
through an escaping symlink is not sent to the model.

## Commands

- **Ask with Editor Context** sends the active selection first, followed by the
  active and other open workspace documents under one exact UTF-8 byte limit.
  It forces `--mode plan --tool-policy read-only`.
- **Edit with Editor Context** uses the same context and forces
  `--mode auto --tool-policy workspace-write`, then opens Source Control for
  focused diff review.
- **Review Remote Changes** downloads the immutable, digest-checked patch for an
  A3S Cloud execution and opens it as a diff document.
- **Apply Remote Changes** requires modal confirmation, performs the CLI's
  whole-patch Git preflight, applies without staging or committing, and opens
  Source Control.

The automation profiles never expose shell, Git, task, runtime, plug-in, MCP,
or network tools to the model. `workspace-write` adds only bounded native file
write/edit/patch tools. Run the full TUI when an interactive task needs broader
reviewed authority.

## Development

```bash
node --test integrations/vscode/core.test.js
cd integrations/vscode
npx @vscode/vsce package
```

Install the resulting VSIX through your editor's extension manager. Configure
`a3sCode.executablePath` if `a3s` is not on the editor process `PATH`; optional
ACL and model overrides are available under `a3sCode.*` settings.
