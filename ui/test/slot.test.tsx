import { describe, expect, it } from "vitest";
import type { TypedSlotProps } from "@tabularis/plugin-api";
import { renderToStaticMarkup } from "react-dom/server";

import ConnectionFields from "../src/index";

type Props = TypedSlotProps<"connection-modal.extra_fields">;
type Context = Props["context"];

const context = (overrides: Partial<Context> = {}): Context => ({
  driver: "dynamodb",
  extra: {},
  setExtraField: () => {},
  ...overrides,
});

const html = (overrides: Partial<Context> = {}) =>
  renderToStaticMarkup(<ConnectionFields context={context(overrides)} pluginId="dynamodb" />);

describe("ConnectionFields slot component", () => {
  it("renders the AWS region select, profile and session token fields", () => {
    const markup = html();
    expect(markup).toContain('aria-label="AWS region"');
    expect(markup).toContain('aria-label="AWS profile"');
    expect(markup).toContain('aria-label="Session token"');
  });

  it("offers every region plus a plugin-default option", () => {
    const markup = html();
    expect(markup).toContain('<option value="us-east-1">us-east-1</option>');
    expect(markup).toContain('<option value="ap-southeast-2">ap-southeast-2</option>');
    expect(markup).toContain('<option value="" selected="">Default (plugin setting)</option>');
  });

  it("reflects the region stored in the connection's extra map", () => {
    const markup = html({ extra: { region: "eu-west-1" } });
    expect(markup).toContain('value="eu-west-1" selected=""');
  });

  it("reflects a stored profile and session token", () => {
    const markup = html({ extra: { profile: "staging", session_token: "tok-123" } });
    expect(markup).toContain('value="staging"');
    expect(markup).toContain('value="tok-123"');
  });

  it("tells the user what to put in the generic username/password fields", () => {
    expect(html()).toContain("Username = AWS Access Key ID");
  });

  it("renders nothing for other drivers", () => {
    expect(html({ driver: "postgres" })).toBe("");
  });
});