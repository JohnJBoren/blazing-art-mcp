// Test fixture for multi-language ingest e2e test.
// Declarations: 1 type (Vec3), 1 method (Norm), 1 function (main).
// References: 1 call (Norm), 1 type ref (Vec3).

package main

import "math"

type Vec3 struct {
	X, Y, Z float64
}

func (v Vec3) Norm() float64 {
	return math.Sqrt(v.X*v.X + v.Y*v.Y + v.Z*v.Z)
}

func main() {
	v := Vec3{X: 3, Y: 4, Z: 0}
	_ = v.Norm()
}
