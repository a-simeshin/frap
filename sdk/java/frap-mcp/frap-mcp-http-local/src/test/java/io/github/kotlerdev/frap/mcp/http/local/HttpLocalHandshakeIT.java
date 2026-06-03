package io.github.kotlerdev.frap.mcp.http.local;

import static org.assertj.core.api.Assertions.assertThat;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.net.ServerSocket;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

/**
 * End-to-end MCP handshake against the REAL built {@code frap-mcp-http-local} fat jar,
 * spoken over genuine MCP JSON-RPC 2.0 <b>HTTP</b> (Spring AI streamable-http transport).
 *
 * <p>Spawns {@code java -jar frap-mcp-http-local.jar --server.port=<free>} as a subprocess.
 * That jar boots the full Spring AI streamable-http MCP server (single {@code POST /mcp}
 * endpoint) in {@code frap.io.mode=file}, which lazily spawns the bundled native
 * {@code frap-core-rpc} binary. So this is the true transport + engine path, no mocks.</p>
 *
 * <p><b>Net-new transport handling (not a line-by-line copy of {@code StdioHandshakeIT}).</b>
 * Streamable-http differs from stdio in three ways this test must honour:</p>
 * <ul>
 *   <li>Requests are HTTP POSTs via {@link HttpClient} with
 *       {@code Accept: application/json, text/event-stream}.</li>
 *   <li>{@code initialize} may return an {@code Mcp-Session-Id} header; once seen it is
 *       echoed back on every subsequent request (initialized notification, tools/list,
 *       tools/call).</li>
 *   <li>The response body may be a single SSE stream ({@code event: message} /
 *       {@code data: {json}} lines) or a plain JSON body — {@link #parseBody} handles
 *       both by Content-Type, concatenating {@code data:} lines for the SSE case.</li>
 * </ul>
 *
 * <p>File-mode tools take/return absolute file paths, so the {@code sample-snapshot.json}
 * fixture is materialised on disk first and its absolute path is passed to the tool.</p>
 */
class HttpLocalHandshakeIT {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private static final Duration BOOT_TIMEOUT = Duration.ofSeconds(60);
    private static final Duration REQUEST_TIMEOUT = Duration.ofSeconds(30);
    private static final String PROTOCOL_VERSION = "2024-11-05";

    private static final List<String> EXPECTED_TOOLS = List.of(
        "frap_snapshot_script",
        "frap_help",
        "frap_build_element_map",
        "frap_filter_element_map",
        "frap_generate_page_object",
        "frap_heal");

    private static Process process;
    private static HttpClient http;
    private static URI endpoint;

    /** Absolute path to the snapshot fixture materialised on disk (file-mode tools read a path). */
    private static Path snapshotPath;

    /** Set from the initialize response header; echoed on all following requests if present. */
    private static volatile String sessionId;

    /** JSON-RPC id sequence shared across the handshake. */
    private static final AtomicInteger ID = new AtomicInteger(1);

