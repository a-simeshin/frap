package io.github.kotlerdev.frap.mcp.http;

import org.springframework.boot.SpringApplication;
import org.springframework.boot.autoconfigure.SpringBootApplication;
import org.springframework.context.annotation.ComponentScan;

/**
 * Executable Spring Boot entry point for the frap MCP server over the
 * streamable-http (Spring MVC) transport.
 *
 * <p>Unlike the stdio runner, this is a normal web application: it starts an
 * embedded servlet container and serves the MCP protocol over HTTP at the
 * {@code /mcp} endpoint (configurable via
 * {@code spring.ai.mcp.server.streamable-http.mcp-endpoint}). The streamable-http
 * transport is selected by {@code spring.ai.mcp.server.protocol=STREAMABLE} in
 * {@code application.properties}, backed by the
 * {@code spring-ai-starter-mcp-server-webmvc} starter.</p>
 *
 * <p>Component-scans {@code io.github.kotlerdev.frap.mcp.tools} so the
 * {@code @McpTool} service ({@code FrapTools}) and the {@code @Lazy}
 * {@code FrapCoreClient} bean from {@code frap-mcp-tools} are picked up.
 * Spring AI's annotation auto-configuration discovers the {@code @McpTool}
 * methods and registers all 5 tools with the streamable-http server; no
 * explicit {@code ToolCallbackProvider} bean is required (mirrors the stdio
 * runner, which also relies purely on annotation auto-config).</p>
 */
@SpringBootApplication
@ComponentScan(basePackages = {
    "io.github.kotlerdev.frap.mcp.http",
    "io.github.kotlerdev.frap.mcp.tools"
})
public class HttpApplication {

    public static void main(String[] args) {
        SpringApplication.run(HttpApplication.class, args);
    }
}
