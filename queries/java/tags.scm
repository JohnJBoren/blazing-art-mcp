; Vendored from tree-sitter-java master @ e10607b45ff745f5f876bfa3e94fbcc6b44bdc11
; Source: https://github.com/tree-sitter/tree-sitter-java/blob/master/queries/tags.scm
; License: MIT

(class_declaration
  name: (identifier) @name) @definition.class

(method_declaration
  name: (identifier) @name) @definition.method

(method_invocation
  name: (identifier) @name
  arguments: (argument_list) @reference.call)

(interface_declaration
  name: (identifier) @name) @definition.interface

(type_list
  (type_identifier) @name) @reference.implementation

(object_creation_expression
  type: (type_identifier) @name) @reference.class

(superclass (type_identifier) @name) @reference.class

; --- v0.3 Task 2: import + field-type reference captures ---

; `import com.example.Foo;` — captures the trailing identifier.
(import_declaration
  (scoped_identifier
    name: (identifier) @name)) @reference.class

; `private SomeType field;` — captures the type as a class reference.
(field_declaration
  type: (type_identifier) @name) @reference.class

; `void method(SomeType param)` — captures the parameter type.
(formal_parameter
  type: (type_identifier) @name) @reference.class

; Generic argument: `List<Foo>` — captures Foo.
(type_arguments
  (type_identifier) @name) @reference.type

