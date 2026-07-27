; Vendored from tree-sitter-python master @ 26855eabccb19c6abf499fbc5b8dc7cc9ab8bc64
; Source: https://github.com/tree-sitter/tree-sitter-python/blob/master/queries/tags.scm
; License: MIT
;
; ## blazing-art-mcp augmentation
;
; Upstream tree-sitter-python tags.scm only captures @reference.call. We add
; import-style and decorator references here — both are very common patterns
; that an agent asking "where is Foo used?" wants to find.

(class_definition
  name: (identifier) @name) @definition.class

(function_definition
  name: (identifier) @name) @definition.function

(call
  function: [
      (identifier) @name
      (attribute
        attribute: (identifier) @name)
  ]) @reference.call

; --- blazing-art-mcp augmentation ---

; `from x import Foo` — Foo is a reference to the imported symbol.
(import_from_statement
  name: (dotted_name (identifier) @name)) @reference.class

; `import x.Y` — Y is a reference (rough approximation; rooted at the identifier).
(import_statement
  name: (dotted_name (identifier) @name)) @reference.class

; `@decorator` — decorator names are references to whatever they bind to.
(decorator
  (identifier) @name) @reference.call

; `Foo.bar` style attribute access — captures both the receiver and the attribute.
; Receiver is captured as @reference.class to distinguish from a plain @reference.call.
(attribute
  object: (identifier) @name) @reference.class
