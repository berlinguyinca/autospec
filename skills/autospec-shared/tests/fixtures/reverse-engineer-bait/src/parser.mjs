// parser.mjs — argument parser (imported by cli.mjs and config.mjs and validator.mjs)

import { formatDate } from './utils.mjs';

/**
 * Parses CLI arguments into an options object.
 * @param {string[]} argv
 * @returns {{ name: string, verbose: boolean, date: string }}
 */
export function parseArgs(argv) {
  const name = argv[0] || 'world';
  const verbose = argv.includes('--verbose');
  const date = formatDate(new Date());
  return { name, verbose, date };
}
