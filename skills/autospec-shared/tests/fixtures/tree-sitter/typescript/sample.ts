// sample.ts — TypeScript fixture for autospec-docs walker unit tests.

import { readFileSync } from 'fs';
import path from 'path';

export interface Config {
    host: string;
    port: number;
}

export type StatusCode = 200 | 400 | 404 | 500;

export const DEFAULT_PORT = 3000;

export function parseConfig(filePath: string): Config {
    const raw = readFileSync(filePath, 'utf8');
    return JSON.parse(raw) as Config;
}

export class Server {
    private config: Config;
    constructor(config: Config) {
        this.config = config;
    }
    start(): void {
        console.log(`Listening on ${this.config.host}:${this.config.port}`);
    }
}
