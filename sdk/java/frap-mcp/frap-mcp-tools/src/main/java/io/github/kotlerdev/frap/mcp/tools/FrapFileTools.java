package io.github.kotlerdev.frap.mcp.tools;

import io.github.kotlerdev.frap.core.dto.DOMSnapshot;
import io.github.kotlerdev.frap.core.dto.ElementMap;
import io.github.kotlerdev.frap.core.dto.FilterSpec;
import io.github.kotlerdev.frap.core.dto.GeneratedArtifact;
import io.github.kotlerdev.frap.core.dto.GeneratedFile;
import io.github.kotlerdev.frap.core.dto.GenerateOptions;
import io.github.kotlerdev.frap.core.dto.HealRequest;
import io.github.kotlerdev.frap.core.dto.HealResult;
import io.github.kotlerdev.frap.core.dto.MapOptions;
import io.github.kotlerdev.frap.core.dto.Signature;
import io.github.kotlerdev.frap.mcp.tools.io.ArtifactStore;
import io.github.kotlerdev.frap.mcp.tools.io.ElementMapFileResult;
import io.github.kotlerdev.frap.mcp.tools.io.GeneratedArtifactFileResult;

import java.util.List;

import org.springaicommunity.mcp.annotation.McpTool;
import org.springaicommunity.mcp.annotation.McpToolParam;
import org.springframework.boot.autoconfigure.condition.ConditionalOnProperty;
import org.springframework.stereotype.Component;

/**
 * File-based frap MCP tools for the local (stdio) transport.
 *
 * <p>Exposes the SAME four tool names as the inline {@link FrapTools}
 * ({@code frap_build_element_map}, {@code frap_filter_element_map},
 * {@code frap_generate_page_object}, {@code frap_heal}) so the two tool sets are
 * swappable per transport. The difference is purely I/O: instead of moving big
 * payloads (DOM snapshot, ElementMap, generated source) through the agent's context
 * inline, this variant exchanges <b>absolute file paths</b> on a shared local
 * filesystem and returns a compact digest, keeping the agent's context small.</p>
 *
 * <p>Each method reads its large input from a file via {@link ArtifactStore}, delegates
 * the business logic to {@link FrapToolService} (the single source of
 * {@code FrapCoreClient} call logic, which wraps checked I/O errors in an
 * {@link IllegalStateException}), then writes the large output back to a file and
 * returns its path plus a summary.</p>
 *
 * <p>This file tool set is gated on {@code frap.io.mode=file}, keeping it active only on
 * the stdio runner where server and agent share one filesystem.</p>
 *
 * <p><b>Tool pipeline (call in this order).</b> The three generation tools form a fixed
 * chain where each tool's <i>output path</i> is the next tool's <i>input path</i>:</p>
 * <pre>
 *   1. frap_snapshot_script        no input            -&gt; JS string (run it in the page)
 *      run JS in browser           JS string           -&gt; { html, elements } snapshot
 *      save snapshot to a JSON file on disk            -&gt; domSnapshotPath
 *   2. frap_build_element_map      domSnapshotPath     -&gt; { elementMapPath, summary }
 *     (2b. frap_filter_element_map elementMapPath      -&gt; { elementMapPath, summary })  [optional]
 *   3. frap_generate_page_object   elementMapPath      -&gt; written .java file paths
 * </pre>
 * <p>{@code frap_heal} is a separate maintenance tool, not part of this chain: it repairs
 * a single stale selector against a fresh snapshot file.</p>
 *
 * <p>STEP 1 ({@code frap_snapshot_script}) lives in {@link FrapSnapshotTool} (always-on,
 * transport-neutral), not here.</p>
 */
@Component
@ConditionalOnProperty(name = "frap.io.mode", havingValue = "file")
public class FrapFileTools {

    // --- tool descriptions ---

