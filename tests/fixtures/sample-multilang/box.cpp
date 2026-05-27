// Test fixture for C++: class Box, method volume, free fn cube, call ref to volume.

class Box {
public:
    int width;
    int height;
    int depth;

    int volume() const {
        return width * height * depth;
    }
};

int cube(int side) {
    Box b;
    b.width = side;
    b.height = side;
    b.depth = side;
    return b.volume();
}
