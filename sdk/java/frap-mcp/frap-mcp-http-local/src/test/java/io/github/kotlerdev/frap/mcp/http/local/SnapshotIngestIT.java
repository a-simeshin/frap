package io.github.kotlerdev.frap.mcp.http.local;

import static org.assertj.core.api.Assertions.assertThat;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.io.File;
import java.io.IOException;
import java.net.ServerSocket;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

/**
 * End-to-end integration test of the snapshot ingest endpoint
 * ({@code POST /frap/ingest}) against the REAL built {@code frap-mcp-http-local}
 * fat jar, plus a cross-check that an ingested snapshot is consumable by the MCP
 * pipeline ({@code frap_build_element_map}).
 *
 * <p>Spawns {@code java -jar frap-mcp-http-local.jar --server.port=<free>
 * --frap.io.work-dir=<temp>} as a subprocess. The deterministic {@code work-dir}
 * lets the test know exactly where ingested files land and isolates them from any
 * shared tmp state. The jar boots the full Spring MVC server hosting both the
 * streamable-http MCP endpoint ({@code POST /mcp}) and the
 * {@link SnapshotIngestController} ({@code POST /frap/ingest}); no mocks.</p>
 *
 * <p>The ingest contract: the page POSTs raw {@code {html, elements:[...]}} JSON;
 * the server persists it <b>byte-for-byte</b> under work-dir and returns only
 * {@code {"snapshot_path":"<absolute>"}}. The agent then feeds that path into
 * {@code frap_build_element_map(domSnapshotPath=...)} — exercised here over genuine
 * MCP JSON-RPC (session header + SSE/JSON body parsing) so the ingested file is
 * proven to be real pipeline input, not just a blob on disk.</p>
 */
class SnapshotIngestIT {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private static final Duration BOOT_TIMEOUT = Duration.ofSeconds(60);
    private static final Duration REQUEST_TIMEOUT = Duration.ofSeconds(30);
    private static final String PROTOCOL_VERSION = "2024-11-05";

    /**
     * The verbatim DOM-snapshot JSON the page POSTs. The ingest contract is that the
     * server persists this byte-for-byte, so the assertion compares the on-disk bytes
     * to exactly these bytes. Shaped like a real {@code {html, elements:[...]}} snapshot
     * so {@code frap_build_element_map} can consume it.
     */
    private static final String SNAPSHOT_JSON =
        "{\"html\":\"<html></html>\",\"elements\":[{\"selector\":\"a.x\",\"tag\":\"a\","
            + "\"attributes\":{},\"text_content\":\"A\",\"path\":[\"a:-\"],\"position_in_parent\":0}]}";

    private static Process process;
    private static HttpClient http;

    /** Streamable-http MCP endpoint ({@code POST /mcp}) of the spawned jar. */
    private static URI mcpEndpoint;

    /** Ingest endpoint ({@code POST /frap/ingest}) of the spawned jar. */
    private static URI ingestEndpoint;

    /** Deterministic work-dir passed to the jar; ingested files must land under here. */
    private static Path workDir;

    /** Set from the initialize response header; echoed on all following MCP requests if present. */
    private static volatile String sessionId;

    /** JSON-RPC id sequence shared across the handshake. */
    private static final AtomicInteger ID = new AtomicInteger(1);

