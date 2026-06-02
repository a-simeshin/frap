package io.github.kotlerdev.frap.mcp.stdio;

import org.springframework.boot.SpringApplication;
import org.springframework.boot.autoconfigure.SpringBootApplication;
import org.springframework.context.annotation.ComponentScan;

/**
 * Executable Spring Boot entry point for the frap MCP server over the stdio transport.
 *
 * <p>stdout is reserved for the MCP JSON-RPC protocol, so the banner and console
 * logging are disabled in {@code application.properties}; logs go to a file.</p>
 *
 * <p>Component-scans {@code io.github.kotlerdev.frap.mcp.tools} so the (future)
 * {@code @McpTool} service and {@code FrapCoreClient} bean from {@code frap-mcp-tools}
 * are picked up and auto-registered with the MCP server by Spring AI.</p>
 */
@SpringBootApplication
@ComponentScan(basePackages = {
    "io.github.kotlerdev.frap.mcp.stdio",
    "io.github.kotlerdev.frap.mcp.tools"
})
public class StdioApplication {

    public static void main(String[] args) {
        SpringApplication.run(StdioApplication.class, args);
    }
}
