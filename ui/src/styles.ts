import type { CSSProperties } from "react";

/** Driver id this bundle's contributions are gated on. */
export const PLUGIN_ID = "dynamodb";

export const containerStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: "10px",
};

export const fieldStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: "4px",
};

export const rowStyle: CSSProperties = {
  display: "grid",
  gridTemplateColumns: "1fr 1fr",
  gap: "12px",
};

export const labelStyle: CSSProperties = {
  fontSize: "10px",
  textTransform: "uppercase",
  fontWeight: 600,
  letterSpacing: "0.05em",
  color: "var(--color-text-muted, #94a3b8)",
};

export const inputStyle: CSSProperties = {
  width: "100%",
  padding: "7px 10px",
  background: "var(--color-bg-base, #131929)",
  border: "1px solid rgba(255,255,255,0.15)",
  borderRadius: "6px",
  color: "var(--color-text-primary, #e2e8f0)",
  fontSize: "13px",
  outline: "none",
  boxSizing: "border-box",
};

export const hintStyle: CSSProperties = {
  margin: 0,
  fontSize: "11px",
  lineHeight: 1.5,
  color: "var(--color-text-muted, #94a3b8)",
};