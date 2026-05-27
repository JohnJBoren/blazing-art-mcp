// Test fixture: 1 class, 2 methods, references via object_creation + method_invocation.

public class Greeter {
    private String name;

    public Greeter(String name) {
        this.name = name;
    }

    public String greet() {
        return "hello " + this.name;
    }

    public static void main(String[] args) {
        Greeter g = new Greeter("world");
        System.out.println(g.greet());
    }
}