    @BeforeAll
    static void startServer() throws Exception {
        snapshotPath = Files.createTempDirectory("frap-http-it")
            .resolve("sample-snapshot.json")
            .toAbsolutePath();
        try (InputStream in = HttpLocalHandshakeIT.class.getResourceAsStream("/sample-snapshot.json")) {
            assertThat(in).as("sample-snapshot.json must be on the test classpath").isNotNull();
            Files.copy(in, snapshotPath);
        }
        assertThat(Files.size(snapshotPath))
            .as("materialised snapshot fixture must be non-empty")
            .isPositive();

        int port = freePort();
        endpoint = URI.create("http://localhost:" + port + "/mcp");

        File jar = resolveJar();
        String java = System.getProperty("java.home") + File.separator + "bin" + File.separator + "java";

        // Discard the child's stdout (Spring banner/INFO logs): inheriting it into the
        // failsafe fork's native stdout triggers spurious "Corrupted channel" dumps. Keep
        // stderr for diagnosing real boot failures.
        process = new ProcessBuilder(
                java, "-jar", jar.getAbsolutePath(), "--server.port=" + port)
            .redirectOutput(ProcessBuilder.Redirect.DISCARD)
            .redirectError(ProcessBuilder.Redirect.INHERIT)
            .start();

        http = HttpClient.newBuilder()
            .connectTimeout(Duration.ofSeconds(5))
            .build();

        awaitReady();
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
    void initialize_then_toolsList_exposes_file_mode_signatures() throws Exception {
        JsonNode listResponse = request("tools/list", MAPPER.createObjectNode());
        JsonNode toolsNode = listResponse.path("result").path("tools");
        assertThat(toolsNode.isArray()).as("tools/list result must be an array: %s", listResponse).isTrue();

        List<String> toolNames = new ArrayList<>();
        toolsNode.forEach(t -> toolNames.add(t.path("name").asText()));
        assertThat(toolNames)
            .as("http-local server must expose the full file-mode tool set")
            .containsExactlyInAnyOrderElementsOf(EXPECTED_TOOLS);

        JsonNode build = null;
        for (JsonNode tool : toolsNode) {
            if ("frap_build_element_map".equals(tool.path("name").asText())) {
                build = tool;
                break;
            }
        }
        assertThat(build).as("frap_build_element_map must be present in tools/list").isNotNull();

        JsonNode props = build.path("inputSchema").path("properties");
        assertThat(props.isObject())
            .as("frap_build_element_map inputSchema.properties must be an object: %s", build)
            .isTrue();

        // FILE-MODE signature: a path parameter (domSnapshotPath), NOT an inline domSnapshot object.
        assertThat(props.has("domSnapshotPath"))
            .as("file-mode frap_build_element_map must expose a path parameter 'domSnapshotPath': %s", props)
            .isTrue();
        assertThat(props.has("domSnapshot"))
            .as("file-mode frap_build_element_map must NOT expose an inline 'domSnapshot' object: %s", props)
            .isFalse();
    }

    @Test
    void buildElementMap_over_http_returns_path_not_inline() throws Exception {
        JsonNode result = callTool("frap_build_element_map", args -> {
            args.put("domSnapshotPath", snapshotPath.toString());
        });

        assertThat(result.path("isError").asBoolean(false))
            .as("frap_build_element_map must not be an error result: %s", result)
            .isFalse();

        JsonNode payload = structuredOrText(result);

        String elementMapPath = payload.path("element_map_path").asText(null);
        assertThat(elementMapPath)
            .as("file-mode build result must carry an element_map_path: %s", payload)
            .isNotBlank();
        assertThat(Files.exists(Path.of(elementMapPath)))
            .as("the returned element-map file must exist on disk: %s", elementMapPath)
            .isTrue();

        // The whole point of file-mode: no inline elements array round-trips through the wire.
        assertThat(payload.has("elements"))
            .as("file-mode build result must NOT inline an 'elements' array: %s", payload)
            .isFalse();
        assertThat(payload.path("summary").path("element_count").asInt(0))
            .as("summary must report a positive element count from the real binary: %s", payload)
            .isGreaterThan(0);
    }

    @Test
    void roundtrip_build_then_generate_pageobject_writes_java_files() throws Exception {
        JsonNode buildResult = callTool("frap_build_element_map", args ->
            args.put("domSnapshotPath", snapshotPath.toString()));
        assertThat(buildResult.path("isError").asBoolean(false))
            .as("build step must not error: %s", buildResult)
            .isFalse();

        String elementMapPath = structuredOrText(buildResult).path("element_map_path").asText(null);
        assertThat(elementMapPath)
            .as("build step must yield an element_map_path to feed into generate")
            .isNotBlank();

        JsonNode genResult = callTool("frap_generate_page_object", args -> {
            args.put("elementMapPath", elementMapPath);
            args.put("language", "java_playwright");
            args.put("className", "SampleHttpPage");
            args.put("packageName", "it.http.local");
        });
        assertThat(genResult.path("isError").asBoolean(false))
            .as("generate step must not error: %s", genResult)
            .isFalse();

        JsonNode payload = structuredOrText(genResult);
        JsonNode filePaths = payload.path("file_paths");
        assertThat(filePaths.isArray() && filePaths.size() > 0)
            .as("generate result must carry a non-empty file_paths array: %s", payload)
            .isTrue();

        boolean javaSeen = false;
        for (JsonNode fp : filePaths) {
            Path path = Path.of(fp.asText());
            assertThat(Files.exists(path))
                .as("each generated file must exist on disk: %s", path)
                .isTrue();
            if (path.toString().endsWith(".java")) {
                javaSeen = true;
                assertThat(Files.size(path))
                    .as("generated .java file must be non-empty: %s", path)
                    .isPositive();
            }
        }
        assertThat(javaSeen)
            .as("real native generator must write at least one .java file: %s", payload)
            .isTrue();
    }

    // --- MCP JSON-RPC over streamable-http -------------------------------------------

    /**
     * Performs the {@code initialize} + {@code notifications/initialized} handshake, polling
     * until the server answers (boot) or the timeout elapses.
     */
    private static void awaitReady() throws Exception {
        long deadline = System.nanoTime() + BOOT_TIMEOUT.toNanos();
        Exception last = null;
        while (System.nanoTime() < deadline) {
            if (!process.isAlive()) {
                throw new IllegalStateException("http-local server exited during boot");
            }
            try {
                ObjectNode initParams = MAPPER.createObjectNode();
                initParams.put("protocolVersion", PROTOCOL_VERSION);
                initParams.set("capabilities", MAPPER.createObjectNode());
                ObjectNode clientInfo = MAPPER.createObjectNode();
                clientInfo.put("name", "frap-mcp-http-local-it");
                clientInfo.put("version", "1.0.0");
                initParams.set("clientInfo", clientInfo);

                JsonNode initResponse = request("initialize", initParams);
                if (!initResponse.hasNonNull("error")
                    && initResponse.path("result").path("serverInfo").path("name").isTextual()) {
                    notify("notifications/initialized", MAPPER.createObjectNode());
                    return;
                }
                last = new IllegalStateException("unexpected initialize response: " + initResponse);
            } catch (IOException | InterruptedException | IllegalStateException e) {
                last = e instanceof Exception ex ? ex : new IllegalStateException(e);
                if (e instanceof InterruptedException) {
                    Thread.currentThread().interrupt();
                }
            }
            Thread.sleep(250);
        }
        throw new IllegalStateException("http-local server did not become ready within " + BOOT_TIMEOUT, last);
    }

    /** Sends a JSON-RPC request (with an id) and returns the matching response object. */
    private static JsonNode request(String method, JsonNode params) throws Exception {
        ObjectNode req = MAPPER.createObjectNode();
        req.put("jsonrpc", "2.0");
        req.put("id", ID.getAndIncrement());
        req.put("method", method);
        req.set("params", params);

        HttpResponse<String> resp = post(MAPPER.writeValueAsString(req));
        captureSession(resp);

        int status = resp.statusCode();
        if (status >= 400) {
            throw new IllegalStateException("HTTP " + status + " for " + method + ": " + resp.body());
        }
        JsonNode body = parseBody(resp);
        if (body == null || body.isMissingNode()) {
            throw new IllegalStateException("empty body for " + method + " (HTTP " + status + ")");
        }
        return body;
    }

    /** Sends a JSON-RPC notification (no id, no response expected). */
    private static void notify(String method, JsonNode params) throws Exception {
        ObjectNode req = MAPPER.createObjectNode();
        req.put("jsonrpc", "2.0");
        req.put("method", method);
        req.set("params", params);
        HttpResponse<String> resp = post(MAPPER.writeValueAsString(req));
        captureSession(resp);
    }

    /** Invokes a tool via {@code tools/call} and returns its {@code result} node. */
    private JsonNode callTool(String name, java.util.function.Consumer<ObjectNode> argBuilder) throws Exception {
        ObjectNode arguments = MAPPER.createObjectNode();
        argBuilder.accept(arguments);
        ObjectNode params = MAPPER.createObjectNode();
        params.put("name", name);
        params.set("arguments", arguments);

        JsonNode response = request("tools/call", params);
        assertThat(response.hasNonNull("error"))
            .as("tools/call %s must not return a JSON-RPC error: %s", name, response)
            .isFalse();
        return response.path("result");
    }

    private static HttpResponse<String> post(String json) throws IOException, InterruptedException {
        HttpRequest.Builder builder = HttpRequest.newBuilder(endpoint)
            .timeout(REQUEST_TIMEOUT)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .POST(HttpRequest.BodyPublishers.ofString(json, StandardCharsets.UTF_8));
        String sid = sessionId;
        if (sid != null) {
            builder.header("Mcp-Session-Id", sid);
        }
        return http.send(builder.build(), HttpResponse.BodyHandlers.ofString(StandardCharsets.UTF_8));
    }

    private static void captureSession(HttpResponse<String> resp) {
        resp.headers().firstValue("Mcp-Session-Id").ifPresent(value -> {
            if (!value.isBlank()) {
                sessionId = value;
            }
        });
    }

    /**
     * Extracts a JSON-RPC message from the response body. Streamable-http may answer either
     * as a single SSE stream ({@code event:}/{@code data:} lines) or as a plain JSON body;
     * this concatenates {@code data:} payloads for the SSE case and otherwise parses the raw
     * body. Returns the first parseable JSON object found.
     */
    private static JsonNode parseBody(HttpResponse<String> resp) throws IOException {
        String body = resp.body();
        if (body == null || body.isBlank()) {
            return MAPPER.missingNode();
        }
        String contentType = resp.headers().firstValue("Content-Type").orElse("").toLowerCase();
        boolean sse = contentType.contains("text/event-stream") || body.contains("data:");
        if (sse) {
            StringBuilder data = new StringBuilder();
            for (String line : body.split("\\R")) {
                if (line.startsWith("data:")) {
                    String chunk = line.substring("data:".length());
                    if (chunk.startsWith(" ")) {
                        chunk = chunk.substring(1);
                    }
                    data.append(chunk);
                }
            }
            String payload = data.toString().trim();
            if (!payload.isEmpty()) {
                return MAPPER.readTree(payload);
            }
        }
        return MAPPER.readTree(body.trim());
    }

    /**
     * Returns the tool result payload: {@code structuredContent} when present, otherwise the
     * first text {@code content} block parsed as JSON. File-mode tools return small structured
     * objects ({@code element_map_path}, {@code file_paths}), exposed either way by the server.
     */
    private static JsonNode structuredOrText(JsonNode result) throws IOException {
        JsonNode structured = result.path("structuredContent");
        if (structured.isObject() && structured.size() > 0) {
            return structured;
        }
        JsonNode content = result.path("content");
        if (content.isArray()) {
            for (JsonNode block : content) {
                if ("text".equals(block.path("type").asText())) {
                    String text = block.path("text").asText("");
                    if (!text.isBlank()) {
                        String trimmed = text.trim();
                        if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
                            return MAPPER.readTree(trimmed);
                        }
                    }
                }
            }
        }
        throw new IllegalStateException("no structured/text JSON payload in tool result: " + result);
    }

    private static int freePort() throws IOException {
        try (ServerSocket socket = new ServerSocket(0)) {
            return socket.getLocalPort();
        }
    }

    private static File resolveJar() {
        File primary = new File("target/frap-mcp-http-local.jar");
        if (primary.isFile()) {
            return primary;
        }
        File fromModule = new File(System.getProperty("user.dir"), "target/frap-mcp-http-local.jar");
        if (fromModule.isFile()) {
            return fromModule;
        }
        throw new IllegalStateException(
            "frap-mcp-http-local fat jar not found at " + primary.getAbsolutePath()
                + " — run `mvn -pl frap-mcp-http-local package` (or the full `verify`) first. "
                + "The reactor builds frap-core-rpc into the jar during package.");
    }
}
