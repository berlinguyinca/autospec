; TypeScript tree-sitter queries for autospec-docs walker
; Captures: exports, entry_points, imports

; ── Exported functions ────────────────────────────────────────────────────────

(export_statement
  declaration: (function_declaration
    name: (identifier) @export.name
    parameters: (formal_parameters) @export.params)
  (#set! export.kind "function"))

(export_statement
  declaration: (lexical_declaration
    (variable_declarator
      name: (identifier) @export.name
      value: [(arrow_function) (function_expression)])) @export.decl
  (#set! export.kind "function"))

; ── Exported classes ──────────────────────────────────────────────────────────

(export_statement
  declaration: (class_declaration
    name: (type_identifier) @export.name)
  (#set! export.kind "class"))

; ── Exported type aliases / interfaces ───────────────────────────────────────

(export_statement
  declaration: (type_alias_declaration
    name: (type_identifier) @export.name)
  (#set! export.kind "type"))

(export_statement
  declaration: (interface_declaration
    name: (type_identifier) @export.name)
  (#set! export.kind "type"))

; ── Exported constants ────────────────────────────────────────────────────────

(export_statement
  declaration: (lexical_declaration
    (variable_declarator
      name: (identifier) @export.name))
  (#set! export.kind "const"))

; Named re-exports: export { foo, bar }
(export_statement
  (export_clause
    (export_specifier
      name: (identifier) @export.name))
  (#set! export.kind "const"))

; ── CLI entry points ─────────────────────────────────────────────────────────
; Detect `#!/usr/bin/env node` shebang at top of file

(program
  (hash_bang_line) @entry.shebang
  (#set! entry.kind "cli_command"))

; ── HTTP routes (Express / Fastify / Hono patterns) ─────────────────────────

(call_expression
  function: (member_expression
    object: (identifier)
    property: (property_identifier) @http.method
    (#match? @http.method "^(get|post|put|patch|delete|head|options|all|route)$"))
  arguments: (arguments
    (string) @entry.route)
  (#set! entry.kind "http_route"))

; ── Import statements ─────────────────────────────────────────────────────────

(import_statement
  source: (string (string_fragment) @import.source)
  (import_clause
    (named_imports
      (import_specifier
        name: (identifier) @import.name))))

(import_statement
  source: (string (string_fragment) @import.source)
  (import_clause
    (identifier) @import.name))
