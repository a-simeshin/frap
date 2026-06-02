package io.github.kotlerdev.frap.mcp.tools;

import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.charset.StandardCharsets;

import org.springframework.core.io.ClassPathResource;
import org.springframework.stereotype.Component;

/**
 * Loads and caches the browser-side DOM snapshot script (classpath resource
 * {@code frap/snapshot.js}).
 *
 * <p>The script is a self-executing JavaScript expression that, when run in a page
 * context (e.g. Playwright/CDP {@code page.evaluate}), returns a frap-contract DOM
 * snapshot of the shape {@code {html, elements:[...]}}. It is loaded once at
 * construction time and cached for the lifetime of the bean.</p>
 */
@Component
public class SnapshotScript {

    private static final String RESOURCE_PATH = "frap/snapshot.js";

    private final String script;

    public SnapshotScript() {
        this.script = load();
    }

    private static String load() {
        try {
            return new ClassPathResource(RESOURCE_PATH).getContentAsString(StandardCharsets.UTF_8);
        } catch (IOException e) {
            throw new UncheckedIOException("Failed to read snapshot script resource: " + RESOURCE_PATH, e);
        }
    }

    /**
     * Returns the cached snapshot JavaScript source.
     *
     * @return the self-executing snapshot script returning {@code {html, elements}}
     */
    public String snapshotJs() {
        return script;
    }
}
