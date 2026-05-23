// Test fixture for blazing-art-mcp integration tests.
// Declarations: struct Circle, struct Rectangle, fn area, impl Circle.
// Whether the impl method counts as a separate declaration depends on
// tree-sitter-rust's grammar — the integration tests lock the actual count.

pub struct Circle {
    pub radius: f64,
}

pub struct Rectangle {
    pub width: f64,
    pub height: f64,
}

pub fn area(r: &Rectangle) -> f64 {
    r.width * r.height
}

impl Circle {
    pub fn new(radius: f64) -> Self {
        Circle { radius }
    }
}
