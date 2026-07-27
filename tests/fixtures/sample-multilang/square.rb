# Test fixture: 1 module, 1 class, 2 methods, 1 reference (call).

module Geometry
  class Square
    def initialize(side)
      @side = side
    end

    def area
      @side * @side
    end
  end
end

s = Geometry::Square.new(5)
puts s.area
