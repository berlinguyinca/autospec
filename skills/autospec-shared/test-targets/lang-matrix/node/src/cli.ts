#!/usr/bin/env node
// cli.ts — CLI entry point
import { Command } from 'commander';
import { greet } from './lib.js';

const program = new Command();
program
  .name('hello')
  .description('CLI greeter')
  .argument('<name>', 'name to greet')
  .action((name: string) => {
    process.stdout.write(`${greet(name)}\n`);
  });

program.parse();
