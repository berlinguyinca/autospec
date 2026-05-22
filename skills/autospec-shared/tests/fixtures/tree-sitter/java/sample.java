// sample.java — Java fixture for autospec-docs walker unit tests.

package com.example;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Paths;

public class ConfigLoader {

    public static final int DEFAULT_PORT = 8080;

    public static String loadConfig(String path) throws IOException {
        return Files.readString(Paths.get(path));
    }

    public static String formatAddress(String host, int port) {
        return host + ":" + port;
    }

    public static void main(String[] args) {
        System.out.println(formatAddress("localhost", DEFAULT_PORT));
    }
}
