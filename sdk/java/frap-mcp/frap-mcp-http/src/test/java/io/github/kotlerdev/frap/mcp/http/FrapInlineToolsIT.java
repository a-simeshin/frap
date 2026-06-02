package io.github.kotlerdev.frap.mcp.http;

import static org.assertj.core.api.Assertions.assertThat;

import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.PropertyNamingStrategies;
import io.github.kotlerdev.frap.core.dto.DOMSnapshot;
import io.github.kotlerdev.frap.core.dto.ElementMap;
import io.github.kotlerdev.frap.core.dto.ElementNode;
import io.github.kotlerdev.frap.core.dto.GeneratedArtifact;
import io.github.kotlerdev.frap.core.dto.MapOptions;
import io.github.kotlerdev.frap.mcp.tools.FrapTools;
import java.io.InputStream;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.boot.test.context.SpringBootTest;
import org.springframework.test.context.DynamicPropertyRegistry;
import org.springframework.test.context.DynamicPropertySource;

/**
 * Integration tests for the INLINE frap MCP tools against the REAL bundled native binary.
 *
 * <p>This is the http-module home of the inline tool set (relocated from the old stdio
 * {@code FrapToolsIT}). {@code @SpringBootTest} boots the full {@link HttpApplication} context,
 * which component-scans {@code io.github.kotlerdev.frap.mcp.tools}. The inline {@link FrapTools}
 * bean is gated with {@code @ConditionalOnProperty(name="frap.io.mode", havingValue="inline",
 * matchIfMissing=true)}; the http runner sets {@code frap.io.mode=inline}, and the test re-asserts
 * that mode explicitly via {@link DynamicPropertySource}.</p>
 *
 * <p>The (lazy) {@code FrapCoreClient} is created via {@code FrapRpcClient.create()}, which spawns
 * the genuine Rust {@code frap-core-rpc} binary bundled inside {@code frap-core-java}. So these
 * tests exercise the true Spring &rarr; frap-core &rarr; native-binary path, not a mock.</p>
 *
 * <p>Regression guard: HTTP keeps inline semantics — {@code frapBuildElementMap} returns a real,
 * fully-populated in-memory {@link ElementMap} (content inline), NOT a file path.</p>
 *
 * <p>The DOM snapshot fixture ({@code sample-snapshot.json}) is synthetic and PII-free. It is
 * deserialized with the same Jackson configuration {@code FrapRpcClient} uses (SNAKE_CASE naming,
 * tolerant of unknown fields).</p>
 */
@SpringBootTest(classes = HttpApplication.class)
class FrapInlineToolsIT {

    private static final ObjectMapper MAPPER = new ObjectMapper()
        .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)
        .disable(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES);

    private static DOMSnapshot snapshot;

    @Autowired
    private FrapTools tools;

    @DynamicPropertySource
    static void frapProperties(DynamicPropertyRegistry registry) {
        registry.add("frap.io.mode", () -> "inline");
    }

    @BeforeAll
    static void loadFixture() throws Exception {
        try (InputStream in = FrapInlineToolsIT.class.getResourceAsStream("/sample-snapshot.json")) {
            assertThat(in).as("sample-snapshot.json must be on the test classpath").isNotNull();
            snapshot = MAPPER.readValue(in, DOMSnapshot.class);
        }
        assertThat(snapshot.elements()).as("fixture must declare DOM elements").isNotEmpty();
    }

    @Test
    void build_element_map_returns_inline_map_from_the_real_binary() {
        ElementMap map = tools.frapBuildElementMap(snapshot, MapOptions.defaults());

        assertThat(map)
            .as("inline build must return the ElementMap object itself, not a path")
            .isNotNull();
        // The fixed contract is top-level `elements`, NOT `nodes`.
        assertThat(map.elements())
            .as("real binary must return a non-empty in-memory element-map (content inline)")
            .isNotNull()
            .isNotEmpty();

        ElementNode first = map.elements().get(0);
        assertThat(first.recommendedSelector())
            .as("each element must carry a non-empty recommended selector")
            .isNotBlank();
    }

    @Test
    void generate_page_object_emits_a_non_empty_artifact() {
        ElementMap map = tools.frapBuildElementMap(snapshot, MapOptions.defaults());

        GeneratedArtifact artifact =
            tools.frapGeneratePageObject(map, "java_playwright", "SamplePage", "com.example.pages");

        assertThat(artifact).isNotNull();
        assertThat(artifact.files())
            .as("generated artifact must contain at least one file (content inline)")
            .isNotNull()
            .isNotEmpty();
        assertThat(artifact.files().get(0).content())
            .as("generated page object content must be non-empty")
            .isNotBlank();
    }
}
