// Test fixture for blazing-art-mcp integration tests.
// Declarations: 1 struct, 1 fn.

pub struct Coordinate {
    pub x: f64,
    pub y: f64,
}

pub fn origin() -> Coordinate {
    Coordinate { x: 0.0, y: 0.0 }
}
