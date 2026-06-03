package io.github.kotlerdev.frap.demo.snapshot;

import static org.assertj.core.api.Assertions.assertThat;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import com.microsoft.playwright.Browser;
import com.microsoft.playwright.BrowserType;
import com.microsoft.playwright.Page;
import com.microsoft.playwright.Playwright;
import com.sun.net.httpserver.HttpServer;
import java.io.BufferedReader;
import java.io.File;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.net.InetSocketAddress;
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
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Tag;
import org.junit.jupiter.api.Test;

/**
 * REAL browser, cross-client e2e for the {@code frap_snapshot_script} <b>file mode</b> script.
 *
 * <p>Goal: prove the page-context script returned by {@code frap_snapshot_script} (file mode) actually
 * executes in a browser with NO {@code require} errors, POSTs the snapshot to the local frap ingest
 * endpoint, and the server persists a file — the script returning {@code {snapshot_path}} (not the raw
 * {@code {html, elements}}) is the proof the {@code fetch} round-trip worked. Verified on three clients:</p>
 *
 * <ol>
 *   <li><b>playwright-cli / Playwright Java</b> — {@code page.evaluate(script)}.</li>
 *   <li><b>playwright-mcp</b> — spawned {@code @playwright/mcp} over stdio MCP JSON-RPC:
 *       {@code browser_navigate} + {@code browser_evaluate{function}}.</li>
 *   <li><b>chrome-devtools-mcp</b> — spawned {@code npx -y chrome-devtools-mcp@latest} over stdio MCP:
 *       {@code navigate_page{type:url}} + {@code evaluate_script{function}}.</li>
 * </ol>
 *
 * <p>Shared harness (set up once for all three clients):</p>
 * <ul>
 *   <li>Spawns the REAL built {@code frap-mcp-http-local.jar} on a free port {@code P} with a
 *       deterministic {@code --frap.io.work-dir=<tempdir>}. That gives both {@code /mcp} and
 *       {@code /frap/ingest}, and bakes {@code frap.io.ingest-url=http://127.0.0.1:P/frap/ingest}
 *       into the file-mode script.</li>
 *   <li>Fetches the REAL file-mode script {@code S} from the server via MCP
 *       ({@code tools/call frap_snapshot_script}) — never hard-coded; it carries the real ingest URL.</li>
 *   <li>Serves a tiny no-CSP test page from an in-process {@link HttpServer} on {@code 127.0.0.1}
 *       (localhost ingest is a secure context and the page sets no CSP, so the {@code fetch} succeeds).</li>
 * </ul>
 *
 * <p>Per-client asserts: the result carries a {@code snapshot_path} (NOT the raw {@code {html, elements}}
 * — proof the {@code fetch} hit ingest); that file exists under the work dir; and its contents are valid
 * JSON with {@code html} and an {@code elements} array. No {@code require} errors anywhere.</p>
 */
@Tag("e2e")
@DisplayName("frap_snapshot_script file mode — real browser, three clients")
class SnapshotScriptCrossClientE2eTest {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private static final Duration MCP_BOOT_TIMEOUT = Duration.ofSeconds(90);
    private static final Duration HTTP_REQUEST_TIMEOUT = Duration.ofSeconds(30);
    private static final String PROTOCOL_VERSION = "2024-11-05";

    /** Minimal no-CSP page with a few interactive nodes for the collector to pick up. */
    private static final String TEST_PAGE_HTML =
        """
        <!doctype html>
        <html lang="en">
          <head><meta charset="utf-8"><title>frap snapshot test page</title></head>
          <body>
            <nav>
              <a id="nav-home" data-testid="nav-home" href="/home">Home</a>
              <a id="nav-about" data-testid="nav-about" href="/about">About</a>
            </nav>
            <main>
              <button data-testid="primary-action" type="button">Do it</button>
              <input data-testid="search" type="text" placeholder="Search" />
            </main>
          </body>
        </html>
        """;

    // --- frap-mcp-http-local subprocess (gives /mcp + /frap/ingest) ---
    private static Process serverProcess;
    private static HttpClient http;
    private static URI mcpEndpoint;
    private static Path workDir;
    private static volatile String sessionId;
    private static final AtomicInteger MCP_ID = new AtomicInteger(1);

