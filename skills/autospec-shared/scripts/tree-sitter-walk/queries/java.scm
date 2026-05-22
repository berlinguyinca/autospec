; Java tree-sitter queries for autospec-docs walker
; Captures: exports (public methods/classes), entry_points, imports

; ── Public classes ────────────────────────────────────────────────────────────

(class_declaration
  (modifiers) @_mods
  name: (identifier) @export.name
  (#match? @_mods "public")
  (#set! export.kind "class"))

(interface_declaration
  (modifiers) @_mods
  name: (identifier) @export.name
  (#match? @_mods "public")
  (#set! export.kind "type"))

; ── Public methods ────────────────────────────────────────────────────────────

(method_declaration
  (modifiers) @_mods
  name: (identifier) @export.name
  parameters: (formal_parameters) @export.params
  (#match? @_mods "public")
  (#set! export.kind "function"))

; ── Public constants (public static final) ───────────────────────────────────

(field_declaration
  (modifiers) @_mods
  declarator: (variable_declarator
    name: (identifier) @export.name)
  (#match? @_mods "public")
  (#set! export.kind "const"))

; ── CLI entry points (public static void main(String[] args)) ────────────────

(method_declaration
  (modifiers) @_mods
  type: (void_type)
  name: (identifier) @entry.main
  (#match? @_mods "public")
  (#match? @_mods "static")
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
