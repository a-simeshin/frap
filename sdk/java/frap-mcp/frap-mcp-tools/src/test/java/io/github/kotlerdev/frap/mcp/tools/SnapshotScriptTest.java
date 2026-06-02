package io.github.kotlerdev.frap.mcp.tools;

import static org.assertj.core.api.Assertions.assertThat;

import org.junit.jupiter.api.Test;

/**
 * Unit tests for {@link SnapshotScript}: it must load the bundled browser-side
 * snapshot JS from the classpath ({@code frap/snapshot.js}) and expose a non-empty
 * source that mentions the frap-contract keys {@code html} and {@code elements}.
 */
class SnapshotScriptTest {

    @Test
    void snapshotJsIsNonEmptyAndContainsContractKeys() {
        SnapshotScript snapshotScript = new SnapshotScript();

        String js = snapshotScript.snapshotJs();

        assertThat(js)
            .isNotNull()
            .isNotBlank()
            .contains("html")
            .contains("elements");
    }
}
