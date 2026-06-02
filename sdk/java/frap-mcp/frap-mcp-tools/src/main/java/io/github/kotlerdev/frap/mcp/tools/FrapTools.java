package io.github.kotlerdev.frap.mcp.tools;

import io.github.kotlerdev.frap.core.dto.DOMSnapshot;
import io.github.kotlerdev.frap.core.dto.ElementMap;
import io.github.kotlerdev.frap.core.dto.FilterSpec;
import io.github.kotlerdev.frap.core.dto.GeneratedArtifact;
import io.github.kotlerdev.frap.core.dto.GenerateOptions;
import io.github.kotlerdev.frap.core.dto.HealRequest;
import io.github.kotlerdev.frap.core.dto.HealResult;
import io.github.kotlerdev.frap.core.dto.MapOptions;

import org.springaicommunity.mcp.annotation.McpTool;
import org.springaicommunity.mcp.annotation.McpToolParam;
import org.springframework.boot.autoconfigure.condition.ConditionalOnProperty;
import org.springframework.stereotype.Component;

/**
 * Transport-neutral frap MCP tools exposed to the agent CLI.
 *
 * <p>Each method is annotated with {@link McpTool}; Spring AI auto-registers them with
 * whichever MCP transport (stdio / streamable-http) the active runner enables, and
 * auto-generates a JSON schema for each typed-DTO parameter (Jackson binds the
 * snake_case JSON the agent sends). All parameters here are typed records from
 * {@code frap-core-java}; none required a String-JSON fallback (Spring AI 1.1's schema
 * generation handled the nested records, including {@link DOMSnapshot} and
 * {@link ElementMap}, cleanly).</p>
 *
 * <p>Each tool delegates to {@link FrapToolService}, which wraps checked I/O errors from
 * the underlying native client in an {@link IllegalStateException} so the MCP layer
 * surfaces a clean error result instead of a checked exception leaking through the
 * framework.</p>
 *
 * <p>This inline tool set is gated on {@code frap.io.mode=inline} (the default when the
 * property is absent), keeping it active on the HTTP runner where large payloads travel
 * inline in the JSON.</p>
 *
 * <p><b>Tool pipeline (call in this order).</b> The three generation tools form a
 * fixed chain where each tool's output is the next tool's input:</p>
 * <pre>
 *   1. frap_snapshot_script        no input            -&gt; JS string (run it in the page)
 *      run JS in browser           JS string           -&gt; { html, elements } snapshot
 *   2. frap_build_element_map      snapshot            -&gt; ElementMap (elements + clusters)
 *     (2b. frap_filter_element_map ElementMap          -&gt; smaller ElementMap)   [optional]
 *   3. frap_generate_page_object   ElementMap          -&gt; files [{ path, content }]
 * </pre>
 * <p>{@code frap_heal} is a separate maintenance tool, not part of this chain: it repairs
 * a single stale selector against a fresh snapshot.</p>
 *
 * <p>STEP 1 ({@code frap_snapshot_script}) lives in {@link FrapSnapshotTool} (always-on,
 * transport-neutral), not here.</p>
 */
@Component
@ConditionalOnProperty(name = "frap.io.mode", havingValue = "inline", matchIfMissing = true)
public class FrapTools {

    // --- tool descriptions ---

    private static final String BUILD_DESC =
        """
        STEP 2 of 3. Call this AFTER frap_snapshot_script AND after you have \
        executed that returned script inside the page. INLINE MODE: you pass the actual \
        snapshot OBJECT here (this HTTP server has no shared filesystem, so everything \
        travels as JSON). WHERE THE INPUT COMES FROM: 'domSnapshot' is the object that the \
        frap_snapshot_script JavaScript RETURNED when you ran it in the page via \
        Playwright/CDP — shape { html, elements:[...] }. Pass that exact object; do not \
        modify it. WHAT THIS TOOL DOES: it scans the snapshot, finds interactive elements \
        (buttons, links, inputs, etc.), groups repeating UI items (table rows, cards, \
        menu links, tiles) into 'clusters', and gives every element a recommended locator \
        plus a confidence score from 0 to 1 (higher = more stable). OUTPUT: the full \
        ElementMap object = { elements:[...], clusters:[...], metadata:{...} }; each \
        element carries a recommended_selector and a confidence. WHAT TO DO NEXT: pass the \
        WHOLE returned ElementMap object, unchanged, as the 'elementMap' argument of STEP \
        3 = frap_generate_page_object. OPTIONAL: before STEP 3 you may pass it to \
        frap_filter_element_map to drop noise. Do not hand-edit the ElementMap. FULL \
        ORDER: 1) frap_snapshot_script -> run JS in page -> 2) frap_build_element_map \
        (this) -> (optional) frap_filter_element_map -> 3) frap_generate_page_object. \
        EXAMPLE — call arguments: { "domSnapshot": { "html": "<html>...</html>", \
        "elements": [ { "selector": "a[id='nav-link-main']", "tag": "a", \
        "attributes": { "id": "nav-link-main", "href": "/app/main" }, \
        "text_content": "Main", "path": ["div:-", "aside:-", "nav:-", \
        "a:-"], "position_in_parent": 0 } ] }, "options": { "url": \
        "https://example.com/app/main" } }. Result: the full ElementMap object \
        { elements, clusters, metadata } — pass it straight into \
        frap_generate_page_object.""";

