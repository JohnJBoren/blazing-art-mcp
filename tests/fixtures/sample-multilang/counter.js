// Test fixture for JS: class Counter, function increment, call refs.

class Counter {
  constructor(initial) {
    this.value = initial;
  }

  bump() {
    this.value += 1;
    return this.value;
  }
}

function increment(c) {
  return c.bump();
}

const c = new Counter(0);
increment(c);
