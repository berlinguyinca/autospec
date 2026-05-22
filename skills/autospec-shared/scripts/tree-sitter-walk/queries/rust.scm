; Rust tree-sitter queries for autospec-docs walker
; Captures: exports (pub items), entry_points, imports (use declarations)

; ── Public functions ─────────────────────────────────────────────────────────

(function_item
  (visibility_modifier) @_pub (#eq? @_pub "pub")
  name: (identifier) @export.name
  parameters: (parameters) @export.params
  (#set! export.kind "function"))

; ── Public structs / enums / traits / type aliases ───────────────────────────

(struct_item
  (visibility_modifier) @_pub (#eq? @_pub "pub")
  name: (type_identifier) @export.name
  (#set! export.kind "type"))

(enum_item
  (visibility_modifier) @_pub (#eq? @_pub "pub")
  name: (type_identifier) @export.name
  (#set! export.kind "type"))

(trait_item
  (visibility_modifier) @_pub (#eq? @_pub "pub")
  name: (type_identifier) @export.name
  (#set! export.kind "type"))

(type_item
  (visibility_modifier) @_pub (#eq? @_pub "pub")
  name: (type_identifier) @export.name
  (#set! export.kind "type"))

; ── Public constants ─────────────────────────────────────────────────────────

(const_item
  (visibility_modifier) @_pub (#eq? @_pub "pub")
  name: (identifier) @export.name
  (#set! export.kind "const"))

; ── CLI entry points (fn main()) ─────────────────────────────────────────────

(function_item
  name: (identifier) @entry.main
  (#eq? @entry.main "main")
  (#set! entry.kind "cli_command"))

; ── HTTP routes (actix-web / axum / rocket attribute patterns) ───────────────

; #[get("/path")]  #[post("/path")]  etc.
(attribute_item
  (attribute
    (identifier) @_method
    (#match? @_method "^(get|post|put|patch|delete|head|options|route)$")
    arguments: (token_tree
      (string_literal) @entry.route))
  (#set! entry.kind "http_route"))

; ── Use declarations ──────────────────────────────────────────────────────────

(use_declaration
  argument: (scoped_identifier
    path: (identifier) @import.source
    name: (identifier) @import.name))

(use_declaration
  argument: (identifier) @import.source)

(use_declaration
  argument: (scoped_use_list
    path: (identifier) @import.source
    list: (use_list
      (identifier) @import.name)))