    private static final String BUILD_DESC =
        """
        NEW TO FRAP? If the frap workflow is not already in your context, call frap_help FIRST — it returns the full tool order and what to pass at each step.

        frap_build_element_map — STEP 2 of 3 · FILE MODE (path in, path+digest out)

        Builds a clustered ElementMap from a saved DOM snapshot file.

        WHEN
          After frap_snapshot_script, after you ran its JS in the page, and after you
          saved the returned object to a JSON file.

        INPUT
          • domSnapshotPath (string, required) — absolute path to that snapshot JSON file.
              - its content is the raw { "html": ..., "elements": [...] } from the snapshot JS
              - frap does NOT create this file; you write it yourself
              - example: /tmp/frap/snapshot-main.json
          • options (object, optional) — { url, include_non_interactive, max_elements }

        OUTPUT
          { element_map_path, summary }
          • element_map_path — absolute path to the written ElementMap JSON file
          • summary — compact digest so you never open the file, with keys:
              1. element_count
              2. cluster_count
              3. single_clusters
              4. list_clusters
              5. largest_list_size
              6. conf_avg
              7. locators_ge_080
              8. strategy_counts

        NEXT
          Pass element_map_path as 'elementMapPath' to STEP 3 = frap_generate_page_object
          (or first to frap_filter_element_map). Pass the path — do not open or edit the file.

        EXAMPLE
          call:
          {
            "domSnapshotPath": "/tmp/frap/snapshot-main.json",
            "options": { "url": "https://example.com/app/main" }
          }
          result:
          {
            "element_map_path": "/tmp/frap/element-map-3f2a.json",
            "summary": {
              "element_count": 57,
              "cluster_count": 7,
              "single_clusters": 5,
              "list_clusters": 2,
              "largest_list_size": 48,
              "conf_avg": 0.59,
              "locators_ge_080": 8,
              "strategy_counts": { "id": 8, "aria-label": 21, "structural": 22, "text": 6 }
            }
          }

        PIPELINE
          1. frap_snapshot_script  → save snapshot to a file
          2. frap_build_element_map        (you are here)
          3. frap_filter_element_map       (optional)
          4. frap_generate_page_object""";

    private static final String BUILD_DOMSNAPSHOTPATH_PARAM =
        """
        (string, required) The ABSOLUTE filesystem path to the JSON file you created in \
        STEP 1. That file's entire content must be the raw object returned by the \
        frap_snapshot_script JavaScript, i.e. { html: <string>, elements: [ ... ] }, \
        written as valid UTF-8 JSON with NO extra wrapper. Example: \
        /tmp/frap/snapshot-main.json. Do NOT pass the snapshot object itself here — only \
        its path; do NOT edit the file.""";

    private static final String BUILD_OPTIONS_PARAM =
        """
        (object, optional) Map options { url, include_non_interactive, max_elements }:
          • url — only labels the result metadata
          • include_non_interactive — true also keeps non-clickable elements
          • max_elements — caps how many elements are returned
        Omit this whole argument to use defaults.""";

    private static final String FILTER_DESC =
        """
        NEW TO FRAP? If the frap workflow is not already in your context, call frap_help FIRST — it returns the full tool order and what to pass at each step.

        frap_filter_element_map — OPTIONAL · FILE MODE (path in, smaller path+digest out)

        Shrinks an ElementMap file by keeping only the elements/clusters that match a
        filter. Runs BETWEEN STEP 2 and STEP 3.

        WHEN
          After frap_build_element_map, when you want to drop noise before STEP 3.

        INPUT
          • elementMapPath (string, required) — the ABSOLUTE path that frap_build_element_map
            returned in its 'element_map_path' output field.
              - example: /tmp/frap/element-map-3f2a.json
          • filter (object, required) — describes what to keep:
              { interactive_only, min_cluster_size, tags }

        OUTPUT
          { element_map_path, summary }
          • element_map_path — absolute path to a NEW, smaller ElementMap file with the SAME
            shape ({ elements, clusters, metadata })
          • summary — its compact digest (same keys as frap_build_element_map's summary)

        NEXT
          Pass the returned element_map_path as 'elementMapPath' to frap_generate_page_object.
          Skip this tool entirely if you want to keep every element.
          Pass the path string; do NOT open or edit the files.

        EXAMPLE
          call:
          {
            "elementMapPath": "/tmp/frap/element-map-3f2a.json",
            "filter": { "interactive_only": true, "min_cluster_size": 2, "tags": ["a", "button"] }
          }
          result:
          {
            "element_map_path": "/tmp/frap/element-map-filtered-9c1d.json",
            "summary": {
              "element_count": 31,
              "cluster_count": 4
            }
          }

        PIPELINE
          1. frap_snapshot_script  → save snapshot to a file
          2. frap_build_element_map
          3. frap_filter_element_map       (you are here, optional)
          4. frap_generate_page_object""";

    private static final String FILTER_ELEMENTMAPPATH_PARAM =
        """
        (string, required) The ABSOLUTE path to an ElementMap JSON file, exactly as \
        returned in the 'element_map_path' field of frap_build_element_map. Example: \
        /tmp/frap/element-map-3f2a.json. Pass the path; do not edit the file.""";

    private static final String FILTER_FILTER_PARAM =
        """
        (object, required) Filter spec { interactive_only, min_cluster_size, tags }:
          • interactive_only=true — keeps only clickable/typeable elements (buttons, links, inputs)
          • min_cluster_size=N — drops clusters with fewer than N members
          • tags=[...] — keeps only those HTML tag names, e.g. ['a','button']
        Set only the fields you need.""";