    private static final String BUILD_DOMSNAPSHOT_PARAM =
        """
        REQUIRED. The DOM snapshot OBJECT returned by running the \
        frap_snapshot_script JavaScript in your browser page. Shape: { html: <string>, \
        elements: [ { selector, tag, attributes, text_content, path, \
        position_in_parent }, ... ] }. Pass exactly what the script returned — do not \
        wrap it, trim it, or edit it.""";

    private static final String BUILD_OPTIONS_PARAM =
        """
        OPTIONAL. Map options { url, include_non_interactive, max_elements }. \
        'url' only labels the result metadata (e.g. https://example.com/app); \
        'include_non_interactive' = true also keeps non-clickable elements; \
        'max_elements' caps how many elements are returned. Omit this whole argument \
        to use defaults.""";

    private static final String GENERATE_DESC =
        """
        STEP 3 of 3 (final). Call this AFTER frap_build_element_map (or after the \
        optional frap_filter_element_map). INLINE MODE: you pass the ElementMap OBJECT in, \
        and you get source-code TEXT back. WHERE THE INPUT COMES FROM: 'elementMap' is the \
        exact object that frap_build_element_map (or frap_filter_element_map) returned in \
        STEP 2. You also choose 'language', 'className', and 'packageName'. OUTPUT: a \
        GeneratedArtifact = { files: [ { path, content }, ... ] }. Each entry is one \
        source file: 'path' is a suggested relative file path, 'content' is the FULL \
        source code as text. WHAT TO DO: write each file's 'content' to its 'path' on disk \
        yourself — this inline server returns the code but does not write files for you. \
        After that you are done. FULL ORDER: 1) frap_snapshot_script -> 2) \
        frap_build_element_map -> (optional) frap_filter_element_map -> 3) \
        frap_generate_page_object (this). \
        EXAMPLE — call arguments: { "elementMap": <the exact object returned by \
        frap_build_element_map>, "language": "java_playwright", "className": \
        "MainPage", "packageName": "com.example.pages" }. Result: { "files": \
        [ { "path": "com/example/pages/MainPage.java", "content": "package \
        com.example.pages; ..." } ] } — write each content to its path yourself.""";

    private static final String GENERATE_ELEMENTMAP_PARAM =
        """
        REQUIRED. The exact ElementMap object returned by \
        frap_build_element_map (or by frap_filter_element_map). Pass it through \
        unchanged — do not edit it.""";

    private static final String GENERATE_LANGUAGE_PARAM =
        """
        REQUIRED. Target output format / framework. Currently supported: \
        'java_playwright'. Example: java_playwright.""";

    private static final String GENERATE_CLASSNAME_PARAM =
        """
        REQUIRED. The class name for the generated Page Object. Example: \
        PaymentsPage.""";

    private static final String GENERATE_PACKAGENAME_PARAM =
        """
        REQUIRED. The package / namespace for the generated class. Example: \
        com.example.pages.""";

    private static final String FILTER_DESC =
        """
        OPTIONAL helper that runs BETWEEN STEP 2 and STEP 3. WHERE THE INPUT \
        COMES FROM: 'elementMap' is the object that frap_build_element_map returned; plus \
        a 'filter' describing what to keep. OUTPUT: a smaller ElementMap object with the \
        SAME shape ({ elements, clusters, metadata }), containing only the \
        elements/clusters that match the filter. WHAT TO DO NEXT: pass the returned \
        ElementMap object as the 'elementMap' argument of frap_generate_page_object. Skip \
        this tool entirely if you want to keep every element. \
        EXAMPLE — call arguments: { "elementMap": <object from frap_build_element_map>, \
        "filter": { "interactive_only": true, "min_cluster_size": 2, "tags": \
        ["a", "button"] } }. Result: a smaller ElementMap object with the same shape.""";

