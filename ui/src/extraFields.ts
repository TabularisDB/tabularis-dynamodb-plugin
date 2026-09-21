// Pure helpers for the connection-modal.extra_fields slot.
//
// Keeping the "what gets written to the `extra` map" decisions in plain
// functions means they can be unit-tested without a DOM: the tests assert the
// exact key/value pairs the host receives through `setExtraField`.
//
// Values are stored verbatim (no trimming) so that editing an input does not
// eat the spaces the user is typing; trimming, blank detection and precedence
// against the top-level params live in the plugin's Rust side
// (`src/handlers/connection.rs`).

/** Keys the plugin reads out of the opaque `extra` connection map. */
export const EXTRA_KEYS = {
  region: "region",
  profile: "profile",
  sessionToken: "session_token",
} as const;

export type ExtraMap = Record<string, string> | undefined | null;

/** Signature of the host-provided setter (empty value clears the field). */
export type ExtraFieldSetter = (key: string, value: string) => void;

/** Raw value of an extra field; "" when unset or not a string. */
export function extraValue(extra: ExtraMap, key: string): string {
  const value = extra?.[key];
  return typeof value === "string" ? value : "";
}

/** The region the plugin should sign with, or "" to use the plugin default. */
export function selectedRegion(extra: ExtraMap): string {
  return extraValue(extra, EXTRA_KEYS.region).trim();
}

export function updateRegion(setExtraField: ExtraFieldSetter, region: string): void {
  setExtraField(EXTRA_KEYS.region, region);
}

export function updateProfile(setExtraField: ExtraFieldSetter, profile: string): void {
  setExtraField(EXTRA_KEYS.profile, profile);
}

export function updateSessionToken(setExtraField: ExtraFieldSetter, token: string): void {
  setExtraField(EXTRA_KEYS.sessionToken, token);
}

/**
 * Names of the two generic fields the host renders next to this slot. The
 * manifest cannot relabel built-in form fields and a plugin must not touch the
 * DOM outside its own subtree, so the mapping is spelled out as a hint
 * instead.
 */
export const CREDENTIAL_HINT =
  "Username = AWS Access Key ID, Password = AWS Secret Access Key. Leave both empty when using a profile.";

export const PROFILE_HINT = "Named profile from ~/.aws/credentials.";

export const SESSION_TOKEN_HINT = "Only needed for temporary (STS) credentials.";