    /** The real file-mode page-context script (async () => {...}) returned by frap_snapshot_script. */
    private static String snapshotScript;

    // --- in-process no-CSP test page server ---
    private static HttpServer pageServer;
    private static String testPageUrl;

    @BeforeAll
    static void startHarness() throws Exception {
        // 0) Make sure Chromium is installed for Playwright Java (and reused by @playwright/mcp).
        installChromium();

        // 1) Spawn the REAL frap-mcp-http-local fat jar on a free port with a deterministic work dir.
        workDir = Files.createTempDirectory("frap-xclient-workdir").toAbsolutePath();
        int port = freePort();
        mcpEndpoint = URI.create("http://127.0.0.1:" + port + "/mcp");

        File jar = resolveHttpLocalJar();
        String java = System.getProperty("java.home") + File.separator + "bin" + File.separator + "java";
        serverProcess = new ProcessBuilder(
                java,
                "-jar", jar.getAbsolutePath(),
                "--server.port=" + port,
                "--frap.io.work-dir=" + workDir)
            .redirectOutput(ProcessBuilder.Redirect.DISCARD)
            .redirectError(ProcessBuilder.Redirect.INHERIT)
            .start();

        http = HttpClient.newBuilder().connectTimeout(Duration.ofSeconds(5)).build();
        awaitMcpReady();

        // 2) Pull the REAL file-mode script over MCP — it embeds the real ingest URL for THIS port.
        snapshotScript = fetchSnapshotScript();
        assertThat(snapshotScript)
            .as("file-mode script must be a page-context async function with the baked ingest URL")
            .startsWith("async ()")
            .contains("127.0.0.1:" + port + "/frap/ingest")
            .contains("fetch(")
            .doesNotContain("require(");

        // 3) Serve the no-CSP test page in-process on localhost.
        startPageServer();
    }

