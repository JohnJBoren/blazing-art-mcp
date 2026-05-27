/* Test fixture for C: struct Point, fns origin and translate, call ref to translate. */

struct Point {
    int x;
    int y;
};

struct Point origin(void) {
    struct Point p = {0, 0};
    return p;
}

void translate(struct Point *p, int dx, int dy) {
    p->x += dx;
    p->y += dy;
}

int main(void) {
    struct Point p = origin();
    translate(&p, 1, 2);
    return p.x;
}
