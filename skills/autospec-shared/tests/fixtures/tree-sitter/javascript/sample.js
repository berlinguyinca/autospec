#!/usr/bin/env node
// sample.js — JavaScript fixture for autospec-docs walker unit tests.

const express = require('express');
import { readFile } from 'fs/promises';

const app = express();

export const VERSION = '1.0.0';

export function greet(name) {
    return `Hello, ${name}!`;
}

export class EventEmitter {
    constructor() {
        this.listeners = {};
    }
    on(event, handler) {
        this.listeners[event] = handler;
    }
}

app.get('/health', (req, res) => {
    res.json({ status: 'ok' });
});

app.post('/greet', (req, res) => {
    res.json({ message: greet(req.body.name) });
});
