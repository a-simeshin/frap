package io.github.kotlerdev.frap.mcp.tools;

import io.github.kotlerdev.frap.core.client.FrapCoreClient;
import io.github.kotlerdev.frap.core.dto.DOMSnapshot;
import io.github.kotlerdev.frap.core.dto.ElementMap;
import io.github.kotlerdev.frap.core.dto.FilterSpec;
import io.github.kotlerdev.frap.core.dto.GeneratedArtifact;
import io.github.kotlerdev.frap.core.dto.GenerateOptions;
import io.github.kotlerdev.frap.core.dto.HealRequest;
import io.github.kotlerdev.frap.core.dto.HealResult;
import io.github.kotlerdev.frap.core.dto.MapOptions;

import java.io.IOException;

import org.springframework.stereotype.Component;

/**
 * Single source of frap tool business logic.
 *
 * <p>Thin wrapper over {@link FrapCoreClient}: every method delegates straight to the
 * native client and wraps the checked {@link IOException} in an {@link IllegalStateException}
 * so the MCP layer surfaces a clean error result instead of a checked exception leaking
 * through the framework. The error message style mirrors the original {@code FrapTools}
 * inline tools (e.g. {@code "frap frap_build_element_map failed: ..."}).</p>
 *
 * <p>Both the inline ({@code FrapTools}) and file ({@code FrapFileTools}) tool adapters
 * delegate here, so the {@link FrapCoreClient} call logic exists in exactly one place.</p>
 */
@Component
public class FrapToolService {

    /** The long-lived frap core client backed by the bundled native binary. */
    private final FrapCoreClient client;

    public FrapToolService(final FrapCoreClient client) {
        this.client = client;
    }

    /**
     * Builds an {@link ElementMap} (elements + clusters) from a DOM snapshot.
     *
     * @param snapshot the {@code { html, elements }} DOM snapshot
     * @param options  optional map options ({@code url}, {@code include_non_interactive}, {@code max_elements})
     * @return the built element map
     */
    public ElementMap buildElementMap(final DOMSnapshot snapshot, final MapOptions options) {
        try {
            return client.buildElementMap(snapshot, options);
        } catch (final IOException e) {
            throw new IllegalStateException("frap frap_build_element_map failed: " + e.getMessage(), e);
        }
    }

    /**
     * Filters an {@link ElementMap} down to what matches the given spec.
     *
     * @param map  the element map to filter
     * @param spec the filter spec ({@code interactive_only}, {@code min_cluster_size}, {@code tags})
     * @return a smaller element map with the same shape
     */
    public ElementMap filterElementMap(final ElementMap map, final FilterSpec spec) {
        try {
            return client.filterElementMap(map, spec);
        } catch (final IOException e) {
            throw new IllegalStateException("frap frap_filter_element_map failed: " + e.getMessage(), e);
        }
    }

    /**
     * Generates Page Object source files from an {@link ElementMap}.
     *
     * @param map     the element map to generate from
     * @param options target language / class name / package name options
     * @return the generated artifact ({@code files:[{ path, content }]})
     */
    public GeneratedArtifact generatePageObject(final ElementMap map, final GenerateOptions options) {
        try {
            return client.generatePageObject(map, options);
        } catch (final IOException e) {
            throw new IllegalStateException("frap frap_generate_page_object failed: " + e.getMessage(), e);
        }
    }

    /**
     * Repairs a single stale selector against a fresh DOM snapshot.
     *
     * @param request the heal request (selector, signature, snapshot, min confidence)
     * @return the heal result with confidence and candidate selectors
     */
    public HealResult heal(final HealRequest request) {
        try {
            return client.heal(request);
        } catch (final IOException e) {
            throw new IllegalStateException("frap frap_heal failed: " + e.getMessage(), e);
        }
    }
}
