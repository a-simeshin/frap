package io.github.kotlerdev.frap.mcp.tools;

import org.springaicommunity.mcp.annotation.McpTool;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.stereotype.Component;

/**
 * Always-on, transport-neutral {@code frap_snapshot_script} tool.
 *
 * <p>This is STEP 1 of the Page Object pipeline and is shared by both transports
 * (inline / HTTP and file / local stdio). It hands the agent the browser-side DOM
 * snapshot script and does no server work itself; the snapshot it produces is the
 * input to {@code frap_build_element_map}.</p>
 *
 * <p>The tool is not gated by {@code frap.io.mode} — it is active on every runner so
 * exactly one snapshot tool exists regardless of transport.</p>
 */
@Component
public class FrapSnapshotTool {

    // --- tool descriptions ---

    private static final String SNAPSHOT_DESC =
        """
        NEW TO FRAP? If the frap workflow is not already in your context, call frap_help FIRST — it returns the full tool order and what to pass at each step.

        frap_snapshot_script — STEP 1 of 3 (returns JS source; you run it client-side)

        Returns a piece of JavaScript SOURCE CODE as a plain text string. That returned
        text IS the DOM-parsing script — there is nothing hidden and nothing to download
        elsewhere. This tool does NOT open or touch any browser itself, and does NOT fetch
        anything.

        WHEN
          ALWAYS CALL THIS FIRST when you are asked to build an element map or generate a
          Page Object from a web page. This step is MANDATORY and it runs ON YOUR (the
          client) SIDE, not on the frap server — frap has no browser and cannot reach your
          page. You run the returned JavaScript yourself using ANY browser-automation tool
          you have available and active, for example:
            • playwright-cli
            • the Playwright MCP server
            • the chrome-devtools MCP server
            • a direct connection to the browser's CDP (Chrome DevTools Protocol) debugging port
          Any one of them works; pick whichever is available to you.

        INPUT
          None — call it with no arguments.

        OUTPUT
          One value: a string, which is JavaScript source code.

        HOW TO USE THE RETURNED STRING
          1. Make sure the web page you want to map is open in YOUR OWN browser automation
             — Playwright, Puppeteer, or Chrome DevTools Protocol (CDP). frap has no browser
             of its own; you provide the page.
          2. Execute the returned JavaScript INSIDE that page and capture its return value:
               • Playwright (Java): Object snapshot = page.evaluate(scriptStringReturnedByThisTool);
               • Playwright (JS/TS): const snapshot = await page.evaluate(scriptReturnedByThisTool);
               • CDP: call Runtime.evaluate with expression = the returned script and
                 returnByValue = true, then read result.value.
          3. The script is self-executing and RETURNS a JSON object with EXACTLY this shape:
             { html: <string>, elements: [ { selector, tag, attributes, text_content, path,
             position_in_parent }, ... ] }. THAT returned object is the DOM snapshot.

        NEXT
          This snapshot is the input to STEP 2 = frap_build_element_map. Which form you pass
          depends on the transport:
            • INLINE MODE (HTTP server) — the returned script is a page-context IIFE. Run it
              with page.evaluate and pass the resulting snapshot object DIRECTLY as the
              'domSnapshot' argument of frap_build_element_map.
            • FILE MODE (local stdio server) — the returned script ALSO runs in PAGE CONTEXT
              (no filesystem access). When a local frap ingest endpoint is configured it is an
              async function that POSTs the snapshot to frap and returns ONLY
              { snapshot_path: "<abs path>" } — hand that path straight to
              frap_build_element_map as 'domSnapshotPath'; the big snapshot never enters your
              context. If the POST is blocked (CSP connect-src) the script instead returns the
              raw { html, elements } object as a fallback — save THAT to a file yourself as RAW
              JSON (no wrapper) and pass its absolute path as 'domSnapshotPath'. If no ingest
              endpoint is configured the script is just the plain IIFE returning
              { html, elements } — save it to a file yourself and pass that path.
          How to tell which mode you are in: look at frap_build_element_map's parameters —
          if it wants 'domSnapshot' (an object) you are in inline mode; if it wants
          'domSnapshotPath' (a string path) you are in file mode.

        EXAMPLE
          call: none.
          What the returned JavaScript produces when you run it in the page:
          {
            "html": "<html>...</html>",
            "elements": [
              {
                "selector": "a[id='nav-link-main']",
                "tag": "a",
                "attributes": { "id": "nav-link-main" },
                "text_content": "Main",
                "path": ["div:-", "aside:-", "nav:-", "a:-"],
                "position_in_parent": 0
              }
            ]
          }
          • INLINE next: pass that object to frap_build_element_map as domSnapshot.
          • FILE next: save it to e.g. /tmp/frap/snapshot-main.json and pass that path as
            domSnapshotPath.

        PIPELINE
          1. frap_snapshot_script  → run the returned JS in the page (you are here)
          2. frap_build_element_map
          3. frap_filter_element_map       (optional)
          4. frap_generate_page_object
          (frap_heal is a separate repair tool, not part of this chain.)""";

