// Server.java — Spring Boot HTTP server
package com.example;

import org.springframework.boot.SpringApplication;
import org.springframework.boot.autoconfigure.SpringBootApplication;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.PathVariable;
import org.springframework.web.bind.annotation.RestController;

@SpringBootApplication
@RestController
public class Server {
    public static void main(String[] args) {
        SpringApplication.run(Server.class, args);
    }

    @GetMapping("/greet/{name}")
    public String greet(@PathVariable String name) {
        return Lib.greet(name);
    }
}
