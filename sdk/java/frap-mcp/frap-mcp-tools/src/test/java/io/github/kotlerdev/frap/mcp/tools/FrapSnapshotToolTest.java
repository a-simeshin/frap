package io.github.kotlerdev.frap.mcp.tools;

import static org.assertj.core.api.Assertions.assertThat;

import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;

/**
 * Unit tests for {@link FrapSnapshotTool}: the mode-aware {@code frap_snapshot_script} tool
 * shared by both transports.
 *
 * <p>The tool is constructed manually with the desired {@code frap.io.mode} flag, the
 * {@code frap.io.ingest-url} value and a <b>real</b> {@link SnapshotScript} (loaded from the
 * {@code frap/snapshot.js} classpath resource), so the actual emitted script is asserted as
 * produced in production without spawning the native binary.</p>
 *
 * <p>Every branch returns a PAGE-CONTEXT script (no Node, no {@code require}, no {@code fs}):</p>
 * <ul>
 *   <li>inline → the original page-context IIFE verbatim;</li>
 *   <li>file + empty ingest URL → the same plain page-context IIFE;</li>
 *   <li>file + ingest URL → an async page-context function that POSTs the snapshot and falls back
 *       to returning the raw snapshot.</li>
 * </ul>
 */
@DisplayName("FrapSnapshotTool")
class FrapSnapshotToolTest {

    /** Ingest endpoint configured for the file-mode fetch branch. */
    private static final String INGEST_URL = "http://127.0.0.1:8765/frap/ingest";

    /** Real snapshot-script loader (reads {@code frap/snapshot.js} from the classpath). */
    private SnapshotScript snapshotScript;

    @BeforeEach
    void setUp() {
        snapshotScript = new SnapshotScript();
    }

    // --- inline mode --------------------------------------------------------

    @Test
    @DisplayName("inline mode returns the page-context IIFE unchanged (no fetch/Node machinery)")
    void inlineMode_returns_page_context_iife_unchanged() {
        // given — inline mode; ingest URL is irrelevant in this branch
        final FrapSnapshotTool tool = new FrapSnapshotTool(snapshotScript, "inline", INGEST_URL);

        // when
        final String script = tool.frapSnapshotScript();

        // then — it is the original page-context script verbatim
        assertThat(script).isEqualTo(snapshotScript.snapshotJs());
        assertThat(script).contains("document.querySelectorAll");
        // and carries none of the fetch / Node-context machinery
        assertThat(script)
            .doesNotContain("fetch(")
            .doesNotContain("require")
            .doesNotContain("fs");
    }

    // --- file mode, ingest URL present --------------------------------------

    @Test
    @DisplayName("file mode with ingest URL returns a page-context fetch script (POST + raw fallback)")
    void fileMode_with_ingest_url_returns_page_context_fetch_script() {
        // given — file mode with a configured local ingest endpoint
        final FrapSnapshotTool tool = new FrapSnapshotTool(snapshotScript, "file", INGEST_URL);

        // when
        final String script = tool.frapSnapshotScript();

        // then — it POSTs the snapshot to the injected URL
        assertThat(script).contains("fetch(");
        assertThat(script).contains(INGEST_URL);
        assertThat(script).contains("JSON.stringify(snapshot)");

        // the snapshot is sent VERBATIM — never re-wrapped as {elements: snapshot}
        assertThat(script).doesNotContain("elements: snapshot");

        // raw-snapshot fallback when the POST is blocked (CSP connect-src)
        assertThat(script).contains("return snapshot");

        // and it stays a PAGE-CONTEXT function: no Node / filesystem / outer evaluator
        assertThat(script)
            .doesNotContain("require")
            .doesNotContain("fs")
            .doesNotContain("writeFileSync")
            .doesNotContain("page.evaluate");
    }

    // --- file mode, no ingest URL -------------------------------------------

    @Test
    @DisplayName("file mode without ingest URL returns the plain page IIFE (agent saves it itself)")
    void fileMode_without_ingest_url_returns_plain_page_iife() {
        // given — file mode but no HTTP listener to POST to
        final FrapSnapshotTool tool = new FrapSnapshotTool(snapshotScript, "file", "");

        // when
        final String script = tool.frapSnapshotScript();

        // then — it falls back to the plain page-context IIFE
        assertThat(script).isEqualTo(snapshotScript.snapshotJs());
        assertThat(script)
            .doesNotContain("fetch(")
            .doesNotContain("require");
    }
}
