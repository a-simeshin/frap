package io.github.kotlerdev.frap.mcp.http.local;

import org.springframework.boot.SpringApplication;
import org.springframework.boot.autoconfigure.SpringBootApplication;
import org.springframework.context.annotation.ComponentScan;

/**
 * Executable Spring Boot entry point for the frap MCP server over the
 * streamable-http (Spring MVC) transport, running in file mode.
 *
 * <p>Like the {@code frap-mcp-http} runner, this is a normal web application: it
 * starts an embedded servlet container and serves the MCP protocol over HTTP at
 * the {@code /mcp} endpoint (configurable via
 * {@code spring.ai.mcp.server.streamable-http.mcp-endpoint}). The streamable-http
 * transport is selected by {@code spring.ai.mcp.server.protocol=STREAMABLE} in
 * {@code application.properties}, backed by the
 * {@code spring-ai-starter-mcp-server-webmvc} starter.</p>
 *
 * <p>Unlike {@code frap-mcp-http}, this runner uses {@code frap.io.mode=file}: it
 * exchanges local file paths instead of inline JSON, like the stdio runner, and
 * therefore requires a filesystem shared with the client.</p>
 *
 * <p>Component-scans {@code io.github.kotlerdev.frap.mcp.tools} so the file-mode
 * {@code @McpTool} service ({@code FrapFileTools}) and the {@code @Lazy}
 * {@code FrapCoreClient} bean from {@code frap-mcp-tools} are picked up.
 * Spring AI's annotation auto-configuration discovers the {@code @McpTool}
 * methods and registers the tools with the streamable-http server; no explicit
 * {@code ToolCallbackProvider} bean is required (mirrors the other runners,
 * which also rely purely on annotation auto-config). {@code FrapFileTools} is
 * activated via {@code @ConditionalOnProperty} keyed on {@code frap.io.mode}.</p>
 */
@SpringBootApplication
@ComponentScan(basePackages = {
    "io.github.kotlerdev.frap.mcp.http.local",
    "io.github.kotlerdev.frap.mcp.tools"
})
public class HttpLocalApplication {

    public static void main(String[] args) {
        SpringApplication.run(HttpLocalApplication.class, args);
    }
}