    @AfterAll
    static void stopHarness() {
        if (pageServer != null) {
            pageServer.stop(0);
        }
        if (serverProcess != null) {
            serverProcess.destroy();
            try {
                if (!serverProcess.waitFor(10, TimeUnit.SECONDS)) {
                    serverProcess.destroyForcibly();
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                serverProcess.destroyForcibly();
            }
        }
    }

    // ====================================================================================
    // Client 1: Playwright Java (== playwright-cli page.evaluate)
    // ====================================================================================

    @Test
    @DisplayName("playwright-cli (Playwright Java page.evaluate) runs the script; ingest writes a file")
    void playwrightCli_runs_script_and_ingest_writes_file() {
        try (Playwright pw = Playwright.create()) {
            Browser browser = pw.chromium().launch(new BrowserType.LaunchOptions().setHeadless(true));
            try {
                Page page = browser.newPage();
                page.navigate(testPageUrl);

                // page.evaluate accepts a function-expression string; the file-mode script is
                // exactly "async () => {...}", so this runs it in page context and awaits it.
                Object raw = page.evaluate(snapshotScript);
                JsonNode result = MAPPER.valueToTree(raw);

                assertSnapshotPathResult("playwright-cli", result);
            } finally {
                browser.close();
            }
        }
    }

    // ====================================================================================
    // Client 2: playwright-mcp (@playwright/mcp) over stdio MCP
    // ====================================================================================

    @Test
    @DisplayName("playwright-mcp (browser_evaluate) runs the script; ingest writes a file")
    void playwrightMcp_runs_script_and_ingest_writes_file() throws Exception {
        StdioMcpClient client = StdioMcpClient.spawn(
            npx(), "@playwright/mcp@0.0.45",
            "--headless", "--isolated",
            "--executable-path", bundledChromiumExecutable());
        try {
            client.initialize("frap-xclient-playwright-mcp");

            client.callTool("browser_navigate", args -> args.put("url", testPageUrl));

            JsonNode evalResult = client.callTool(
                "browser_evaluate", args -> args.put("function", snapshotScript));

            JsonNode payload = extractJsonFromToolResult("playwright-mcp", evalResult);
            assertSnapshotPathResult("playwright-mcp", payload);
        } finally {
            client.close();
        }
    }

    // ====================================================================================
    // Client 3: chrome-devtools-mcp over stdio MCP
    // ====================================================================================

    @Test
    @DisplayName("chrome-devtools-mcp (evaluate_script) runs the script; ingest writes a file")
    void chromeDevtoolsMcp_runs_script_and_ingest_writes_file() throws Exception {
        StdioMcpClient client = StdioMcpClient.spawn(
            npx(), "-y", "chrome-devtools-mcp@latest",
            "--headless", "--isolated",
            "--executablePath", bundledChromiumExecutable());
        try {
            client.initialize("frap-xclient-chrome-devtools-mcp");

            client.callTool("navigate_page", args -> {
                args.put("type", "url");
                args.put("url", testPageUrl);
            });

            JsonNode evalResult = client.callTool(
                "evaluate_script", args -> args.put("function", snapshotScript));

            JsonNode payload = extractJsonFromToolResult("chrome-devtools-mcp", evalResult);
            assertSnapshotPathResult("chrome-devtools-mcp", payload);
        } finally {
            client.close();
        }
    }

    // ====================================================================================
    // Shared assertions
    // ====================================================================================

    /**
     * Asserts the script's return value is the ingest's {@code {snapshot_path}} (not the raw
     * snapshot fallback), the file exists under the work dir, and its content is valid snapshot JSON.
     */
    private static void assertSnapshotPathResult(String client, JsonNode result) {
        assertThat(result)
            .as("[%s] script must return a JSON object", client)
            .isNotNull();
        assertThat(result.isObject())
            .as("[%s] script result must be an object, was: %s", client, result)
            .isTrue();

        // The whole point of file mode: fetch succeeded => server returned only the path,
        // NOT the raw {html, elements} fallback. So snapshot_path present AND no raw elements.
        String snapshotPath = result.path("snapshot_path").asText(null);
        assertThat(snapshotPath)
            .as("[%s] result must carry snapshot_path (proves fetch->ingest worked), was: %s",
                client, result)
            .isNotBlank();
        assertThat(result.has("elements"))
            .as("[%s] result must NOT be the raw {html,elements} fallback: %s", client, result)
            .isFalse();

        Path file = Path.of(snapshotPath);
        assertThat(Files.exists(file))
            .as("[%s] ingested snapshot file must exist on disk: %s", client, snapshotPath)
            .isTrue();
        assertThat(file.toAbsolutePath().startsWith(workDir))
            .as("[%s] ingested file %s must live under work dir %s", client, file, workDir)
            .isTrue();

        try {
            JsonNode snapshot = MAPPER.readTree(Files.readString(file));
            assertThat(snapshot.path("html").isTextual())
                .as("[%s] persisted snapshot must have a string 'html': %s", client, snapshot)
                .isTrue();
            assertThat(snapshot.path("elements").isArray())
                .as("[%s] persisted snapshot must have an 'elements' array: %s", client, snapshot)
                .isTrue();
            assertThat(snapshot.path("elements").size())
                .as("[%s] persisted snapshot 'elements' must be non-empty", client)
                .isGreaterThan(0);
        } catch (IOException e) {
            throw new IllegalStateException(
                "[" + client + "] failed to read persisted snapshot " + file, e);
        }
    }

    /**
     * Extracts the JSON payload the page-context script returned, from an MCP {@code tools/call}
     * result. Both servers wrap the evaluated value in a text content block; the actual value is
     * the first parseable JSON object containing {@code snapshot_path} (or a raw snapshot object).
     */
    private static JsonNode extractJsonFromToolResult(String client, JsonNode result) {
        assertThat(result.path("isError").asBoolean(false))
            .as("[%s] tools/call must not be an error result: %s", client, result)
            .isFalse();

        JsonNode content = result.path("content");
        assertThat(content.isArray())
            .as("[%s] tools/call result must have a content array: %s", client, result)
            .isTrue();

        for (JsonNode block : content) {
            String text = block.path("text").asText("");
            JsonNode parsed = tryFindSnapshotJson(text);
            if (parsed != null) {
                return parsed;
            }
        }
        throw new IllegalStateException(
            "[" + client + "] no snapshot JSON found in evaluate result: " + result);
    }

    /**
     * Scans a text block (which may include prose around the value) for the first JSON object that
     * looks like the script's return value: either {@code {snapshot_path}} or a raw {@code {html,...}}.
     */
    private static JsonNode tryFindSnapshotJson(String text) {
        if (text == null || text.isBlank()) {
            return null;
        }
        // Try whole-text parse first.
        JsonNode whole = parseIfMatches(text.trim());
        if (whole != null) {
            return whole;
        }
        // Otherwise scan for balanced { ... } substrings and try each.
        int len = text.length();
        for (int i = 0; i < len; i++) {
            if (text.charAt(i) != '{') {
                continue;
            }
            int depth = 0;
            boolean inStr = false;
            boolean esc = false;
            for (int j = i; j < len; j++) {
                char c = text.charAt(j);
                if (inStr) {
                    if (esc) {
                        esc = false;
                    } else if (c == '\\') {
                        esc = true;
                    } else if (c == '"') {
                        inStr = false;
                    }
                    continue;
                }
                if (c == '"') {
                    inStr = true;
                } else if (c == '{') {
                    depth++;
                } else if (c == '}') {
                    depth--;
                    if (depth == 0) {
                        JsonNode candidate = parseIfMatches(text.substring(i, j + 1));
                        if (candidate != null) {
                            return candidate;
                        }
                        break;
                    }
                }
            }
        }
        return null;
    }

    private static JsonNode parseIfMatches(String json) {
        if (!json.startsWith("{")) {
            return null;
        }
        try {
            JsonNode node = MAPPER.readTree(json);
            if (node.isObject() && (node.has("snapshot_path") || node.has("html") || node.has("elements"))) {
                return node;
            }
        } catch (IOException ignored) {
            // not JSON; keep scanning
        }
        return null;
    }

    // ====================================================================================
    // Harness helpers: MCP-over-HTTP to fetch the script, in-process page server, browser install
    // ====================================================================================

    private static String fetchSnapshotScript() throws Exception {
        ObjectNode params = MAPPER.createObjectNode();
        params.put("name", "frap_snapshot_script");
        params.set("arguments", MAPPER.createObjectNode());

        JsonNode response = mcpHttpRequest("tools/call", params);
        assertThat(response.hasNonNull("error"))
            .as("frap_snapshot_script tools/call must not error: %s", response)
            .isFalse();

        JsonNode content = response.path("result").path("content");
        assertThat(content.isArray() && content.size() > 0)
            .as("frap_snapshot_script must return a text content block: %s", response)
            .isTrue();
        String script = content.get(0).path("text").asText("");
        assertThat(script).as("snapshot script must be non-empty").isNotBlank();
        return script;
    }

    private static void awaitMcpReady() throws Exception {
        long deadline = System.nanoTime() + MCP_BOOT_TIMEOUT.toNanos();
        Exception last = null;
        while (System.nanoTime() < deadline) {
            if (!serverProcess.isAlive()) {
                throw new IllegalStateException("frap-mcp-http-local exited during boot");
            }
            try {
                ObjectNode initParams = MAPPER.createObjectNode();
                initParams.put("protocolVersion", PROTOCOL_VERSION);
                initParams.set("capabilities", MAPPER.createObjectNode());
                ObjectNode clientInfo = MAPPER.createObjectNode();
                clientInfo.put("name", "frap-xclient-harness");
                clientInfo.put("version", "1.0.0");
                initParams.set("clientInfo", clientInfo);

                JsonNode initResponse = mcpHttpRequest("initialize", initParams);
                if (!initResponse.hasNonNull("error")
                    && initResponse.path("result").path("serverInfo").path("name").isTextual()) {
                    mcpHttpNotify();
                    return;
                }
                last = new IllegalStateException("unexpected initialize response: " + initResponse);
            } catch (IOException | InterruptedException | IllegalStateException e) {
                last = e;
                if (e instanceof InterruptedException) {
                    Thread.currentThread().interrupt();
                }
            }
            Thread.sleep(250);
        }
        throw new IllegalStateException("frap-mcp-http-local not ready within " + MCP_BOOT_TIMEOUT, last);
    }

    private static JsonNode mcpHttpRequest(String method, JsonNode params) throws Exception {
        ObjectNode req = MAPPER.createObjectNode();
        req.put("jsonrpc", "2.0");
        req.put("id", MCP_ID.getAndIncrement());
        req.put("method", method);
        req.set("params", params);

        HttpRequest.Builder builder = HttpRequest.newBuilder(mcpEndpoint)
            .timeout(HTTP_REQUEST_TIMEOUT)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .POST(HttpRequest.BodyPublishers.ofString(MAPPER.writeValueAsString(req), StandardCharsets.UTF_8));
        if (sessionId != null) {
            builder.header("Mcp-Session-Id", sessionId);
        }
        HttpResponse<String> resp = http.send(builder.build(), HttpResponse.BodyHandlers.ofString(StandardCharsets.UTF_8));
        resp.headers().firstValue("Mcp-Session-Id").ifPresent(v -> {
            if (!v.isBlank()) {
                sessionId = v;
            }
        });
        if (resp.statusCode() >= 400) {
            throw new IllegalStateException("HTTP " + resp.statusCode() + " for " + method + ": " + resp.body());
        }
        return parseMcpHttpBody(resp);
    }

    private static void mcpHttpNotify() throws Exception {
        ObjectNode req = MAPPER.createObjectNode();
        req.put("jsonrpc", "2.0");
        req.put("method", "notifications/initialized");
        req.set("params", MAPPER.createObjectNode());

        HttpRequest.Builder builder = HttpRequest.newBuilder(mcpEndpoint)
            .timeout(HTTP_REQUEST_TIMEOUT)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .POST(HttpRequest.BodyPublishers.ofString(MAPPER.writeValueAsString(req), StandardCharsets.UTF_8));
        if (sessionId != null) {
            builder.header("Mcp-Session-Id", sessionId);
        }
        http.send(builder.build(), HttpResponse.BodyHandlers.ofString(StandardCharsets.UTF_8));
    }

    /** Streamable-http may answer as SSE (data: lines) or plain JSON; handle both. */
    private static JsonNode parseMcpHttpBody(HttpResponse<String> resp) throws IOException {
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

    private static void startPageServer() throws IOException {
        int port = freePort();
        pageServer = HttpServer.create(new InetSocketAddress("127.0.0.1", port), 0);
        byte[] html = TEST_PAGE_HTML.getBytes(StandardCharsets.UTF_8);
        pageServer.createContext("/", exchange -> {
            exchange.getResponseHeaders().add("Content-Type", "text/html; charset=utf-8");
            // No Content-Security-Policy header => the page may fetch() the localhost ingest.
            exchange.sendResponseHeaders(200, html.length);
            try (OutputStream os = exchange.getResponseBody()) {
                os.write(html);
            }
        });
        pageServer.start();
        testPageUrl = "http://127.0.0.1:" + port + "/";
    }

    private static void installChromium() throws IOException, InterruptedException {
        // Install Chromium for Playwright Java via its CLI. @playwright/mcp reuses the same
        // ms-playwright browser cache. CLI.main calls System.exit(), so it MUST run in a separate
        // JVM (running it in-process would tear down this test JVM). The driver bundled in the
        // Playwright jar is on the same classpath as this test, so reuse the current classpath.
        String java = System.getProperty("java.home") + File.separator + "bin" + File.separator + "java";
        String classpath = System.getProperty("java.class.path");
        Process p = new ProcessBuilder(
                java, "-cp", classpath, "com.microsoft.playwright.CLI", "install", "chromium")
            .redirectOutput(ProcessBuilder.Redirect.INHERIT)
            .redirectError(ProcessBuilder.Redirect.INHERIT)
            .start();
        boolean done = p.waitFor(5, TimeUnit.MINUTES);
        if (!done) {
            p.destroyForcibly();
            throw new IllegalStateException("Playwright Chromium install timed out");
        }
        if (p.exitValue() != 0) {
            throw new IllegalStateException("Playwright Chromium install failed, exit=" + p.exitValue());
        }
    }

    /**
     * Locates a real, locally-available Chromium executable from the Playwright browser cache
     * ({@code ms-playwright}) so the spawned MCP servers launch the SAME bundled Chromium that
     * Playwright Java installed — instead of defaulting to a system Google Chrome channel that
     * is not installed in this environment. Both MCP servers accept it via an executable-path flag.
     */
    private static String bundledChromiumExecutable() {
        String cacheRoot = System.getenv("PLAYWRIGHT_BROWSERS_PATH");
        Path base;
        if (cacheRoot != null && !cacheRoot.isBlank() && !"0".equals(cacheRoot)) {
            base = Path.of(cacheRoot);
        } else {
            String home = System.getProperty("user.home");
            String os = System.getProperty("os.name").toLowerCase();
            if (os.contains("mac")) {
                base = Path.of(home, "Library", "Caches", "ms-playwright");
            } else if (os.contains("win")) {
                base = Path.of(System.getenv("LOCALAPPDATA"), "ms-playwright");
            } else {
                base = Path.of(home, ".cache", "ms-playwright");
            }
        }
        if (!Files.isDirectory(base)) {
            throw new IllegalStateException(
                "Playwright browser cache not found at " + base
                    + " — Chromium install must run first (see @BeforeAll installChromium).");
        }

        // Relative executable layout inside a chromium-<rev> dir, by OS.
        String os = System.getProperty("os.name").toLowerCase();
        String[] relCandidates;
        if (os.contains("mac")) {
            relCandidates = new String[] {"chrome-mac/Chromium.app/Contents/MacOS/Chromium"};
        } else if (os.contains("win")) {
            relCandidates = new String[] {"chrome-win/chrome.exe"};
        } else {
            relCandidates = new String[] {"chrome-linux/chrome"};
        }

        Path best = null;
        long bestRev = -1;
        try (java.util.stream.Stream<Path> dirs = Files.list(base)) {
            for (Path dir : (Iterable<Path>) dirs::iterator) {
                String name = dir.getFileName().toString();
                if (!name.startsWith("chromium-") || name.contains("headless")) {
                    continue;
                }
                long rev;
                try {
                    rev = Long.parseLong(name.substring("chromium-".length()));
                } catch (NumberFormatException e) {
                    continue;
                }
                for (String rel : relCandidates) {
                    Path exe = dir.resolve(rel);
                    if (Files.isRegularFile(exe) && rev > bestRev) {
                        best = exe;
                        bestRev = rev;
                    }
                }
            }
        } catch (IOException e) {
            throw new IllegalStateException("failed to scan Playwright cache " + base, e);
        }
        if (best == null) {
            throw new IllegalStateException(
                "no Chromium executable found under " + base + " (looked for " + relCandidates[0] + ")");
        }
        return best.toAbsolutePath().toString();
    }

    /**
     * Resolves the {@code npx} launcher. Prefers the inherited PATH; falls back to common install
     * locations (Homebrew, nvm) so the surefire fork finds it even with a trimmed PATH.
     */
    private static String npx() {
        String override = System.getProperty("frap.npx");
        if (override != null && !override.isBlank() && new File(override).canExecute()) {
            return override;
        }
        for (String candidate : new String[] {
            "/opt/homebrew/bin/npx", "/usr/local/bin/npx", "/usr/bin/npx"
        }) {
            if (new File(candidate).canExecute()) {
                return candidate;
            }
        }
        return "npx"; // last resort: rely on PATH
    }

    private static int freePort() throws IOException {
        try (ServerSocket socket = new ServerSocket(0)) {
            return socket.getLocalPort();
        }
    }

    private static File resolveHttpLocalJar() {
        String prop = System.getProperty("frap.http.local.jar");
        if (prop != null && !prop.isBlank()) {
            File fromProp = new File(prop);
            if (fromProp.isFile()) {
                return fromProp;
            }
        }
        File fallback = new File(
            "../../../sdk/java/frap-mcp/frap-mcp-http-local/target/frap-mcp-http-local.jar");
        if (fallback.isFile()) {
            return fallback;
        }
        throw new IllegalStateException(
            "frap-mcp-http-local fat jar not found (system property frap.http.local.jar="
                + prop + ", fallback " + fallback.getAbsolutePath()
                + "). Build it with `mvn -pl frap-mcp-http-local package` in sdk/java first.");
    }

    // ====================================================================================
    // Minimal stdio MCP JSON-RPC client (newline-framed) for spawned MCP servers
    // ====================================================================================

    /** A tiny line-framed stdio MCP JSON-RPC 2.0 client for a spawned MCP server subprocess. */
    private static final class StdioMcpClient implements AutoCloseable {

        private final Process process;
        private final BufferedReader stdout;
        private final OutputStream stdin;
        private final AtomicInteger id = new AtomicInteger(1);
        private final Thread stderrPump;

        private StdioMcpClient(Process process) {
            this.process = process;
            this.stdout = new BufferedReader(
                new InputStreamReader(process.getInputStream(), StandardCharsets.UTF_8));
            this.stdin = process.getOutputStream();
            // Drain stderr so the child never blocks on a full pipe; surface it for diagnostics.
            this.stderrPump = new Thread(() -> {
                try (BufferedReader err = new BufferedReader(
                        new InputStreamReader(process.getErrorStream(), StandardCharsets.UTF_8))) {
                    String line;
                    while ((line = err.readLine()) != null) {
                        System.err.println("[mcp-stderr] " + line);
                    }
                } catch (IOException ignored) {
                    // process ended
                }
            }, "mcp-stderr-pump");
            this.stderrPump.setDaemon(true);
            this.stderrPump.start();
        }

        static StdioMcpClient spawn(String... command) throws IOException {
            Process p = new ProcessBuilder(command)
                .redirectErrorStream(false)
                .start();
            return new StdioMcpClient(p);
        }

        void initialize(String clientName) throws IOException {
            ObjectNode params = MAPPER.createObjectNode();
            params.put("protocolVersion", PROTOCOL_VERSION);
            params.set("capabilities", MAPPER.createObjectNode());
            ObjectNode clientInfo = MAPPER.createObjectNode();
            clientInfo.put("name", clientName);
            clientInfo.put("version", "1.0.0");
            params.set("clientInfo", clientInfo);

            JsonNode initResponse = rpc("initialize", params);
            if (initResponse.hasNonNull("error")) {
                throw new IllegalStateException("initialize failed: " + initResponse);
            }
            sendInitialized();
        }

        JsonNode callTool(String name, java.util.function.Consumer<ObjectNode> argBuilder)
            throws IOException {
            ObjectNode arguments = MAPPER.createObjectNode();
            argBuilder.accept(arguments);
            ObjectNode params = MAPPER.createObjectNode();
            params.put("name", name);
            params.set("arguments", arguments);

            JsonNode response = rpc("tools/call", params);
            if (response.hasNonNull("error")) {
                throw new IllegalStateException("tools/call " + name + " failed: " + response);
            }
            return response.path("result");
        }

        private void sendInitialized() throws IOException {
            ObjectNode req = MAPPER.createObjectNode();
            req.put("jsonrpc", "2.0");
            req.put("method", "notifications/initialized");
            req.set("params", MAPPER.createObjectNode());
            writeLine(MAPPER.writeValueAsString(req));
        }

        /** Sends a request and reads JSON lines until the matching id (or an error) is seen. */
        private JsonNode rpc(String method, JsonNode params) throws IOException {
            int reqId = id.getAndIncrement();
            ObjectNode req = MAPPER.createObjectNode();
            req.put("jsonrpc", "2.0");
            req.put("id", reqId);
            req.put("method", method);
            req.set("params", params);
            writeLine(MAPPER.writeValueAsString(req));

            // tools/call can take a while (browser launch / navigation); read with a deadline.
            long deadline = System.nanoTime() + Duration.ofSeconds(120).toNanos();
            while (System.nanoTime() < deadline) {
                if (!process.isAlive() && !stdout.ready()) {
                    throw new IllegalStateException(
                        "MCP server exited before responding to " + method);
                }
                String line = stdout.readLine();
                if (line == null) {
                    throw new IllegalStateException(
                        "MCP server closed stdout before responding to " + method);
                }
                if (line.isBlank()) {
                    continue;
                }
                JsonNode msg;
                try {
                    msg = MAPPER.readTree(line);
                } catch (IOException notJson) {
                    continue; // log noise on stdout; skip
                }
                if (msg.has("id") && msg.path("id").asInt(-1) == reqId) {
                    return msg;
                }
                // server-initiated notifications / other ids: ignore and keep reading
            }
            throw new IllegalStateException("timed out waiting for response to " + method);
        }

        private void writeLine(String json) throws IOException {
            stdin.write(json.getBytes(StandardCharsets.UTF_8));
            stdin.write('\n');
            stdin.flush();
        }

        @Override
        public void close() {
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
    }
}
