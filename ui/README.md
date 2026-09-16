# UI extension bundle

React UI extension for the DynamoDB Tabularis plugin. It contributes the
AWS-specific connection fields to the host's
[`connection-modal.extra_fields`](https://github.com/TabularisDB/tabularis/blob/main/plugins/PLUGIN_GUIDE.md#available-slots)
slot, which Tabularis renders between HOST/PORT and USERNAME/PASSWORD.

## What it renders

| Field | Kind | Written to | Consumed by |
|---|---|---|---|
| **AWS region** | select (34 regions + "Default (plugin setting)") | `extra["region"]` | `src/handlers/connection.rs` region precedence (wins over the endpoint hostname and the plugin setting) |
| **AWS profile (optional)** | text | `extra["profile"]` | `normalized_params()` → `profile`, which also exempts the connection from region defaulting |
| **Session token (optional)** | password | `extra["session_token"]` | `normalized_params()` → `session_token` for temporary (STS) credentials |

Values land in the opaque per-connection `extra` map that the host persists
and forwards to the plugin verbatim. The plugin trims them, ignores blanks and
lets explicit top-level params win.

The two generic USERNAME/PASSWORD fields stay where they are and keep their
generic labels — the manifest has no per-field label override and a plugin must
not manipulate the DOM outside its own subtree — so the bundle spells the
mapping out in a hint under them: *Username = AWS Access Key ID, Password = AWS
Secret Access Key*.

## Build

```bash
npm install
npm run build     # -> dist/index.js (IIFE, ~3 kB)
npm test          # build + unit tests + bundle-contract tests
npm run typecheck
```

`dist/` is not committed: `just build` / `just release` (and the release
workflow) build it before packaging, and `just dev-install` copies
`ui/dist/index.js` into the plugin folder.

## How it works

- `src/index.tsx` uses `defineSlot("connection-modal.extra_fields", …)` and
  returns `null` for any driver other than `dynamodb`.
- React, `react/jsx-runtime` and `@tabularis/plugin-api` are Vite externals —
  the host injects them as globals (`React`, `ReactJSXRuntime`,
  `__TABULARIS_API__`) at load time, so nothing is double-bundled. The IIFE
  global must stay `__tabularis_plugin__`; the host reads it to find the
  component.
- `test/bundle.test.tsx` evaluates `dist/index.js` through the host's exact
  loading path (`new Function("React", "ReactJSXRuntime", "__TABULARIS_API__",
  source + "… __tabularis_plugin__ …")`) and renders it, so a broken bundle
  fails in CI instead of silently disappearing in the GUI.
- `test/manifest.test.ts` keeps the region list in sync with the `region`
  plugin setting in `.tabularium` and checks that the declared module path is
  actually built.