    private static final String FILTER_ELEMENTMAP_PARAM =
        """
        REQUIRED. The exact ElementMap object returned by \
        frap_build_element_map. Pass it through unchanged.""";

    private static final String FILTER_FILTER_PARAM =
        """
        REQUIRED. Filter spec { interactive_only, min_cluster_size, tags }. \
        interactive_only=true keeps only clickable/typeable elements (buttons, links, \
        inputs); min_cluster_size=N drops clusters with fewer than N members; \
        tags=[...] keeps only those HTML tag names, e.g. ['a','button']. Set only the \
        fields you need.""";

    private static final String HEAL_DESC =
        """
        STANDALONE repair tool — NOT part of the 3-step generation pipeline. Use \
        it when a selector you ALREADY have has STOPPED matching because the page changed. \
        WHERE THE INPUTS COME FROM (all inside 'request'): primary_selector = your \
        old/broken selector string; original_signature = that element's structural \
        fingerprint, copied from the element's 'signature' field in an ElementMap you \
        built earlier with frap_build_element_map; dom_snapshot = a FRESH snapshot OBJECT \
        obtained by running frap_snapshot_script in the page RIGHT NOW (same { html, \
        elements } shape as STEP 1); min_confidence = a 0..1 threshold. OUTPUT: a \
        HealResult = { healed, selector, confidence, top_candidates, ... }. If \
        healed=true, 'selector' is the repaired locator and 'confidence' is how sure frap \
        is. SAFETY: if frap is unsure (several equally likely matches, or everything is \
        below min_confidence) it returns healed=false instead of guessing wrong — then \
        re-snapshot or make the selector more specific. \
        EXAMPLE — call arguments: { "request": { "primary_selector": \
        "#nav-link-main", "original_signature": <the element's signature copied from a \
        prior ElementMap>, "dom_snapshot": { "html": "<html>...</html>", \
        "elements": [ ... ] }, "min_confidence": 0.85 } }. Result: { "healed": true, \
        "selector": "#nav-link-main", "confidence": 0.91, "top_candidates": [] }.""";

    private static final String HEAL_REQUEST_PARAM =
        """
        REQUIRED. A HealRequest object: { primary_selector: <old/broken \
        selector string>, original_signature: <the element's Signature copied from a \
        prior ElementMap element's 'signature' field>, dom_snapshot: <a FRESH { html, \
        elements } snapshot object from running frap_snapshot_script now>, \
        min_confidence: <0..1, optional> }.""";

    private final FrapToolService service;

    public FrapTools(FrapToolService service) {
        this.service = service;
    }

    @McpTool(
        name = "frap_build_element_map",
        description = BUILD_DESC
    )
    public ElementMap frapBuildElementMap(
        @McpToolParam(
            description = BUILD_DOMSNAPSHOT_PARAM,
            required = true
        ) DOMSnapshot domSnapshot,
        @McpToolParam(
            description = BUILD_OPTIONS_PARAM,
            required = false
        ) MapOptions options
    ) {
        return service.buildElementMap(domSnapshot, options);
    }

    @McpTool(
        name = "frap_generate_page_object",
        description = GENERATE_DESC
    )
    public GeneratedArtifact frapGeneratePageObject(
        @McpToolParam(
            description = GENERATE_ELEMENTMAP_PARAM,
            required = true
        ) ElementMap elementMap,
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
        GenerateOptions options = new GenerateOptions(language, className, packageName, true);
        return service.generatePageObject(elementMap, options);
    }

    @McpTool(
        name = "frap_filter_element_map",
        description = FILTER_DESC
    )
    public ElementMap frapFilterElementMap(
        @McpToolParam(
            description = FILTER_ELEMENTMAP_PARAM,
            required = true
        ) ElementMap elementMap,
        @McpToolParam(
            description = FILTER_FILTER_PARAM,
            required = true
        ) FilterSpec filter
    ) {
        return service.filterElementMap(elementMap, filter);
    }

    @McpTool(
        name = "frap_heal",
        description = HEAL_DESC
    )
    public HealResult frapHeal(
        @McpToolParam(
            description = HEAL_REQUEST_PARAM,
            required = true
        ) HealRequest request
    ) {
        return service.heal(request);
    }
}
