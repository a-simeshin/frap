package io.github.kotlerdev.frap.mcp.stdio;

import static org.assertj.core.api.Assertions.assertThat;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.File;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.TimeUnit;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

/**
 * End-to-end MCP handshake against the REAL built stdio fat jar.
 *
 * <p>Spawns {@code java -jar frap-mcp-stdio.jar} as a subprocess (a real
 * {@link ProcessBuilder} / child_process) and speaks newline-delimited MCP JSON-RPC
 * 2.0 over its stdin/stdout: {@code initialize} &rarr; {@code notifications/initialized}
 * &rarr; {@code tools/list} &rarr; one {@code tools/call}. The jar boots the full
 * Spring AI MCP server, which in turn loads {@code frap-core-rpc}, so this is the true
 * transport path with no mocks.</p>
 *
 * <p>The MCP stdio transport is line-delimited JSON (one JSON object per line), not the
 * LSP-style {@code Content-Length} framing.</p>
 */
class StdioHandshakeIT {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private static final Duration BOOT_TIMEOUT = Duration.ofSeconds(30);
    private static final List<String> EXPECTED_TOOLS = List.of(
        "frap_snapshot_script",
        "frap_help",
        "frap_build_element_map",
        "frap_generate_page_object",
        "frap_filter_element_map",
        "frap_heal");

    private static Process process;
    private static BufferedWriter stdin;
    private static BufferedReader stdout;

    @BeforeAll
    static void startServer() throws Exception {
        File jar = resolveJar();
        String java = System.getProperty("java.home") + File.separator + "bin" + File.separator + "java";

        process = new ProcessBuilder(java, "-jar", jar.getAbsolutePath())
            .redirectError(ProcessBuilder.Redirect.INHERIT)
            .start();

        stdin = new BufferedWriter(new OutputStreamWriter(process.getOutputStream(), StandardCharsets.UTF_8));
        stdout = new BufferedReader(new InputStreamReader(process.getInputStream(), StandardCharsets.UTF_8));
    }

    @AfterAll
    static void stopServer() {
        if (process != null) {
            process.destroy();
            try {
                if (!process.waitFor(10, TimeUnit.SECONDS)) {
                    process.destroyForcibly();
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                process.destroyForcibly();
            }
        }
    }

    @Test
    void initialize_and_tools_list_expose_six_tools_over_stdio() throws Exception {
        // 1. initialize
        ObjectNode initParams = MAPPER.createObjectNode();
        initParams.put("protocolVersion", "2024-11-05");
        initParams.set("capabilities", MAPPER.createObjectNode());
        ObjectNode clientInfo = MAPPER.createObjectNode();
        clientInfo.put("name", "frap-mcp-stdio-it");
        clientInfo.put("version", "1.0.0");
        initParams.set("clientInfo", clientInfo);

        JsonNode initResponse = request(1, "initialize", initParams);
        assertThat(initResponse.hasNonNull("error"))
            .as("initialize must not return an error: %s", initResponse)
            .isFalse();
        assertThat(initResponse.path("result").path("serverInfo").path("name").asText())
            .as("server must identify itself in initialize result")
            .isNotBlank();

        // 2. notifications/initialized (no response expected)
        notify("notifications/initialized", MAPPER.createObjectNode());

        // 3. tools/list
        JsonNode listResponse = request(2, "tools/list", MAPPER.createObjectNode());
        JsonNode toolsNode = listResponse.path("result").path("tools");
        assertThat(toolsNode.isArray()).as("tools/list result must be an array").isTrue();

        List<String> toolNames = new ArrayList<>();
        toolsNode.forEach(t -> toolNames.add(t.path("name").asText()));

        assertThat(toolNames)
            .as("stdio server must expose exactly the six frap tools")
            .containsExactlyInAnyOrderElementsOf(EXPECTED_TOOLS);

        // 4. one tools/call — frap_snapshot_script needs no arguments and never touches
        //    the core client, so it is the cheapest deterministic end-to-end call.
        ObjectNode callParams = MAPPER.createObjectNode();
        callParams.put("name", "frap_snapshot_script");
        callParams.set("arguments", MAPPER.createObjectNode());

        JsonNode callResponse = request(3, "tools/call", callParams);
        assertThat(callResponse.hasNonNull("error"))
            .as("tools/call must not return a JSON-RPC error: %s", callResponse)
            .isFalse();
        JsonNode result = callResponse.path("result");
        assertThat(result.path("isError").asBoolean(false))
            .as("frap_snapshot_script result must not be an error: %s", result)
            .isFalse();
        assertThat(result.path("content").isArray() && result.path("content").size() > 0)
            .as("frap_snapshot_script must return non-empty content")
            .isTrue();
    }

    // --- JSON-RPC helpers over line-delimited stdio ---------------------------------

    private JsonNode request(int id, String method, JsonNode params) throws Exception {
        ObjectNode req = MAPPER.createObjectNode();
        req.put("jsonrpc", "2.0");
        req.put("id", id);
        req.put("method", method);
        req.set("params", params);

        writeLine(MAPPER.writeValueAsString(req));
        return readResponseFor(id);
    }

    private void notify(String method, JsonNode params) throws Exception {
        ObjectNode req = MAPPER.createObjectNode();
        req.put("jsonrpc", "2.0");
        req.put("method", method);
        req.set("params", params);
        writeLine(MAPPER.writeValueAsString(req));
    }

    private void writeLine(String json) throws Exception {
        stdin.write(json);
        stdin.write("\n");
        stdin.flush();
    }

    /**
     * Reads newline-delimited JSON from the server until a response whose {@code id}
     * matches the request id is seen, skipping any server-initiated notifications.
     */
    private JsonNode readResponseFor(int id) throws Exception {
        long deadline = System.nanoTime() + BOOT_TIMEOUT.toNanos();
        while (System.nanoTime() < deadline) {
            if (!process.isAlive() && !stdout.ready()) {
                throw new IllegalStateException("stdio server exited before answering id=" + id);
            }
            if (!stdout.ready()) {
                Thread.sleep(50);
                continue;
            }
            String line = stdout.readLine();
            if (line == null) {
                throw new IllegalStateException("stdio stream closed before answering id=" + id);
            }
            line = line.trim();
            if (line.isEmpty() || !line.startsWith("{")) {
                continue;
            }
            JsonNode node = MAPPER.readTree(line);
            if (node.has("id") && node.path("id").asInt() == id) {
                return node;
            }
            // otherwise a notification / unrelated message: keep reading
        }
        throw new IllegalStateException("timed out waiting for response id=" + id);
    }

    private static File resolveJar() {
        // Preferred: the reactor-built fat jar (package precedes integration-test, so it
        // exists by the time failsafe runs). finalName is frap-mcp-stdio (see pom).
        File primary = new File("target/frap-mcp-stdio.jar");
        if (primary.isFile()) {
            return primary;
        }
        // Fallback: absolute path from module root, in case cwd differs.
        File fromModule = new File(
            System.getProperty("user.dir"), "target/frap-mcp-stdio.jar");
        if (fromModule.isFile()) {
            return fromModule;
        }
        throw new IllegalStateException(
            "frap-mcp-stdio fat jar not found at " + primary.getAbsolutePath()
                + " — run `mvn -pl frap-mcp-stdio package` (or the full `verify`) first. "
                + "The reactor builds frap-core-rpc into the jar during package.");
    }
}
