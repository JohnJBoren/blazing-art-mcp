; TSX uses the same tags vocabulary as TypeScript. Upstream tree-sitter-typescript
; ships only one tags.scm at queries/tags.scm — there is no separate tsx/queries/tags.scm.
; We duplicate the TS file rather than symlink so the include-bytes! at compile
; time is straightforward.
;
; If you change queries/typescript/tags.scm, update this file too. The
; integration test `ingest_multilang_fixture` exercises both grammars.

; --- upstream block (verbatim from tree-sitter-typescript master) ---

(function_signature
  name: (identifier) @name) @definition.function

(method_signature
  name: (property_identifier) @name) @definition.method

(abstract_method_signature
  name: (property_identifier) @name) @definition.method

(abstract_class_declaration
  name: (type_identifier) @name) @definition.class

(module
  name: (identifier) @name) @definition.module

(interface_declaration
  name: (type_identifier) @name) @definition.interface

(type_annotation
  (type_identifier) @name) @reference.type

(new_expression
  constructor: (identifier) @name) @reference.class

; --- blazing-art-mcp augmentation ---

(function_declaration
  name: (identifier) @name) @definition.function

(class_declaration
  name: (type_identifier) @name) @definition.class

(method_definition
  name: (property_identifier) @name) @definition.method

(type_alias_declaration
  name: (type_identifier) @name) @definition.class

(enum_declaration
  name: (identifier) @name) @definition.class

(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (member_expression
    property: (property_identifier) @name)) @reference.call
