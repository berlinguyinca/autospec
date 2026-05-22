; Go tree-sitter queries for autospec-docs walker
; Captures: exports (exported identifiers start with uppercase), entry_points, imports

; ── Exported functions ────────────────────────────────────────────────────────

(function_declaration
  name: (identifier) @export.name
  parameters: (parameter_list) @export.params
  (#match? @export.name "^[A-Z]")
  (#set! export.kind "function"))

; ── Exported types (struct, interface, type alias) ───────────────────────────

(type_declaration
  (type_spec
    name: (type_identifier) @export.name
    (#match? @export.name "^[A-Z]"))
  (#set! export.kind "type"))

; ── Exported constants and variables ─────────────────────────────────────────

(const_declaration
  (const_spec
    name: (identifier) @export.name
    (#match? @export.name "^[A-Z]"))
  (#set! export.kind "const"))

(var_declaration
  (var_spec
    name: (identifier) @export.name
    (#match? @export.name "^[A-Z]"))
  (#set! export.kind "const"))

; ── CLI entry points (func main()) ───────────────────────────────────────────

(function_declaration
  name: (identifier) @entry.main
  (#eq? @entry.main "main")
  (#set! entry.kind "cli_command"))

; ── HTTP routes (net/http and gorilla/mux / gin / echo patterns) ─────────────

; http.HandleFunc("/path", handler)
(call_expression
  function: (selector_expression
    field: (field_identifier) @_hf
    (#match? @_hf "^(HandleFunc|Handle|GET|POST|PUT|PATCH|DELETE|Any|Group)$"))
  arguments: (argument_list
    (interpreted_string_literal) @entry.route)
  (#set! entry.kind "http_route"))

; ── Import statements ─────────────────────────────────────────────────────────

(import_spec
  path: (interpreted_string_literal) @import.source)

(import_spec
  name: (identifier) @import.name
  path: (interpreted_string_literal) @import.source)
