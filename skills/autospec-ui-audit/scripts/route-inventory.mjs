#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const SOURCE_EXTENSIONS = new Set([".js", ".jsx", ".ts", ".tsx"]);
const SKIP_DIRECTORIES = new Set([
  ".git", ".next", ".turbo", "build", "coverage", "dist", "node_modules", "out", "target",
]);

class RouteInventoryError extends Error {
  constructor(code, message, exitCode = 1) {
    super(message);
    this.code = code;
    this.exitCode = exitCode;
  }
}

function fail(code, message, exitCode = 1) {
  throw new RouteInventoryError(code, message, exitCode);
}

function parseArgs(argv) {
  const args = { repo: "", outputDir: "" };
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index];
    if (value === "--repo") args.repo = argv[++index] ?? "";
    else if (value === "--output-dir") args.outputDir = argv[++index] ?? "";
    else if (value === "--help") {
      process.stdout.write("Usage: route-inventory.mjs --repo PATH --output-dir PATH\n");
      return null;
    } else fail("ROUTE_INVENTORY_USAGE", `unknown argument: ${value}`, 2);
  }
  if (!args.repo || !args.outputDir) {
    fail("ROUTE_INVENTORY_USAGE", "--repo and --output-dir are required", 2);
  }
  return {
    repo: path.resolve(args.repo),
    outputDir: path.resolve(args.outputDir),
  };
}

function walk(root, outputDir) {
  const files = [];
  function visit(directory) {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const absolute = path.join(directory, entry.name);
      if (absolute === outputDir) continue;
      if (entry.isDirectory()) {
        if (!SKIP_DIRECTORIES.has(entry.name)) visit(absolute);
      } else if (entry.isFile()) files.push(absolute);
    }
  }
  visit(root);
  return files.sort();
}

function lineNumber(source, offset) {
  return source.slice(0, offset).split("\n").length;
}

function stripComments(source) {
  let result = "";
  let quote = "";
  let lineComment = false;
  let blockComment = false;
  for (let index = 0; index < source.length; index += 1) {
    const char = source[index];
    const next = source[index + 1];
    const previous = source[index - 1];
    if (lineComment) {
      if (char === "\n") {
        lineComment = false;
        result += char;
      } else result += " ";
    } else if (blockComment) {
      if (char === "*" && next === "/") {
        result += "  ";
        index += 1;
        blockComment = false;
      } else result += char === "\n" ? "\n" : " ";
    } else if (quote) {
      result += char;
      if (char === quote && previous !== "\\") quote = "";
    } else if (char === '"' || char === "'" || char === "`") {
      quote = char;
      result += char;
    } else if (char === "/" && next === "/") {
      result += "  ";
      index += 1;
      lineComment = true;
    } else if (char === "/" && next === "*") {
      result += "  ";
      index += 1;
      blockComment = true;
    } else result += char;
  }
  return result;
}

function routeTags(source) {
  const tags = [];
  const startPattern = /<\/?Route\b/g;
  for (const match of source.matchAll(startPattern)) {
    let quote = "";
    let braces = 0;
    let cursor = match.index;
    for (; cursor < source.length; cursor += 1) {
      const char = source[cursor];
      const previous = source[cursor - 1];
      if (quote) {
        if (char === quote && previous !== "\\") quote = "";
      } else if (char === '"' || char === "'") quote = char;
      else if (char === "{") braces += 1;
      else if (char === "}") braces -= 1;
      else if (char === ">" && braces === 0) break;
    }
    if (cursor === source.length) continue;
    tags.push({ raw: source.slice(match.index, cursor + 1), offset: match.index });
  }
  return tags;
}

function normalizeRoute(value) {
  const segments = value.split("/").filter(Boolean);
  return `/${segments.join("/")}` || "/";
}

function normalizeRegistryPath(value) {
  const url = new URL(value, "https://autospec.invalid");
  return normalizeRoute(url.pathname);
}

function joinRoute(parent, child) {
  if (child.startsWith("/")) return normalizeRoute(child);
  if (parent === "/") return normalizeRoute(`/${child}`);
  return normalizeRoute(`${parent}/${child}`);
}

