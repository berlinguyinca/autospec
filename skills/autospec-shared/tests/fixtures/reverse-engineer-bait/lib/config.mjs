// config.mjs — configuration loader (imports parser, imported by validator)

import { parseArgs } from '../src/parser.mjs';

/**
 * Default configuration constants.
 */
export const DEFAULT_CONFIG = {
  timeout: 5000,
  retries: 3,
};

/**
 * Loads configuration from environment and argv.
 * @returns {{ timeout: number, retries: number, name: string }}
 */
export function loadConfig() {
  const args = parseArgs(process.argv.slice(2));
  return {
    ...DEFAULT_CONFIG,
    name: args.name,
  };
}
