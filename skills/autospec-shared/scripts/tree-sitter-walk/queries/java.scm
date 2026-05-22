; Java tree-sitter queries for autospec-docs walker
; Captures: exports (public methods/classes), entry_points, imports

; ── Public classes ────────────────────────────────────────────────────────────

(class_declaration
  (modifiers
    (modifier) @_pub (#eq? @_pub "public"))
  name: (identifier) @export.name
  (#set! export.kind "class"))

(interface_declaration
  (modifiers
    (modifier) @_pub (#eq? @_pub "public"))
  name: (identifier) @export.name
  (#set! export.kind "type"))

; ── Public methods ────────────────────────────────────────────────────────────

(method_declaration
  (modifiers
    (modifier) @_pub (#eq? @_pub "public"))
  name: (identifier) @export.name
  parameters: (formal_parameters) @export.params
  (#set! export.kind "function"))

; ── Public constants (public static final) ───────────────────────────────────

(field_declaration
  (modifiers
    (modifier) @_pub (#eq? @_pub "public")
    (modifier) @_sta (#eq? @_sta "static")
    (modifier) @_fin (#eq? @_fin "final"))
  declarator: (variable_declarator
    name: (identifier) @export.name)
  (#set! export.kind "const"))

; ── CLI entry points (public static void main(String[] args)) ────────────────

(method_declaration
  (modifiers
    (modifier) @_pub (#eq? @_pub "public")
    (modifier) @_sta (#eq? @_sta "static"))
  type: (void_type)
  name: (identifier) @entry.main
  (#eq? @entry.main "main")
  (#set! entry.kind "cli_command"))

; ── HTTP routes (Spring @GetMapping / @PostMapping etc.) ─────────────────────

(annotation
  name: (identifier) @_ann
  (#match? @_ann "^(GetMapping|PostMapping|PutMapping|PatchMapping|DeleteMapping|RequestMapping)$")
  arguments: (annotation_argument_list
    (string_literal) @entry.route)
  (#set! entry.kind "http_route"))

; ── Import statements ─────────────────────────────────────────────────────────

(import_declaration
  (scoped_identifier) @import.source)
