package io.github.kotlerdev.frap.mcp.tools;

import org.springaicommunity.mcp.annotation.McpTool;
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
        STEP 1 of 3 — ALWAYS CALL THIS FIRST when you are asked to build an \
        element map or generate a Page Object from a web page. THIS STEP IS MANDATORY and \
        it runs ON YOUR (the client) SIDE, not on the frap server — frap has no browser and \
        cannot reach your page. You must run the returned JavaScript yourself using ANY \
        browser-automation tool you have available and active — for example: playwright-cli, \
        the Playwright MCP server, the chrome-devtools MCP server, or even a direct \
        connection to the browser's CDP (Chrome DevTools Protocol) debugging port. Any one \
        of them works; pick whichever is available to you. WHAT THIS TOOL IS: it does \
        NOT open or touch any browser itself, and it does NOT fetch anything. It simply \
        RETURNS a piece of JavaScript SOURCE CODE as a plain text string. That returned \
        text IS the DOM-parsing script — there is nothing hidden and nothing to download \
        elsewhere: the exact script you must run is the string this tool gives back. \
        INPUT: none — call it with no arguments. OUTPUT: one value, a string, which is \
        JavaScript source code. HOW TO USE THE RETURNED STRING, step by step: THIS STEP IS \
        MANDATORY and it runs ON YOUR (the client) SIDE, not on the frap server — frap has \
        no browser and cannot reach your page. You must run the returned JavaScript yourself \
        using ANY browser-automation tool you have available and active — for example: \
        playwright-cli, the Playwright MCP server, the chrome-devtools MCP server, or even a \
        direct connection to the browser's CDP (Chrome DevTools Protocol) debugging port. \
        Any one of them works; pick whichever is available to you. (1) Make \
        sure the web page you want to map is open in YOUR OWN browser automation — \
        Playwright, Puppeteer, or Chrome DevTools Protocol (CDP). frap has no browser of \
        its own; you provide the page. (2) Execute the returned JavaScript INSIDE that \
        page and capture its return value. Examples — Playwright (Java): \
        `Object snapshot = page.evaluate(scriptStringReturnedByThisTool);` ; Playwright \
        (JS/TS): `const snapshot = await page.evaluate(scriptReturnedByThisTool);` ; CDP: \
        call `Runtime.evaluate` with `expression` = the returned script and \
        `returnByValue` = true, then read `result.value`. (3) The script is self-executing \
        and RETURNS a JSON object with EXACTLY this shape: { html: <string>, elements: \
        [ { selector, tag, attributes, text_content, path, position_in_parent }, ... ] }. \
        THAT returned object is the DOM snapshot. WHERE THE NEXT TOOL GETS ITS INPUT: this \
        snapshot object is the input to STEP 2 = frap_build_element_map. Which form you \
        pass depends on the transport: INLINE MODE (HTTP server) — pass the snapshot \
        object DIRECTLY as the 'domSnapshot' argument of frap_build_element_map. FILE MODE \
        (local stdio server) — write the snapshot object to a file on disk as RAW JSON \
        (the object exactly as returned, valid UTF-8 JSON, do NOT wrap it in any envelope \
        such as {result:...} or {data:...}), then pass that file's ABSOLUTE path (for \
        example /tmp/frap/snapshot-main.json) as the 'domSnapshotPath' argument of \
        frap_build_element_map. HOW TO TELL WHICH MODE YOU ARE IN: look at \
        frap_build_element_map's parameters — if it wants 'domSnapshot' (an object) you \
        are in inline mode; if it wants 'domSnapshotPath' (a string path) you are in file \
        mode. FULL PIPELINE ORDER: 1) frap_snapshot_script (this) -> run the returned JS \
        in the page -> 2) frap_build_element_map -> (optional) frap_filter_element_map -> \
        3) frap_generate_page_object. (frap_heal is a separate repair tool, not part of \
        this chain.) \
        EXAMPLE — call arguments: none. Example of what the returned JavaScript produces \
        when you run it in the page: { "html": "<html>...</html>", "elements": [ { \
        "selector": "a[id='nav-link-main']", "tag": "a", "attributes": { \
        "id": "nav-link-main" }, "text_content": "Main", "path": ["div:-", \
        "aside:-", "nav:-", "a:-"], "position_in_parent": 0 } ] }. INLINE mode: \
        pass that object to frap_build_element_map as domSnapshot. FILE mode: save it to \
        e.g. /tmp/frap/snapshot-main.json and pass that path as domSnapshotPath.""";

    /** Loader for the cached browser-side DOM snapshot script. */
    private final SnapshotScript snapshotScript;

    public FrapSnapshotTool(final SnapshotScript snapshotScript) {
        this.snapshotScript = snapshotScript;
    }

    @McpTool(
        name = "frap_snapshot_script",
        description = SNAPSHOT_DESC
    )
    public String frapSnapshotScript() {
        return snapshotScript.snapshotJs();
    }
}
