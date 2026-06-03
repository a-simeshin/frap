package io.github.kotlerdev.frap.mcp.tools;

import static org.assertj.core.api.Assertions.assertThat;
import static org.mockito.ArgumentMatchers.any;
import static org.mockito.ArgumentMatchers.eq;
import static org.mockito.Mockito.verify;
import static org.mockito.Mockito.when;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.PropertyNamingStrategies;

import io.github.kotlerdev.frap.core.dto.Cluster;
import io.github.kotlerdev.frap.core.dto.ClusterType;
import io.github.kotlerdev.frap.core.dto.DOMSnapshot;
import io.github.kotlerdev.frap.core.dto.ElementMap;
import io.github.kotlerdev.frap.core.dto.ElementNode;
import io.github.kotlerdev.frap.core.dto.FilterSpec;
import io.github.kotlerdev.frap.core.dto.GeneratedArtifact;
import io.github.kotlerdev.frap.core.dto.GeneratedFile;
import io.github.kotlerdev.frap.core.dto.GenerateOptions;
import io.github.kotlerdev.frap.core.dto.HealRequest;
import io.github.kotlerdev.frap.core.dto.HealResult;
import io.github.kotlerdev.frap.core.dto.LocatorRecommendation;
import io.github.kotlerdev.frap.core.dto.MapMetadata;
import io.github.kotlerdev.frap.core.dto.MapOptions;
import io.github.kotlerdev.frap.mcp.tools.io.ArtifactStore;
import io.github.kotlerdev.frap.mcp.tools.io.ElementMapFileResult;
import io.github.kotlerdev.frap.mcp.tools.io.GeneratedArtifactFileResult;

import java.nio.file.Path;
import java.util.List;

import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.extension.ExtendWith;
import org.junit.jupiter.api.io.TempDir;
import org.mockito.ArgumentCaptor;
import org.mockito.Captor;
import org.mockito.Mock;
import org.mockito.junit.jupiter.MockitoExtension;

/**
 * Unit tests for {@link FrapFileTools}: the file (stdio) @McpTool adapter.
 *
 * <p>The business-logic dependency {@link FrapToolService} is mocked (no native binary is
 * spawned), while a <b>real</b> {@link ArtifactStore} over a {@link TempDir} is used so the
 * file I/O — reading inputs, writing artifacts, returning absolute paths and digests — is
 * exercised for real.</p>
 */
@ExtendWith(MockitoExtension.class)
@DisplayName("FrapFileTools")
class FrapFileToolsTest {

    /** Real temporary work directory backing the {@link ArtifactStore}. */
    @TempDir
    private Path workDir;

    /** Mocked business logic (delegation target). */
    @Mock
    private FrapToolService service;

    @Captor
    private ArgumentCaptor<GenerateOptions> generateOptionsCaptor;

    /** Real store over the temp dir. */
    private ArtifactStore store;

    /** Tool under test. */
    private FrapFileTools tools;

    @BeforeEach
    void setUp() {
        store = new ArtifactStore(productionMapper(), workDir.toString());
        tools = new FrapFileTools(service, store);
    }

