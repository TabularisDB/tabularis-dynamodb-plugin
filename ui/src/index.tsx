// UI extension for the DynamoDB plugin.
//
// Contributes the AWS-specific connection fields to the host's
// `connection-modal.extra_fields` slot, which renders directly below
// HOST/PORT and above USERNAME/PASSWORD. Every value is written into the
// opaque per-connection `extra` map, which the host persists and forwards to
// the plugin untouched — `src/handlers/connection.rs` reads `extra["region"]`,
// `extra["profile"]` and `extra["session_token"]`.
//
// See the full slot list at:
// https://github.com/TabularisDB/tabularis/blob/main/plugins/PLUGIN_GUIDE.md#available-slots

import { defineSlot } from "@tabularis/plugin-api";
import type { ChangeEvent } from "react";

import {
  CREDENTIAL_HINT,
  EXTRA_KEYS,
  PROFILE_HINT,
  SESSION_TOKEN_HINT,
  extraValue,
  selectedRegion,
  updateProfile,
  updateRegion,
  updateSessionToken,
} from "./extraFields";
import { regionOptions } from "./regions";
import {
  PLUGIN_ID,
  containerStyle,
  fieldStyle,
  hintStyle,
  inputStyle,
  labelStyle,
  rowStyle,
} from "./styles";

const ConnectionFields = defineSlot(
  "connection-modal.extra_fields",
  ({ context }) => {
    // Only Tabularis connections using this driver get the fields.
    if (context.driver !== PLUGIN_ID) return null;

    const { extra, setExtraField } = context;

    return (
      <div style={containerStyle} data-tabularis-plugin={PLUGIN_ID}>
        <div style={fieldStyle}>
          <label style={labelStyle} htmlFor="dynamodb-region">
            AWS region
          </label>
          <select
            id="dynamodb-region"
            aria-label="AWS region"
            style={inputStyle}
            value={selectedRegion(extra)}
            onChange={(event: ChangeEvent<HTMLSelectElement>) =>
              updateRegion(setExtraField, event.target.value)
            }
          >
            {regionOptions().map((option) => (
              <option key={option.value || "default"} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </div>

        <div style={rowStyle}>
          <div style={fieldStyle}>
            <label style={labelStyle} htmlFor="dynamodb-profile">
              AWS profile (optional)
            </label>
            <input
              id="dynamodb-profile"
              aria-label="AWS profile"
              type="text"
              style={inputStyle}
              value={extraValue(extra, EXTRA_KEYS.profile)}
              placeholder="default"
              autoCorrect="off"
              autoCapitalize="off"
              autoComplete="off"
              spellCheck={false}
              onChange={(event: ChangeEvent<HTMLInputElement>) =>
                updateProfile(setExtraField, event.target.value)
              }
            />
            <p style={hintStyle}>{PROFILE_HINT}</p>
          </div>

          <div style={fieldStyle}>
            <label style={labelStyle} htmlFor="dynamodb-session-token">
              Session token (optional)
            </label>
            <input
              id="dynamodb-session-token"
              aria-label="Session token"
              type="password"
              style={inputStyle}
              value={extraValue(extra, EXTRA_KEYS.sessionToken)}
              autoCorrect="off"
              autoCapitalize="off"
              autoComplete="off"
              spellCheck={false}
              onChange={(event: ChangeEvent<HTMLInputElement>) =>
                updateSessionToken(setExtraField, event.target.value)
              }
            />
            <p style={hintStyle}>{SESSION_TOKEN_HINT}</p>
          </div>
        </div>

        <p style={hintStyle}>{CREDENTIAL_HINT}</p>
      </div>
    );
  },
);

export default ConnectionFields.component;