#!/usr/bin/env node
// cli.mjs — CLI entry point for bait repo

import { greet } from './utils.mjs';
import { parseArgs } from './parser.mjs';

/**
 * Main CLI entry point.
 */
export function main() {
  const args = parseArgs(process.argv.slice(2));
  console.log(greet(args.name));
}

main();