    /** Builds the same mapper config as the {@code frapArtifactObjectMapper} prod bean. */
    private static ObjectMapper productionMapper() {
        return new ObjectMapper()
            .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)
            .disable(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES)
            .setSerializationInclusion(JsonInclude.Include.NON_NULL);
    }

    // --- fixtures -----------------------------------------------------------

    /** Builds an element map with one LIST cluster and one SINGLE cluster. */
    private static ElementMap sampleMap() {
        final ElementNode rowOne = new ElementNode(
            "row-1", "tr:nth-child(1)", "tr:nth-child(1)", "tr", null, "list-1",
            0.91, new LocatorRecommendation("tr:nth-child(1)", "css", 0.91, null, null, null, null));
        final ElementNode submit = new ElementNode(
            "submit", "#submit", "#submit", "button", null, "single-1",
            0.88, new LocatorRecommendation("#submit", "id", 0.88, null, null, null, null));

        final Cluster listCluster = new Cluster(
            "list-1", ClusterType.LIST, List.of("row-1"), "tr", "tr", "tbody > tr");
        final Cluster singleCluster = new Cluster(
            "single-1", ClusterType.SINGLE, List.of("submit"), "button", "button", "#submit");

        final MapMetadata metadata = new MapMetadata("https://example.test", 2, 2, 1700000000000L);
        return new ElementMap(List.of(rowOne, submit), List.of(listCluster, singleCluster), metadata);
    }

    // --- frap_build_element_map ---------------------------------------------

    @Test
    @DisplayName("frap_build_element_map writes the map file and returns its path plus summary")
    void buildElementMapWritesFileAndReturnsPathPlusSummary() {
        // given
        final String snapshotPath = store.writeJson("dom-snapshot", new DOMSnapshot("<html></html>"));
        final ElementMap built = sampleMap();
        final MapOptions options = MapOptions.defaults();
        when(service.buildElementMap(any(DOMSnapshot.class), eq(options))).thenReturn(built);

        // when
        final ElementMapFileResult result = tools.frapBuildElementMap(snapshotPath, options);

        // then
        assertThat(result).isNotNull();
        final Path mapPath = Path.of(result.elementMapPath());
        assertThat(mapPath.isAbsolute()).isTrue();
        assertThat(mapPath).exists();
        assertThat(mapPath.normalize()).startsWith(workDir.toAbsolutePath().normalize());

        assertThat(result.summary()).isNotNull();
        assertThat(result.summary().elementCount()).isEqualTo(2);
        assertThat(result.summary().clusterCount()).isEqualTo(2);
        assertThat(result.summary().listClusters()).isEqualTo(1);
        assertThat(result.summary().singleClusters()).isEqualTo(1);

        // the written file is the real built map
        final ElementMap reloaded = store.readJson(result.elementMapPath(), ElementMap.class);
        assertThat(reloaded.elements()).hasSize(2);
        verify(service).buildElementMap(any(DOMSnapshot.class), eq(options));
    }

    // --- frap_generate_page_object ------------------------------------------

    @Test
    @DisplayName("frap_generate_page_object writes .java files and returns absolute paths")
    void generatePageObjectWritesJavaFilesAndReturnsAbsolutePaths() {
        // given
        final String mapPath = store.writeJson("element-map", sampleMap());
        final GeneratedFile pageFile = new GeneratedFile(
            "com/example/pages/PaymentsPage.java",
            "package com.example.pages;\npublic class PaymentsPage {}\n");
        final GeneratedFile baseFile = new GeneratedFile(
            "com/example/pages/BasePage.java",
            "package com.example.pages;\npublic class BasePage {}\n");
        final GeneratedArtifact artifact = new GeneratedArtifact(List.of(pageFile, baseFile));
        when(service.generatePageObject(any(ElementMap.class), any(GenerateOptions.class)))
            .thenReturn(artifact);

        // when
        final GeneratedArtifactFileResult result =
            tools.frapGeneratePageObject(mapPath, "java_playwright", "PaymentsPage", "com.example.pages");

        // then
        assertThat(result).isNotNull();
        assertThat(result.fileCount()).isEqualTo(2);
        assertThat(result.filePaths()).hasSize(2);
        assertThat(result.filePaths()).allSatisfy(p -> {
            final Path path = Path.of(p);
            assertThat(path.isAbsolute()).isTrue();
            assertThat(path).exists();
            assertThat(path.normalize()).startsWith(workDir.toAbsolutePath().normalize());
        });

        final Path pagePath = Path.of(result.filePaths().get(0));
        assertThat(pagePath).hasContent(pageFile.content());
        assertThat(pagePath).endsWith(Path.of("com", "example", "pages", "PaymentsPage.java"));

        verify(service).generatePageObject(any(ElementMap.class), generateOptionsCaptor.capture());
        final GenerateOptions captured = generateOptionsCaptor.getValue();
        assertThat(captured.language()).isEqualTo("java_playwright");
        assertThat(captured.className()).isEqualTo("PaymentsPage");
        assertThat(captured.packageName()).isEqualTo("com.example.pages");
    }

    // --- frap_filter_element_map --------------------------------------------

    @Test
    @DisplayName("frap_filter_element_map writes a filtered map file and returns its digest")
    void frapFilterElementMap() {
        // given
        final String mapPath = store.writeJson("element-map", sampleMap());
        final FilterSpec filter = new FilterSpec(true, 1, List.of("button"));
        final ElementNode onlyButton = new ElementNode(
            "submit", "#submit", "#submit", "button", null, "single-1",
            0.88, new LocatorRecommendation("#submit", "id", 0.88, null, null, null, null));
        final Cluster singleCluster = new Cluster(
            "single-1", ClusterType.SINGLE, List.of("submit"), "button", "button", "#submit");
        final ElementMap filtered = new ElementMap(List.of(onlyButton), List.of(singleCluster), null);
        when(service.filterElementMap(any(ElementMap.class), eq(filter))).thenReturn(filtered);

        // when
        final ElementMapFileResult result = tools.frapFilterElementMap(mapPath, filter);

        // then
        assertThat(result).isNotNull();
        final Path filteredPath = Path.of(result.elementMapPath());
        assertThat(filteredPath.isAbsolute()).isTrue();
        assertThat(filteredPath).exists();
        // a NEW file, not the input
        assertThat(result.elementMapPath()).isNotEqualTo(mapPath);

        final ElementMap reloaded = store.readJson(result.elementMapPath(), ElementMap.class);
        assertThat(reloaded.elements())
            .hasSize(1)
            .extracting(ElementNode::tag)
            .containsExactly("button");
        assertThat(result.summary().elementCount()).isEqualTo(1);
        verify(service).filterElementMap(any(ElementMap.class), eq(filter));
    }

    // --- frap_heal ----------------------------------------------------------

    @Test
    @DisplayName("frap_heal reads the snapshot file and returns the HealResult inline (no file)")
    void frapHeal() {
        // given
        final String snapshotPath = store.writeJson("dom-snapshot", new DOMSnapshot("<html></html>"));
        final HealResult expected = HealResult.noHeal("#old", null);
        when(service.heal(any(HealRequest.class))).thenReturn(expected);

        // when
        final HealResult actual = tools.frapHeal(snapshotPath, "#old", null, 0.8);

        // then — small result returned inline, no extra artifact written
        assertThat(actual).isSameAs(expected);

        final ArgumentCaptor<HealRequest> requestCaptor = ArgumentCaptor.forClass(HealRequest.class);
        verify(service).heal(requestCaptor.capture());
        final HealRequest request = requestCaptor.getValue();
        assertThat(request.primarySelector()).isEqualTo("#old");
        assertThat(request.minConfidence()).isEqualTo(0.8);
        assertThat(request.domSnapshot()).isNotNull();
    }

    @Test
    @DisplayName("frap_heal does not write any artifact file to the work dir")
    void frapHealWritesNoArtifact() {
        // given — a separate store with an empty (uncreated) work dir to observe writes
        final Path emptyDir = workDir.resolve("heal-only");
        final ArtifactStore healStore = new ArtifactStore(productionMapper(), emptyDir.toString());
        final FrapFileTools healTools = new FrapFileTools(service, healStore);
        // pre-seed only the snapshot input
        final String snapshotPath = healStore.writeJson("dom-snapshot", new DOMSnapshot("<html></html>"));
        when(service.heal(any(HealRequest.class))).thenReturn(HealResult.noHeal("#x", null));

        // when
        healTools.frapHeal(snapshotPath, "#x", null, null);

        // then — only the single snapshot file exists; heal wrote nothing extra
        assertThat(emptyDir).isDirectoryContaining(p -> p.getFileName().toString().startsWith("dom-snapshot-"));
        assertThat(emptyDir.toFile().listFiles()).hasSize(1);
    }
}
