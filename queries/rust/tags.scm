; Vendored from tree-sitter-rust master @ 77a3747266f4d621d0757825e6b11edcbf991ca5
; Source: https://github.com/tree-sitter/tree-sitter-rust/blob/master/queries/tags.scm
; License: MIT (matches the grammar's license)
;
; ## blazing-art-mcp augmentations
;
; Upstream's @reference.* coverage is limited to call sites and impl blocks.
; In real Rust code, a huge fraction of "where is X used?" answers live in
; type positions (`fn foo(m: &Memory)`, `Arc<Memory>`, struct field types) and
; in scoped paths (`Memory::new(...)`). We add the most common pattern below
; — `scoped_identifier path: (identifier)` — which catches `Memory::new`,
; `Foo::bar`, `crate::module::Type` etc.
;
; Known gaps (deliberately not yet captured to avoid double-emitting at
; declaration sites): bare `type_identifier` references in field types,
; parameter types, return types, generic arguments. Adding these requires
; either negative predicates against declaration positions or per-context
; patterns; tracked for a future tuning pass.
;
; ADT definitions

(struct_item
    name: (type_identifier) @name) @definition.class

(enum_item
    name: (type_identifier) @name) @definition.class

(union_item
    name: (type_identifier) @name) @definition.class

; type aliases

(type_item
    name: (type_identifier) @name) @definition.class

; method definitions

(declaration_list
    (function_item
        name: (identifier) @name) @definition.method)

; function definitions

(function_item
    name: (identifier) @name) @definition.function

; trait definitions
(trait_item
    name: (type_identifier) @name) @definition.interface

; module definitions
(mod_item
    name: (identifier) @name) @definition.module

; macro definitions

(macro_definition
    name: (identifier) @name) @definition.macro

; references

(call_expression
    function: (identifier) @name) @reference.call

(call_expression
    function: (field_expression
        field: (field_identifier) @name)) @reference.call

(macro_invocation
    macro: (identifier) @name) @reference.call

; implementations

(impl_item
    trait: (type_identifier) @name) @reference.implementation

(impl_item
    type: (type_identifier) @name
    !trait) @reference.implementation

; --- blazing-art-mcp augmentation ---
;
; Scoped-path references: `Memory::new`, `Foo::bar`, `crate::module::Type`.
; These are not captured upstream but make up a large fraction of how Rust
; code refers to types and functions.

(scoped_identifier
    path: (identifier) @name) @reference.path

; --- v0.3 Task 2: `use` statement reference captures ---

; `use foo::Bar` — captures Bar.
(use_declaration
    argument: (scoped_identifier
        name: (identifier) @name)) @reference.class

; `use foo::Bar as Baz` — captures Bar.
(use_as_clause
    path: (scoped_identifier
        name: (identifier) @name)) @reference.class

; `use foo::{Bar, Baz}` — captures each list member.
(use_list
    (identifier) @name) @reference.class

(use_list
    (scoped_identifier
        name: (identifier) @name)) @reference.class