function discoverReactRouter(files, repo) {
  const discoveries = [];
  for (const file of files.filter((candidate) => SOURCE_EXTENSIONS.has(path.extname(candidate)))) {
    const source = stripComments(fs.readFileSync(file, "utf8"));
    const parents = [];
    for (const tag of routeTags(source)) {
      if (tag.raw.startsWith("</")) {
        if (parents.length > 0) parents.pop();
        continue;
      }
      const pathMatch = tag.raw.match(/\bpath\s*=\s*(["'])(.*?)\1/);
      const parent = parents.at(-1) ?? "/";
      const routePath = pathMatch ? joinRoute(parent, pathMatch[2]) : parent;
      if (pathMatch) {
        discoveries.push({
          path: routePath,
          lazy: /\blazy\s*=/.test(tag.raw),
          source: `${path.relative(repo, file)}:${lineNumber(source, tag.offset)}`,
        });
      }
      if (!/\/\s*>$/.test(tag.raw)) parents.push(routePath);
    }
  }
  return discoveries;
}

function assertNoRouteCollectionCycles(files, repo) {
  const graph = new Map();
  for (const file of files.filter((candidate) => SOURCE_EXTENSIONS.has(path.extname(candidate)))) {
    const source = stripComments(fs.readFileSync(file, "utf8"));
    const declarations = [...source.matchAll(/\b(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*\[/g)];
    for (const declaration of declarations) {
      const name = declaration[1];
      const start = declaration.index + declaration[0].length;
      let depth = 1;
      let end = start;
      for (; end < source.length && depth > 0; end += 1) {
        if (source[end] === "[") depth += 1;
        else if (source[end] === "]") depth -= 1;
      }
      const body = source.slice(start, end - 1);
      if (!/\bpath\s*:/.test(body)) continue;
      const children = [...body.matchAll(/\bchildren\s*:\s*([A-Za-z_$][\w$]*)/g)].map(
        (match) => match[1],
      );
      graph.set(`${file}:${name}`, {
        file,
        name,
        children,
        display: `${path.relative(repo, file)}:${name}`,
      });
    }
  }
  const visiting = new Set();
  const visited = new Set();
  function visit(node, trail) {
    if (visiting.has(node)) {
      const cycle = [...trail, node.display].join(" -> ");
      fail("ROUTE_INVENTORY_CYCLE", cycle);
    }
    if (visited.has(node)) return;
    visiting.add(node);
    for (const childName of node.children) {
      const child = graph.get(`${node.file}:${childName}`);
      if (child) visit(child, [...trail, node.display]);
    }
    visiting.delete(node);
    visited.add(node);
  }
  for (const node of graph.values()) visit(node, []);
}

function registryEntries(files, repo) {
  const entries = [];
  for (const file of files) {
    const relative = path.relative(repo, file);
    const lower = relative.toLowerCase();
    const rawSource = fs.readFileSync(file, "utf8");
    const source = SOURCE_EXTENSIONS.has(path.extname(file)) ? stripComments(rawSource) : rawSource;
    if (/(^|\/)[^/]*(nav|menu)[^/]*\.(jsx?|tsx?)$/.test(lower)) {
      for (const match of source.matchAll(/\b(?:to|href)\s*=\s*(["'])(\/[^"']*)\1/g)) {
        entries.push({
          path: normalizeRegistryPath(match[2]),
          registry: "navigation",
          source: `${relative}:${lineNumber(source, match.index)}`,
        });
      }
    }
    if (/sitemap[^/]*\.xml$/.test(lower)) {
      for (const match of source.matchAll(/<loc>\s*([^<]+)\s*<\/loc>/g)) {
        entries.push({
          path: normalizeRegistryPath(match[1]),
          registry: "sitemap",
          source: `${relative}:${lineNumber(source, match.index)}`,
        });
      }
    }
    if (/(^|\/)(e2e|tests?)\//.test(lower) || /\.(spec|test)\.[jt]sx?$/.test(lower)) {
      for (const match of source.matchAll(/\b(?:goto|visit)\(\s*(["'])(\/[^"']*)\1/g)) {
        entries.push({
          path: normalizeRegistryPath(match[2]),
          registry: "e2e",
          source: `${relative}:${lineNumber(source, match.index)}`,
        });
      }
    }
  }
  return entries;
}

function routeMatches(routePath, concretePath) {
  const pattern = routePath
    .split("/")
    .map((segment) => {
      if (!segment) return "";
      if (segment === "*") return ".*";
      if (segment.startsWith(":")) return "[^/]+";
      return segment.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    })
    .join("/");
  return new RegExp(`^${pattern}/?$`).test(concretePath);
}

function recordMismatch(records, mismatch) {
  const key = `${mismatch.kind}\u0000${mismatch.path}\u0000${mismatch.reason}`;
  const record = records.get(key) ?? {
    path: mismatch.path,
    kind: mismatch.kind,
    reason: mismatch.reason,
    sources: [],
  };
  for (const source of mismatch.sources) {
    if (!record.sources.includes(source)) record.sources.push(source);
  }
  records.set(key, record);
}

function reconcile(discoveries, registries) {
  const routesByPath = new Map();
  for (const discovery of discoveries) {
    const record = routesByPath.get(discovery.path) ?? {
      path: discovery.path,
      status: discovery.path.includes("*") ? "excluded" : "runtime-eligible",
      reason: discovery.path.includes("*") ? "catch-all route is not a concrete runtime URL" : null,
      lazy: false,
      sources: [],
      registries: [],
    };
    record.lazy ||= discovery.lazy;
    if (!record.sources.includes(discovery.source)) record.sources.push(discovery.source);
    routesByPath.set(discovery.path, record);
  }
  const mismatchesByKey = new Map();
  for (const entry of registries) {
    const matches = [...routesByPath.values()].filter(
      (route) => route.status === "runtime-eligible" && routeMatches(route.path, entry.path),
    );
    if (matches.length === 0) {
      recordMismatch(mismatchesByKey, {
        path: entry.path,
        kind: "registry-only",
        reason: `${entry.registry} entry has no discovered React Router route`,
        sources: [entry.source],
      });
    } else {
      for (const route of matches) {
        const evidence = `${entry.registry}:${entry.source}:${entry.path}`;
        if (!route.registries.includes(evidence)) route.registries.push(evidence);
      }
    }
  }
  for (const route of routesByPath.values()) {
    if (route.status === "runtime-eligible" && route.registries.length === 0) {
      recordMismatch(mismatchesByKey, {
        path: route.path,
        kind: "route-only",
        reason: "route is absent from navbar/menu, sitemap, and E2E registries",
        sources: route.sources,
      });
    }
  }
  const routes = [...routesByPath.values()].sort((left, right) => left.path.localeCompare(right.path));
  const finalPaths = routes.map((route) => route.path);
  if (new Set(finalPaths).size !== finalPaths.length) {
    fail("ROUTE_INVENTORY_DUPLICATE", "final inventory contains duplicate canonical routes");
  }
  for (const route of routes) {
    if (!route.status || (route.status === "excluded" && !route.reason)) {
      fail("ROUTE_INVENTORY_MISSING_CLASSIFICATION", route.path);
    }
    route.sources.sort();
    route.registries.sort();
  }
  const mismatches = [...mismatchesByKey.values()].sort((left, right) => {
    const pathOrder = left.path.localeCompare(right.path);
    return pathOrder || left.kind.localeCompare(right.kind);
  });
  for (const mismatch of mismatches) mismatch.sources.sort();
  return { routes, mismatches };
}

function markdown(inventory) {
  const routeRows = inventory.routes.map(
    (route) => `| \`${route.path}\` | ${route.status} | ${route.reason ?? "—"} | ${route.lazy ? "yes" : "no"} |`,
  );
  const mismatchRows = inventory.mismatches.map(
    (item) => `| \`${item.path}\` | ${item.kind} | ${item.reason} |`,
  );
  return [
    "# Route inventory",
    "",
    `Framework: ${inventory.framework}`,
    "",
    "## Routes",
    "",
    "| Path | Classification | Reason | Lazy |",
    "| --- | --- | --- | --- |",
    ...routeRows,
    "",
    "## Registry mismatches",
    "",
    "| Path | Kind | Reason |",
    "| --- | --- | --- |",
    ...(mismatchRows.length ? mismatchRows : ["| — | — | None |"]),
    "",
  ].join("\n");
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (!args) return;
  const { repo, outputDir } = args;
  if (!fs.statSync(repo, { throwIfNoEntry: false })?.isDirectory()) {
    fail("ROUTE_INVENTORY_REPO_NOT_FOUND", repo, 2);
  }
  const files = walk(repo, outputDir);
  assertNoRouteCollectionCycles(files, repo);
  const discoveries = discoverReactRouter(files, repo);
  if (discoveries.length === 0) fail("ROUTE_INVENTORY_NO_ROUTES", "no React Router routes found");
  const { routes, mismatches } = reconcile(discoveries, registryEntries(files, repo));
  const inventory = { schema_version: 1, framework: "react-router", routes, mismatches };
  fs.mkdirSync(outputDir, { recursive: true });
  fs.writeFileSync(path.join(outputDir, "route-inventory.json"), `${JSON.stringify(inventory, null, 2)}\n`);
  fs.writeFileSync(path.join(outputDir, "route-inventory.md"), markdown(inventory));
  process.stdout.write(`route inventory: ${routes.length} routes, ${mismatches.length} mismatches\n`);
}

try {
  main();
} catch (error) {
  if (!(error instanceof RouteInventoryError)) throw error;
  process.stderr.write(`${error.code}: ${error.message}\n`);
  process.exitCode = error.exitCode;
}
