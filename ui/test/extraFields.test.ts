import { describe, expect, it, vi } from "vitest";

import {
  CREDENTIAL_HINT,
  EXTRA_KEYS,
  extraValue,
  selectedRegion,
  updateProfile,
  updateRegion,
  updateSessionToken,
} from "../src/extraFields";

describe("extra field values", () => {
  it("reads a stored value", () => {
    expect(extraValue({ region: "us-west-2" }, EXTRA_KEYS.region)).toBe("us-west-2");
  });

  it("returns empty string for missing, blank or non-string values", () => {
    expect(extraValue(undefined, EXTRA_KEYS.region)).toBe("");
    expect(extraValue(null, EXTRA_KEYS.region)).toBe("");
    expect(extraValue({}, EXTRA_KEYS.region)).toBe("");
    expect(extraValue({ region: 42 as unknown as string }, EXTRA_KEYS.region)).toBe("");
  });

  it("preserves surrounding whitespace so editing does not eat spaces", () => {
    expect(extraValue({ profile: "my profile" }, EXTRA_KEYS.profile)).toBe("my profile");
    expect(extraValue({ profile: "  my profile " }, EXTRA_KEYS.profile)).toBe("  my profile ");
  });

  it("trims only when resolving the region", () => {
    expect(selectedRegion({ region: " us-west-2 " })).toBe("us-west-2");
    expect(selectedRegion({ region: "   " })).toBe("");
    expect(selectedRegion({})).toBe("");
  });
});

describe("extra field writers", () => {
  it("writes the region under the extra key the plugin reads", () => {
    const setExtraField = vi.fn();
    updateRegion(setExtraField, "ap-southeast-2");
    expect(setExtraField).toHaveBeenCalledWith("region", "ap-southeast-2");
  });

  it("clears the region when the plugin default is selected", () => {
    const setExtraField = vi.fn();
    updateRegion(setExtraField, "");
    expect(setExtraField).toHaveBeenCalledWith("region", "");
  });

  it("writes the profile and session token under their plugin keys", () => {
    const setExtraField = vi.fn();
    updateProfile(setExtraField, "staging");
    updateSessionToken(setExtraField, "FQoGZXIvYXdzEXAMPLE");

    expect(setExtraField).toHaveBeenNthCalledWith(1, "profile", "staging");
    expect(setExtraField).toHaveBeenNthCalledWith(2, "session_token", "FQoGZXIvYXdzEXAMPLE");
  });

  it("stores values verbatim (the plugin trims and drops blanks)", () => {
    const setExtraField = vi.fn();
    updateProfile(setExtraField, "my profile ");
    expect(setExtraField).toHaveBeenCalledWith("profile", "my profile ");
  });
});

describe("Credential hint", () => {
  it("names the generic fields the host renders next to this slot", () => {
    // The manifest cannot relabel the built-in USERNAME/PASSWORD fields, so the
    // mapping has to be spelled out in the component.
    expect(CREDENTIAL_HINT).toContain("Username = AWS Access Key ID");
    expect(CREDENTIAL_HINT).toContain("Password = AWS Secret Access Key");
  });
});