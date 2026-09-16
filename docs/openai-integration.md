# OpenAI integration

Status checked: 2026-09-16

Threadmoth is distributed to OpenAI coding-agent surfaces as a plugin containing
the canonical Agent Skill and the existing local MCP adapter. The model chooses
the intended result; Threadmoth observes, guards, previews, mutates and returns
the certificate.

## Package layout

The OpenAI-native package is deliberately small:

- [`.codex-plugin/plugin.json`](../.codex-plugin/plugin.json) is the documented
  Codex plugin manifest.
- [`.mcp.json`](../.mcp.json) starts the existing local server as
  `threadmoth mcp`.
- [`skills/threadmoth/SKILL.md`](../skills/threadmoth/SKILL.md) remains the one
  semantic authority for when and how agents use Threadmoth.
- [`.agents/plugins/marketplace.json`](../.agents/plugins/marketplace.json)
  makes the GitHub repository importable as a Codex marketplace with an
  `AVAILABLE` / `ON_INSTALL` entry. It points at the canonical repository's
  `main` branch and does not install a binary.

The existing Claude, Gemini and Antigravity manifests remain separate adapters.
They are not copied into the OpenAI manifest and share the same skill and Core.

## Local setup

Install the current `threadmoth` executable separately and make sure it is on
`PATH`. Then import or enable the plugin from the Codex plugin surface, or have
an eligible workspace administrator import the repository marketplace from:

```text
https://github.com/matthewjameswatkins1978-cyber/Threadmoth
```

The runtime health checks are:

```text
threadmoth --version
threadmoth doctor --json
threadmoth capabilities
```

The plugin does not download or install Threadmoth, modify `PATH`, grant extra
filesystem access, start a permanent service, upload source files, or provide a
generic shell. If the executable is missing, the MCP connection cannot provide
mutation; install the binary and retry the health checks.

## MCP boundary

The packaged local server is:

```json
{
  "mcpServers": {
    "threadmoth": {
      "command": "threadmoth",
      "args": ["mcp"]
    }
  }
}
```

It is the existing implementation in `src/cli_mcp.rs`, not an OpenAI-specific
mutation implementation. The tool descriptions were audited against model
selection needs:

| Phase | Tools | Writes? | Result |
|---|---|---:|---|
| Discover | `threadmoth_capabilities`, `threadmoth_inspect`, `threadmoth_suggest`, `threadmoth_explain` | No | Capabilities, identity facts, request suggestions, or reason metadata |
| Prepare | `threadmoth_preview`, `threadmoth_plan`, `threadmoth_transact_preview` | No | Prospective certificate, guarded plan, or refusal |
| Commit | `threadmoth_mutate`, `threadmoth_apply_plan`, `threadmoth_transact`, `threadmoth_exact_replace`, `threadmoth_set_value` | Yes, guarded | Committed certificate or refusal |

Agents must preserve the same request and certificate boundary across these
phases. A refusal is not permission to use a raw editor, script, patch, regex,
or direct write. If Threadmoth does not support the requested shape, the agent
must state that limitation or use an explicitly authorised specialist route.

## What was verified

Repository-level verification confirms the native manifest, skill path, MCP
declaration, and marketplace JSON parse successfully. The local executable
reports Threadmoth 1.9.1, and the existing CLI/MCP regression suite remains the
runtime authority. This repository change does not claim a separate field test
of an OpenAI-hosted model selecting Threadmoth automatically.

The current evidence boundary is:

| Surface | Status | Evidence / limitation |
|---|---|---|
| Codex local | PACKAGED BUT UNVERIFIED | Native manifest and local MCP declaration are present; a separate Codex plugin import and neutral-prompt field run were not available in this checkout |
| ChatGPT Desktop | PACKAGED BUT UNVERIFIED | Plugin packaging is present, but no separate ChatGPT Desktop field run was performed |
| ChatGPT Web | UNAVAILABLE FOR LOCAL MCP | OpenAI documents that ChatGPT connects to remote MCP servers; local stdio requires a supported desktop/local surface or Secure MCP Tunnel |
| OpenAI managed workspace | READY FOR ADMIN IMPORT | An admin can import the GitHub marketplace, then set installation and app/action policy; workspace and plan controls still apply |
| Public Plugin Directory | RESEARCH COMPLETE; SUBMISSION NOT MADE | OpenAI accepts app submissions, but publication is a separate review/submission boundary and was not performed |

## Official references

- [Plugins in ChatGPT and Codex](https://help.openai.com/en/articles/20001256/)
- [Importing and syncing plugin marketplaces from GitHub](https://help.openai.com/en/articles/20001504)
- [Package your plugin](https://developers.openai.com/codex/plugins/build)
- [Build with the Apps SDK](https://help.openai.com/en/articles/12515353-build-with-the-apps-sdk)
- [Developer mode and MCP apps in ChatGPT](https://help.openai.com/en/articles/12584461)

The official guidance checked on 2026-09-16 describes plugins as packages that
can contain skills, apps and app templates, and the Codex plugin reference
documents `.codex-plugin/plugin.json`, `.mcp.json` and
`.agents/plugins/marketplace.json`. The guidance also says that local MCP is not
directly reachable from ChatGPT Web; a remote MCP endpoint or Secure MCP Tunnel
is required for that surface. Those constraints are documented here rather than
being hidden behind a hosted Threadmoth service.

## Submission boundary

Threadmoth is prepared for an eligible workspace administrator or OpenAI review
flow to inspect. No public submission has been made. A public ChatGPT app would
need the current submission flow, privacy and safety review, support/contact
details and a reachable service appropriate to the requested actions. Building
that service would change Threadmoth's local-first trust model, so it is not part
of this packet.
