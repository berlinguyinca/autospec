; JavaScript tree-sitter queries for autospec-docs walker
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
    name: (identifier) @export.name)
  (#set! export.kind "class"))

; ── Exported constants ────────────────────────────────────────────────────────

(export_statement
  declaration: (lexical_declaration
    (variable_declarator
      name: (identifier) @export.name))
  (#set! export.kind "const"))

; Named re-exports
(export_statement
  (export_clause
    (export_specifier
      name: (identifier) @export.name))
  (#set! export.kind "const"))

; module.exports = { ... }
(expression_statement
  (assignment_expression
    left: (member_expression
      object: (identifier) @_mod (#eq? @_mod "module")
      property: (property_identifier) @_exp (#eq? @_exp "exports"))
    right: (object
      (pair
        key: (property_identifier) @export.name)))
  (#set! export.kind "const"))

; ── CLI entry points ─────────────────────────────────────────────────────────

(program
  (hash_bang_line) @entry.shebang
  (#set! entry.kind "cli_command"))

; ── HTTP routes ───────────────────────────────────────────────────────────────

(call_expression
  function: (member_expression
    object: (identifier)
    property: (property_identifier) @http.method
    (#match? @http.method "^(get|post|put|patch|delete|head|options|all|route)$"))
  arguments: (arguments
    (string) @entry.route)
  (#set! entry.kind "http_route"))

; ── Import statements ─────────────────────────────────────────────────────────
; Note: web-tree-sitter does not support non-field anonymous children in patterns,
; so import names are extracted via node traversal in walker.mjs buildOutput().
; We only capture the source here; names are resolved programmatically.

(import_statement
  source: (string (string_fragment) @import.source))

; require()
(call_expression
  function: (identifier) @_req (#eq? @_req "require")
  arguments: (arguments
    (string (string_fragment) @import.source)))
