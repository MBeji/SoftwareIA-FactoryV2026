import data from "./presets.json";

export interface Preset {
  id: string;
  level: 1 | 2 | 3;
  label: string;
  hint: string;
  input: string;
}

/**
 * The three reference inputs of the ingestion engine, one per granularity level.
 * Kept as JSON so that the demo recorder (`scripts/record-demo.mjs`) uses the same text.
 */
export const PRESETS = data as Preset[];
