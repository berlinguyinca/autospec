// Cli.java — CLI entry point
package com.example;

public class Cli {
    public static void main(String[] args) {
        if (args.length < 1) {
            System.err.println("Usage: hello <name>");
            System.exit(1);
        }
        System.out.println(Lib.greet(args[0]));
    }
}