    private static final String GENERATE_DESC =
        """
        NEW TO FRAP? If the frap workflow is not already in your context, call frap_help FIRST — it returns the full tool order and what to pass at each step.

        frap_generate_page_object — STEP 3 of 3 · FILE MODE (path in, written files out)

        Generates Page Object source files from an ElementMap file and writes them to disk.

        WHEN
          After frap_build_element_map (or after the optional frap_filter_element_map).

        INPUT
          • elementMapPath (string, required) — the ABSOLUTE path that frap_build_element_map
            (or frap_filter_element_map) returned in its 'element_map_path' output field.
              - example: /tmp/frap/element-map-3f2a.json
          • language (string, required) — target output format / framework.
          • className (string, required) — class name for the generated Page Object.
          • packageName (string, required) — package / namespace for the generated class.

        OUTPUT
          { file_paths, file_count, work_dir }
          • file_paths — list of ABSOLUTE paths to the Page Object source files that frap
            HAS ALREADY WRITTEN to disk under work_dir
          • file_count — how many files were written
          • work_dir — the base directory they were written under

        NEXT
          Nothing more to write — the files already exist at file_paths; just open/use them.
          The pipeline is finished after this. Pass the input path string; do NOT edit the
          input map file.

        EXAMPLE
          call:
          {
            "elementMapPath": "/tmp/frap/element-map-3f2a.json",
            "language": "java_playwright",
            "className": "MainPage",
            "packageName": "com.example.pages"
          }
          result:
          {
            "file_paths": ["/tmp/frap/com/example/pages/MainPage.java"],
            "file_count": 1,
            "work_dir": "/tmp/frap"
          }

        PIPELINE
          1. frap_snapshot_script  → save snapshot to a file
          2. frap_build_element_map
          3. frap_filter_element_map       (optional)
          4. frap_generate_page_object     (you are here)""";

    private static final String GENERATE_ELEMENTMAPPATH_PARAM =
        """
        (string, required) The ABSOLUTE path to the ElementMap JSON file, exactly as \
        returned in the 'element_map_path' field of frap_build_element_map (or \
        frap_filter_element_map). Example: /tmp/frap/element-map-3f2a.json. Pass the path; \
        do not edit the file.""";

    private static final String GENERATE_LANGUAGE_PARAM =
        """
        (string, required) Target output format / framework. Currently supported: \
        'java_playwright'. Example: java_playwright.""";

    private static final String GENERATE_CLASSNAME_PARAM =
        """
        (string, required) The class name for the generated Page Object. Example: \
        PaymentsPage.""";

    private static final String GENERATE_PACKAGENAME_PARAM =
        """
        (string, required) The package / namespace for the generated class. Example: \
        com.example.pages.""";

    private static final String HEAL_DESC =
        """
        NEW TO FRAP? If the frap workflow is not already in your context, call frap_help FIRST — it returns the full tool order and what to pass at each step.

        frap_heal — STANDALONE · FILE MODE (broken selector + fresh snapshot file in, repaired selector out)

        Repairs a single stale selector against a fresh snapshot file. NOT part of the
        3-step generation pipeline. The result is small and returned inline (no file written).

        WHEN
          A selector you ALREADY have has STOPPED matching because the page changed.

        INPUT
          • domSnapshotPath (string, required) — the ABSOLUTE path to a file containing a
            FRESH { html, elements } snapshot, obtained by running frap_snapshot_script in
            the page RIGHT NOW and saving its returned object to disk.
              - example: /tmp/frap/snapshot-fresh.json
          • primarySelector (string, required) — your old/broken selector string.
          • originalSignature (object, optional) — that element's structural fingerprint,
            copied from the element's 'signature' field in an ElementMap you built earlier.
          • minConfidence (number, optional) — a 0..1 threshold.

        OUTPUT
          A HealResult: { healed, selector, confidence, top_candidates, ... } (it may also
          include diff / original_signature / semantics).
          • If healed=true, 'selector' is the repaired locator.

        SAFETY
          If frap is unsure (several equally likely matches, or everything below
          minConfidence) it returns healed=false instead of guessing wrong — then
          re-snapshot or make the selector more specific.
          Pass the snapshot path; do NOT edit the file.

        EXAMPLE
          call:
          {
            "domSnapshotPath": "/tmp/frap/snapshot-fresh.json",
            "primarySelector": "#nav-link-main",
            "minConfidence": 0.85
          }
          result:
          {
            "healed": true,
            "selector": "#nav-link-main",
            "confidence": 0.91,
            "top_candidates": []
          }""";

    private static final String HEAL_DOMSNAPSHOTPATH_PARAM =
        """
        (string, required) The ABSOLUTE path to a file whose content is a FRESH \
        { html, elements } snapshot object produced by re-running frap_snapshot_script in \
        the page now and saving its return value to disk as raw JSON. Example: \
        /tmp/frap/snapshot-fresh.json. Pass the path; do not edit the file.""";