    /** Loader for the cached browser-side DOM snapshot script. */
    private final SnapshotScript snapshotScript;

    /** Active I/O mode of this server ({@code inline} or {@code file}). */
    private final String ioMode;

    /**
     * Absolute URL of the local frap ingest endpoint the page-context script POSTs the
     * snapshot to in file mode. Empty when no HTTP listener is available (e.g. pure stdio).
     */
    private final String ingestUrl;

    public FrapSnapshotTool(
        final SnapshotScript snapshotScript,
        @Value("${frap.io.mode:inline}") final String ioMode,
        @Value("${frap.io.ingest-url:}") final String ingestUrl
    ) {
        this.snapshotScript = snapshotScript;
        this.ioMode = ioMode;
        this.ingestUrl = ingestUrl;
    }

    @McpTool(
        name = "frap_snapshot_script",
        description = SNAPSHOT_DESC
    )
    public String frapSnapshotScript() {
        if ("file".equalsIgnoreCase(ioMode)) {
            return fileModeScript();
        }
        // INLINE mode: hand back the original page-context IIFE unchanged.
        return snapshotScript.snapshotJs();
    }

    /**
     * Builds the file-mode snapshot script.
     *
     * <p>Both branches return a PAGE-CONTEXT script (no Node, no {@code require}, no {@code fs}),
     * so it runs identically under every client's page evaluator (playwright-mcp
     * {@code browser_evaluate}/{@code browser_run_code}, chrome-devtools-mcp
     * {@code evaluate_script}, playwright-cli {@code page.evaluate}).</p>
     *
     * <ul>
     *   <li><b>No ingest URL</b> (e.g. pure stdio with no HTTP listener): fall back to the plain
     *       page-context IIFE — the same script inline mode returns. The agent gets the raw
     *       {@code {html, elements:[...]}} object and saves it to a file itself.</li>
     *   <li><b>Ingest URL present</b>: an async page-context function runs the shared COLLECTOR
     *       (returning {@code {html, elements:[...]}} VERBATIM), POSTs it to the local frap ingest
     *       endpoint, and returns the server's {@code { snapshot_path }} so the big snapshot never
     *       enters the agent's context. On any fetch/CSP failure it returns the raw snapshot as a
     *       fallback for the agent to persist.</li>
     * </ul>
     *
     * @return the page-context snapshot script source
     */
    private String fileModeScript() {
        if (ingestUrl == null || ingestUrl.isBlank()) {
            // No HTTP listener to POST to: hand back the plain page-context IIFE.
            return snapshotScript.snapshotJs();
        }
        final String collector = collector();
        final String url = jsStringLiteral(ingestUrl);
        return """
            async () => {
              const snapshot = (%s)();
              try {
                const res = await fetch(%s, {
                  method: "POST",
                  headers: { "Content-Type": "application/json" },
                  body: JSON.stringify(snapshot)
                });
                if (res.ok) return await res.json();
              } catch (e) {}
              return snapshot;
            }""".formatted(collector, url);
    }

    /**
     * Returns the shared element collector as a NON-invoked function expression.
     *
     * <p>The cached snapshot script is a self-executing IIFE ({@code (() => {...})()}) whose
     * return value is {@code {html, elements:[...]}}. For {@code page.evaluate(<collector>)}
     * we need the function itself, so the trailing invocation {@code ()} is stripped, leaving
     * {@code (() => {...})}. Inline mode keeps the original IIFE untouched.</p>
     *
     * @return the collector function expression returning {@code {html, elements:[...]}}
     */
    private String collector() {
        final String iife = snapshotScript.snapshotJs().strip();
        if (iife.endsWith("()")) {
            return iife.substring(0, iife.length() - 2).strip();
        }
        return iife;
    }

    /**
     * Renders a value as a double-quoted JavaScript string literal.
     *
     * <p>Backslashes are normalised to forward slashes and double quotes are escaped, so the
     * value is safe to embed inside a double-quoted JS string (e.g. the ingest URL injected
     * into the file-mode fetch script).</p>
     *
     * @param value the raw value (e.g. an absolute ingest URL)
     * @return a valid double-quoted JS string literal
     */
    private static String jsStringLiteral(final String value) {
        final String normalised = value.replace('\\', '/').replace("\"", "\\\"");
        return "\"" + normalised + "\"";
    }
}
