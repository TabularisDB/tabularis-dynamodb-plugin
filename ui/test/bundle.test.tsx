// Bundle contract test.
//
// The Tabularis host does not `import()` plugin bundles — it reads the file
// from the plugin folder and evaluates it as an IIFE whose parameters are the
// React globals, then reads the `__tabularis_plugin__` global:
//
//   src/contexts/PluginSlotProvider.tsx
//     const fn = new Function("React", "ReactJSXRuntime", "__TABULARIS_API__",
//       source + "\n return typeof __tabularis_plugin__ !== 'undefined' ? __tabularis_plugin__ : null;");
//
// This test runs the built `dist/index.js` through that exact loading path, so
// a broken bundle (wrong IIFE name, a bundled copy of React, a missing default
// export) fails here instead of silently disappearing in the GUI.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";
import * as React from "react";
import * as ReactJSXRuntime from "react/jsx-runtime";
import * as pluginApi from "@tabularis/plugin-api";
import { renderToStaticMarkup } from "react-dom/server";
import type { ComponentType } from "react";

import type { TypedSlotProps } from "@tabularis/plugin-api";

type Props = TypedSlotProps<"connection-modal.extra_fields">;

const bundlePath = fileURLToPath(new URL("../dist/index.js", import.meta.url));

function loadBundle(): ComponentType<Props> {
  const source = readFileSync(bundlePath, "utf8");
  const fn = new Function(
    "React",
    "ReactJSXRuntime",
    "__TABULARIS_API__",
    source +
      "\n return typeof __tabularis_plugin__ !== 'undefined' ? __tabularis_plugin__ : null;",
  );
  const raw = fn(React, ReactJSXRuntime, pluginApi) as unknown;
  const component =
    typeof raw === "function" ? raw : ((raw as { default?: unknown } | null)?.default ?? null);

  expect(typeof component).toBe("function");
  return component as ComponentType<Props>;
}

describe("built UI bundle", () => {
  it("exposes a component through the global the host reads", () => {
    expect(loadBundle()).toBeTypeOf("function");
  });

  it("uses the globals the host injects instead of bundling React", () => {
    const source = readFileSync(bundlePath, "utf8");
    expect(source).toContain("__TABULARIS_API__");
    // A bundled React would inline its own internals/licence header.
    expect(source).not.toMatch(/React v\d/);
    expect(source).not.toContain("react-production");
  });

  it("renders the configured connection fields when evaluated the host's way", () => {
    const Component = loadBundle();
    const markup = renderToStaticMarkup(
      <Component
        context={{
          driver: "dynamodb",
          extra: { region: "us-west-2", profile: "staging" },
          setExtraField: () => {},
        }}
        pluginId="dynamodb"
      />,
    );

    expect(markup).toContain('aria-label="AWS region"');
    expect(markup).toContain('value="us-west-2" selected=""');
    expect(markup).toContain('value="staging"');
    expect(markup).toContain('aria-label="Session token"');
    expect(markup).toContain("Username = AWS Access Key ID");
  });

  it("stays out of the way for other drivers", () => {
    const Component = loadBundle();
    const markup = renderToStaticMarkup(
      <Component
        context={{ driver: "postgres", extra: {}, setExtraField: () => {} }}
        pluginId="dynamodb"
      />,
    );
    expect(markup).toBe("");
  });
});