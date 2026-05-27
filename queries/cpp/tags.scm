; Vendored from tree-sitter-cpp master @ 8b5b49eb196bec7040441bee33b2c9a4838d6967
; Source: https://github.com/tree-sitter/tree-sitter-cpp/blob/master/queries/tags.scm
; License: MIT
;
; ## blazing-art-mcp augmentation
;
; Upstream tree-sitter-cpp tags.scm has zero @reference.* captures, like C.
; We add call_expression refs and type_identifier refs.

(struct_specifier name: (type_identifier) @name body:(_)) @definition.class

(declaration type: (union_specifier name: (type_identifier) @name)) @definition.class

(function_declarator declarator: (identifier) @name) @definition.function

(function_declarator declarator: (field_identifier) @name) @definition.function

(function_declarator declarator: (qualified_identifier scope: (namespace_identifier) @local.scope name: (identifier) @name)) @definition.method

(type_definition declarator: (type_identifier) @name) @definition.type

(enum_specifier name: (type_identifier) @name) @definition.type

(class_specifier name: (type_identifier) @name) @definition.class

; --- blazing-art-mcp augmentation ---

(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (field_expression
    field: (field_identifier) @name)) @reference.call

(type_identifier) @name @reference.type
