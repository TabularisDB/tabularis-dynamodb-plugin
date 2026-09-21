// Guards the two things that silently break a UI extension: the `.tabularium`
// declaration not matching what was built, and the region list drifting away
// from the plugin-level setting.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { AWS_REGIONS } from "../src/regions";

const manifestPath = fileURLToPath(new URL("../../.tabularium", import.meta.url));
const uiDir = fileURLToPath(new URL("../", import.meta.url));
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));

describe(".tabularium manifest", () => {
  it("declares the connection-modal.extra_fields contribution", () => {
    const extensions = manifest.ui_extensions ?? [];
    const entry = extensions.find((e: { slot: string }) => e.slot === "connection-modal.extra_fields");

    expect(entry).toBeDefined();
    expect(entry).toMatchObject({
      slot: "connection-modal.extra_fields",
      module: "ui/dist/index.js",
      driver: "dynamodb",
    });
  });

  it("points at a module that is actually built", () => {
    const entry = (manifest.ui_extensions ?? []).find(
      (e: { slot: string }) => e.slot === "connection-modal.extra_fields",
    );
    // The host resolves `module` relative to the plugin folder; in this repo
    // that is the ui/ folder, so drop the leading "ui/".
    const relative = String(entry.module).replace(/^ui\//, "");
    expect(existsSync(join(uiDir, relative))).toBe(true);
  });

  it("keeps the region select in sync with the default-region setting", () => {
    const regionSetting = (manifest.settings ?? []).find((s: { key: string }) => s.key === "region");
    expect(regionSetting).toBeDefined();

    const settingRegions = (regionSetting.options as string[]).filter((option) => option !== "");
    expect(settingRegions).toEqual([...AWS_REGIONS]);
  });
});