    @BeforeAll
    static void startServer() throws Exception {
        workDir = Files.createTempDirectory("frap-ingest-it").toAbsolutePath();

        int port = freePort();
        mcpEndpoint = URI.create("http://127.0.0.1:" + port + "/mcp");
        ingestEndpoint = URI.create("http://127.0.0.1:" + port + "/frap/ingest");

        File jar = resolveJar();
        String java = System.getProperty("java.home") + File.separator + "bin" + File.separator + "java";

        // Discard the child's stdout (Spring banner/INFO logs); keep stderr for boot failures.
        process = new ProcessBuilder(
                java,
                "-jar", jar.getAbsolutePath(),
                "--server.port=" + port,
                "--frap.io.work-dir=" + workDir)
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
    void ingest_endpoint_writes_snapshot_file_and_returns_path() throws Exception {
        HttpResponse<String> resp = postIngest(SNAPSHOT_JSON);

        assertThat(resp.statusCode())
            .as("ingest must answer 200 for a valid JSON object body: %s", resp.body())
            .isEqualTo(200);

        JsonNode body = MAPPER.readTree(resp.body());
        String snapshotPath = body.path("snapshot_path").asText(null);
        assertThat(snapshotPath)
            .as("ingest response must carry a snapshot_path: %s", resp.body())
            .isNotBlank();

        Path persisted = Path.of(snapshotPath);
        assertThat(persisted.isAbsolute())
            .as("snapshot_path must be absolute: %s", snapshotPath)
            .isTrue();
        assertThat(persisted.startsWith(workDir))
            .as("snapshot must be persisted under the configured work-dir %s: %s", workDir, snapshotPath)
            .isTrue();
        assertThat(Files.exists(persisted))
            .as("the file at snapshot_path must exist on disk: %s", snapshotPath)
            .isTrue();

        // Verbatim contract: bytes on disk are byte-for-byte the bytes we POSTed.
        byte[] onDisk = Files.readAllBytes(persisted);
        assertThat(onDisk)
            .as("persisted snapshot must be byte-for-byte identical to the POSTed JSON")
            .isEqualTo(SNAPSHOT_JSON.getBytes(StandardCharsets.UTF_8));
    }

    @Test
    void ingest_path_is_consumable_by_build_element_map_over_mcp() throws Exception {
        // 1) Ingest a snapshot and grab the path the endpoint returns.
        HttpResponse<String> ingestResp = postIngest(SNAPSHOT_JSON);
        assertThat(ingestResp.statusCode())
            .as("ingest must answer 200: %s", ingestResp.body())
            .isEqualTo(200);
        String snapshotPath = MAPPER.readTree(ingestResp.body()).path("snapshot_path").asText(null);
        assertThat(snapshotPath)
            .as("ingest must yield a snapshot_path to feed into the pipeline: %s", ingestResp.body())
            .isNotBlank();

        // 2) Feed that exact path into frap_build_element_map over MCP — proving the
        //    ingested file is genuine pipeline input, not just a blob on disk.
        JsonNode result = callTool("frap_build_element_map", args ->
            args.put("domSnapshotPath", snapshotPath));

        assertThat(result.path("isError").asBoolean(false))
            .as("frap_build_element_map over an ingested snapshot must not error: %s", result)
            .isFalse();

        JsonNode payload = structuredOrText(result);
        String elementMapPath = payload.path("element_map_path").asText(null);
        assertThat(elementMapPath)
            .as("building from an ingested snapshot must produce an element_map_path: %s", payload)
            .isNotBlank();
        assertThat(Files.exists(Path.of(elementMapPath)))
            .as("the element-map built from the ingested snapshot must exist on disk: %s", elementMapPath)
            .isTrue();
    }

    // --- ingest HTTP ----------------------------------------------------------------

    private static HttpResponse<String> postIngest(String json) throws IOException, InterruptedException {
        HttpRequest request = HttpRequest.newBuilder(ingestEndpoint)
            .timeout(REQUEST_TIMEOUT)
            .header("Content-Type", "application/json")
            .POST(HttpRequest.BodyPublishers.ofString(json, StandardCharsets.UTF_8))
            .build();
        return http.send(request, HttpResponse.BodyHandlers.ofString(StandardCharsets.UTF_8));
    }

    // --- MCP JSON-RPC over streamable-http (mirrors HttpLocalHandshakeIT) -------------

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
                clientInfo.put("name", "frap-mcp-http-local-ingest-it");
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

        HttpResponse<String> resp = postMcp(MAPPER.writeValueAsString(req));
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
        HttpResponse<String> resp = postMcp(MAPPER.writeValueAsString(req));
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

    private static HttpResponse<String> postMcp(String json) throws IOException, InterruptedException {
        HttpRequest.Builder builder = HttpRequest.newBuilder(mcpEndpoint)
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
     * body.
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
     * first text {@code content} block parsed as JSON.
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
                + " — run `mvn -pl frap-mcp-http-local package` (or the full `verify`) first.");
    }
}
