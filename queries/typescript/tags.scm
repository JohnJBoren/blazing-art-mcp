; Vendored from tree-sitter-typescript master @ 75b3874edb2dc714fb1fd77a32013d0f8699989f
; Source: https://github.com/tree-sitter/tree-sitter-typescript/blob/master/queries/tags.scm
; License: MIT
;
; The upstream query is geared toward type-system declarations
; (function_signature, method_signature) used by GitHub for code navigation
; over .d.ts surfaces. We augment below with declarations for actual
; .ts implementations: function_declaration, class_declaration,
; method_definition, type_alias_declaration, enum_declaration. Without
; the augmentation we would miss every concrete function and class in
; runtime code.

; --- upstream block (verbatim) ---

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

; --- blazing-art-mcp augmentation: concrete-implementation declarations ---

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

; call references (parallel to Rust's @reference.call)

(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (member_expression
    property: (property_identifier) @name)) @reference.call

; --- v0.3 Task 2: import-style reference captures ---

; `import { Foo } from "x"` — Foo is a reference.
(import_specifier
  name: (identifier) @name) @reference.class

; Generic type arguments: `Foo<Bar>` — Bar is a type reference.
(type_arguments
  (type_identifier) @name) @reference.type