    private static final String HEAL_PRIMARYSELECTOR_PARAM =
        """
        (string, required) The old/broken selector string that no longer matches the \
        changed page.""";

    private static final String HEAL_ORIGINALSIGNATURE_PARAM =
        """
        (object, optional) The element's structural fingerprint (a Signature object) copied \
        from that element's 'signature' field in a previously built ElementMap. It helps \
        frap re-locate the element after the page changed. Omit if you do not have it.""";

    private static final String HEAL_MINCONFIDENCE_PARAM =
        """
        (number, optional) A confidence threshold from 0 to 1, below which frap refuses to \
        heal and returns healed=false. Omit to use the default.""";

    /** Single source of frap tool business logic (delegates to the native client). */
    private final FrapToolService service;

    /** Filesystem-backed read/write/summarize store for large artifacts. */
    private final ArtifactStore store;

    public FrapFileTools(final FrapToolService service, final ArtifactStore store) {
        this.service = service;
        this.store = store;
    }

    @McpTool(
        name = "frap_build_element_map",
        description = BUILD_DESC
    )
    public ElementMapFileResult frapBuildElementMap(
        @McpToolParam(
            description = BUILD_DOMSNAPSHOTPATH_PARAM,
            required = true
        ) String domSnapshotPath,
        @McpToolParam(
            description = BUILD_OPTIONS_PARAM,
            required = false
        ) MapOptions options
    ) {
        final DOMSnapshot snapshot = store.readJson(domSnapshotPath, DOMSnapshot.class);
        final ElementMap map = service.buildElementMap(snapshot, options);
        final String elementMapPath = store.writeJson("element-map", map);
        return new ElementMapFileResult(elementMapPath, store.summarize(map));
    }

    @McpTool(
        name = "frap_filter_element_map",
        description = FILTER_DESC
    )
    public ElementMapFileResult frapFilterElementMap(
        @McpToolParam(
            description = FILTER_ELEMENTMAPPATH_PARAM,
            required = true
        ) String elementMapPath,
        @McpToolParam(
            description = FILTER_FILTER_PARAM,
            required = true
        ) FilterSpec filter
    ) {
        final ElementMap map = store.readJson(elementMapPath, ElementMap.class);
        final ElementMap filtered = service.filterElementMap(map, filter);
        final String filteredPath = store.writeJson("element-map-filtered", filtered);
        return new ElementMapFileResult(filteredPath, store.summarize(filtered));
    }

    @McpTool(
        name = "frap_generate_page_object",
        description = GENERATE_DESC
    )
    public GeneratedArtifactFileResult frapGeneratePageObject(
        @McpToolParam(
            description = GENERATE_ELEMENTMAPPATH_PARAM,
            required = true
        ) String elementMapPath,
        @McpToolParam(
            description = GENERATE_LANGUAGE_PARAM,
            required = true
        ) String language,
        @McpToolParam(
            description = GENERATE_CLASSNAME_PARAM,
            required = true
        ) String className,
        @McpToolParam(
            description = GENERATE_PACKAGENAME_PARAM,
            required = true
        ) String packageName
    ) {
        final ElementMap map = store.readJson(elementMapPath, ElementMap.class);
        final GenerateOptions options = new GenerateOptions(language, className, packageName, true);
        final GeneratedArtifact artifact = service.generatePageObject(map, options);
        final List<String> filePaths = artifact.files().stream()
            .map(this::writeGeneratedFile)
            .toList();
        return new GeneratedArtifactFileResult(filePaths, filePaths.size(), store.workDir());
    }

    /** Writes one generated source file to disk and returns its absolute path. */
    private String writeGeneratedFile(final GeneratedFile file) {
        return store.writeText(file.path(), file.content());
    }

    @McpTool(
        name = "frap_heal",
        description = HEAL_DESC
    )
    public HealResult frapHeal(
        @McpToolParam(
            description = HEAL_DOMSNAPSHOTPATH_PARAM,
            required = true
        ) String domSnapshotPath,
        @McpToolParam(
            description = HEAL_PRIMARYSELECTOR_PARAM,
            required = true
        ) String primarySelector,
        @McpToolParam(
            description = HEAL_ORIGINALSIGNATURE_PARAM,
            required = false
        ) Signature originalSignature,
        @McpToolParam(
            description = HEAL_MINCONFIDENCE_PARAM,
            required = false
        ) Double minConfidence
    ) {
        final DOMSnapshot snapshot = store.readJson(domSnapshotPath, DOMSnapshot.class);
        final HealRequest request =
            new HealRequest(primarySelector, originalSignature, snapshot, minConfidence);
        return service.heal(request);
    }
}
