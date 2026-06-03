package io.github.kotlerdev.frap.mcp.tools;

import org.springaicommunity.mcp.annotation.McpTool;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.stereotype.Component;

/**
 * Always-on {@code frap_help} tool — a plain, beginner-friendly guide to the frap
 * Page Object pipeline.
 *
 * <p>This tool is not gated by {@code frap.io.mode}; it is active on every runner so an
 * unsure agent can always read how to drive frap for THIS server's mode.</p>
 */
@Component
public class FrapHelpTool {

    // --- tool descriptions ---

    private static final String HELP_DESC =
        """
        Read/call this FIRST if you are unsure how to use frap. Returns a plain, \
        step-by-step beginner guide: what frap is, the exact order of tool calls, and what \
        to pass at each step for THIS server's mode. No arguments.""";

    private static final String HELP_GUIDE_HEAD =
        """
        frap — turn a real web page into ready-to-use Page Object test code.

        You do it in 3 steps, ALWAYS in this order — like a recipe: do step 1, then step 2,
        then step 3.

        THIS SERVER IS RUNNING IN:\s""";

    private static final String HELP_GUIDE_REST =
        """
         MODE.

        MODES
          • INLINE mode (HTTP): tools take and return real OBJECTS (the data travels inside
            the messages).
          • FILE mode (local stdio): tools take and return FILE PATHS (strings like
            /tmp/frap/xxx.json); frap reads and writes the files for you so big data stays
            OUT of your context.

        THE 3 STEPS
          1. frap_snapshot_script  (MANDATORY, always first)
               • Call it with no arguments. It returns a piece of JavaScript as TEXT.
               • That JavaScript is the camera that photographs the page. frap CANNOT run
                 it for you — it has no browser. YOU must run it, on your side, using ANY
                 browser tool you have: playwright-cli, the Playwright MCP server, the
                 chrome-devtools MCP server, or a direct CDP debugging-port connection.
               • Open the page you care about in that browser tool, then execute the
                 JavaScript inside the page (for example Playwright: page.evaluate(theScript)).
               • The script runs in PAGE CONTEXT — it is plain page JavaScript, NOT Node.
                 There is no filesystem (no require, no fs) inside the page; do not expect it
                 to write files by itself.
               • INLINE mode: the script returns an object shaped { html, elements: [...] }.
                 Keep that object to pass to step 2.
               • FILE mode: the script ALSO runs in the page. When this server has a local
                 frap ingest endpoint configured, the script is an async function that POSTs
                 the snapshot to that local frap endpoint and returns ONLY
                 { snapshot_path: "<abs path>" }. frap writes the big { html, elements } JSON
                 to disk for you; you hand that snapshot_path straight to step 2 and the big
                 snapshot never enters your context.
               • How to run it in FILE mode, by client:
                 - chrome-devtools-mcp: evaluate_script with this function. You may also pass
                   its filePath parameter so the result is written to a file too — the cleanest
                   route, and it sidesteps CSP entirely.
                 - playwright-mcp: browser_evaluate with the script directly, OR browser_run_code
                   as  async (page) => await page.evaluate(<script>) .
                 - playwright-cli: page.evaluate(<script>).
               • FILE mode CSP fallback: if the page's connect-src blocks the fetch to
                 localhost, the script returns the raw { html, elements } object instead. SAVE
                 that object to a JSON file yourself (raw, no wrapper) — e.g. chrome-devtools
                 filePath or a shell write — and pass that file's absolute path as
                 domSnapshotPath in step 2. To allow the fetch under strict CSP you can launch
                 the debugged Chrome with --disable-web-security, or use a Playwright context
                 with bypassCSP. Note: localhost is a secure context, so an https page calling
                 http://localhost is NOT blocked as mixed content — only CSP connect-src can
                 stop it. If no ingest endpoint is configured at all, the script is simply the
                 plain page IIFE returning { html, elements }: save it yourself and pass the
                 path as domSnapshotPath.
          2. frap_build_element_map
               • INLINE mode: pass the snapshot OBJECT as 'domSnapshot'. FILE mode: pass the
                 snapshot FILE PATH as 'domSnapshotPath'.
               • It studies the photo and produces the element map: every button/link/field,
                 grouped into clusters, each with a recommended locator and a confidence
                 score 0..1.
               • INLINE mode: you get the map OBJECT back. FILE mode: you get
                 { element_map_path, summary } — a file path plus a tiny digest so you never
                 need to open the file.
          3. frap_generate_page_object
               • INLINE mode: pass the map OBJECT as 'elementMap'. FILE mode: pass the map
                 FILE PATH as 'elementMapPath'.
               • Also pass: language (e.g. java_playwright), className (e.g. PaymentsPage),
                 packageName (e.g. com.example.pages).
               • INLINE mode: you get { files: [ { path, content } ] } — write each content
                 to its path yourself. FILE mode: frap already wrote the files; you get
                 { file_paths, file_count, work_dir } — just use them.

        OPTIONAL (between step 2 and step 3) — frap_filter_element_map
          • Shrinks the map (keep only interactive elements, only big clusters, or only
            certain tags). Same input/output style as step 2 (object in inline mode, path
            in file mode).

        SEPARATE TOOL — frap_heal  (NOT part of the 3 steps)
          • Use it later to repair ONE selector that broke after the page changed. Give it
            your old selector plus a FRESH snapshot (object in inline mode, file path in
            file mode). If frap is unsure it returns healed=false instead of guessing.

        GOLDEN RULES
          1. Always start with frap_help (this) if unsure, then frap_snapshot_script — and
             run its JavaScript yourself in a browser tool.
          2. Feed step 1's result into step 2, then step 2's result into step 3.
          3. FILE mode: pass PATHS, never paste big JSON, and do not open or edit the files
             frap writes.
          4. INLINE mode: pass OBJECTS through unchanged.

        ORDER
          frap_snapshot_script
          → (run the JS in your browser tool)
          → frap_build_element_map
          → (optional frap_filter_element_map)
          → frap_generate_page_object

        WORKED EXAMPLE (FILE mode)
          1. frap_snapshot_script() → returns JS text. Run it in your browser tool; it
             returns { html, elements }. Save to /tmp/frap/snapshot-main.json.
          2. frap_build_element_map with:
             {
               "domSnapshotPath": "/tmp/frap/snapshot-main.json"
             }
             → result:
             {
               "element_map_path": "/tmp/frap/element-map-3f2a.json",
               "summary": {...}
             }
          3. frap_generate_page_object with:
             {
               "elementMapPath": "/tmp/frap/element-map-3f2a.json",
               "language": "java_playwright",
               "className": "MainPage",
               "packageName": "com.example.pages"
             }
             → result:
             {
               "file_paths": ["/tmp/frap/com/example/pages/MainPage.java"],
               "file_count": 1
             }""";

    /** Active I/O mode of this server ({@code inline} or {@code file}). */
    private final String ioMode;

    public FrapHelpTool(@Value("${frap.io.mode:inline}") final String ioMode) {
        this.ioMode = ioMode;
    }

    @McpTool(
        name = "frap_help",
        description = HELP_DESC
    )
    public String frapHelp() {
        boolean file = "file".equalsIgnoreCase(ioMode);
        return HELP_GUIDE_HEAD + (file ? "FILE" : "INLINE") + HELP_GUIDE_REST;
    }
}
