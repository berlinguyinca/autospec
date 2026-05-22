; Python tree-sitter queries for autospec-docs walker
; Captures: exports (public top-level functions/classes), entry_points, imports

; ── Public functions (not prefixed with _) ────────────────────────────────────

(function_definition
  name: (identifier) @export.name
  parameters: (parameters) @export.params
  (#not-match? @export.name "^_")
  (#set! export.kind "function"))

; ── Public classes ────────────────────────────────────────────────────────────

(class_definition
  name: (identifier) @export.name
  (#not-match? @export.name "^_")
  (#set! export.kind "class"))

; ── Top-level constants (ALL_CAPS or explicitly __all__) ─────────────────────

(expression_statement
  (assignment
    left: (identifier) @export.name
    (#match? @export.name "^[A-Z][A-Z0-9_]+$"))
  (#set! export.kind "const"))

(expression_statement
  (assignment
    left: (identifier) @_all (#eq? @_all "__all__"))
  (#set! export.kind "const"))

; ── CLI entry points ─────────────────────────────────────────────────────────
; if __name__ == "__main__":

(if_statement
  condition: (comparison_operator
    (identifier) @_name (#eq? @_name "__name__")
    (string) @_main (#match? @_main "__main__"))
  (#set! entry.kind "cli_command"))

; ── HTTP routes (Flask / FastAPI / Django patterns) ──────────────────────────

; @app.route("/path")
(decorated_definition
  (decorator
    (call
      function: (attribute
        attribute: (identifier) @_route (#match? @_route "^(route|get|post|put|patch|delete)$"))
      arguments: (argument_list
        (string) @entry.route)))
  (#set! entry.kind "http_route"))

; ── Import statements ─────────────────────────────────────────────────────────

(import_statement
  name: (dotted_name) @import.source)

(import_from_statement
  module_name: (dotted_name) @import.source
  name: (dotted_name) @import.name)

(import_from_statement
  module_name: (dotted_name) @import.source
  name: (aliased_import
    name: (dotted_name) @import.name))
