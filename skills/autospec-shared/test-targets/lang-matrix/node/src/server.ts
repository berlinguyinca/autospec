// server.ts — Express HTTP server
import express from 'express';
import { greet } from './lib.js';

const app = express();

app.get('/greet/:name', (req, res) => {
  res.json({ message: greet(req.params.name) });
});

export function startServer(port = 3000) {
  return app.listen(port);
}
