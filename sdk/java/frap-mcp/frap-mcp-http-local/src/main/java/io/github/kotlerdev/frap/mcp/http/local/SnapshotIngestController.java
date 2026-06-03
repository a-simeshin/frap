package io.github.kotlerdev.frap.mcp.http.local;

import io.github.kotlerdev.frap.mcp.tools.io.ArtifactStore;

import java.util.Map;

import org.springframework.http.MediaType;
import org.springframework.http.ResponseEntity;
import org.springframework.util.StringUtils;
import org.springframework.web.bind.annotation.CrossOrigin;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RequestBody;
import org.springframework.web.bind.annotation.RestController;

/**
 * HTTP ingest endpoint for browser-produced DOM snapshots.
 *
 * <p>Browser page contexts (playwright-mcp / chrome-devtools-mcp / playwright-cli)
 * have no filesystem access, so they cannot write the snapshot to a local file
 * themselves. To keep the (large) DOM snapshot out of the agent context, the page
 * POSTs its raw {@code {html, elements:[...]}} JSON here; the server persists it
 * verbatim under the work directory and returns only the absolute path. The agent
 * then passes that path to {@code frap_build_element_map(domSnapshotPath=...)}.</p>
 *
 * <p>The body is taken as a raw {@link String} and written byte-for-byte via
 * {@link ArtifactStore#writeRawJson(String, String)} — it is never deserialized
 * into a DTO, so no element is lost or reshaped in transit.</p>
 *
 * <p>CORS: the originating page (e.g. {@code sberbank.ru}) POSTs cross-origin with
 * {@code Content-Type: application/json}, which triggers a preflight {@code OPTIONS}.
 * {@code @CrossOrigin(origins = "*")} lets Spring MVC answer that preflight (200) and
 * permits the actual POST from any origin. This endpoint is intended to be bound to
 * localhost only.</p>
 */
@RestController
@CrossOrigin(origins = "*", allowedHeaders = "*")
public class SnapshotIngestController {

    /** Filename prefix for ingested snapshots: {@code snapshot-<uuid>.json}. */
    private static final String SNAPSHOT_KIND = "snapshot";

    private final ArtifactStore artifactStore;

    /**
     * @param artifactStore the shared file-mode artifact store (a {@code @Component})
     */
    public SnapshotIngestController(final ArtifactStore artifactStore) {
        this.artifactStore = artifactStore;
    }

    /**
     * Persists a raw DOM-snapshot JSON body to the work directory and returns its path.
     *
     * @param rawJson the verbatim snapshot JSON ({@code {html, elements:[...]}})
     * @return {@code 200} with {@code {"snapshot_path":"<absolute path>"}}, or
     *         {@code 400} if the body is not a non-empty JSON object
     */
    @PostMapping(
        path = "/frap/ingest",
        consumes = MediaType.APPLICATION_JSON_VALUE,
        produces = MediaType.APPLICATION_JSON_VALUE
    )
    public ResponseEntity<Map<String, String>> ingest(@RequestBody final String rawJson) {
        if (!looksLikeJsonObject(rawJson)) {
            return ResponseEntity.badRequest().body(Map.of("error", "request body must be a non-empty JSON object"));
        }
        final String snapshotPath = artifactStore.writeRawJson(SNAPSHOT_KIND, rawJson);
        return ResponseEntity.ok(Map.of("snapshot_path", snapshotPath));
    }

    /** Lightweight check that the body is a non-empty JSON object — no full parse. */
    private static boolean looksLikeJsonObject(final String body) {
        if (!StringUtils.hasText(body)) {
            return false;
        }
        final String trimmed = body.trim();
        return trimmed.startsWith("{") && trimmed.endsWith("}");
    }
}
