; Vendored from tree-sitter-c master @ b780e47fc780ddc8da13afa35a3f4ed5c157823d
; Source: https://github.com/tree-sitter/tree-sitter-c/blob/master/queries/tags.scm
; License: MIT
;
; ## blazing-art-mcp augmentation
;
; Upstream tree-sitter-c tags.scm has ZERO @reference.* captures. Without
; augmentation `findReferences` would always return empty for C code. We add
; a minimal call_expression capture so function calls resolve.

(struct_specifier name: (type_identifier) @name body:(_)) @definition.class

(declaration type: (union_specifier name: (type_identifier) @name)) @definition.class

(function_declarator declarator: (identifier) @name) @definition.function

(type_definition declarator: (type_identifier) @name) @definition.type

(enum_specifier name: (type_identifier) @name) @definition.type

; --- blazing-art-mcp augmentation ---

(call_expression
  function: (identifier) @name) @reference.call

; Type usages (struct foo *p) — capture struct/union name in non-declaration
; positions. Note: this fires alongside the definition pattern when a struct
; declares itself, but the (role, span) dedup keeps each emission separate
; (def + ref are different roles even at the same span).
(type_identifier) @name @reference.type
