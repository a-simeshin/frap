package io.github.kotlerdev.frap.mcp.tools.io;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.PropertyNamingStrategies;

import io.github.kotlerdev.frap.core.dto.Cluster;
import io.github.kotlerdev.frap.core.dto.ClusterType;
import io.github.kotlerdev.frap.core.dto.ElementMap;
import io.github.kotlerdev.frap.core.dto.ElementNode;
import io.github.kotlerdev.frap.core.dto.LocatorRecommendation;
import io.github.kotlerdev.frap.core.dto.MapMetadata;

import java.nio.file.Path;
import java.util.List;

import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

/**
 * Unit tests for {@link ArtifactStore}: the filesystem-backed read/write/summarize store
 * used by the file (stdio) transport.
 *
 * <p>Each test runs against a real {@link TempDir} work directory and the same Jackson
 * configuration ({@code SNAKE_CASE}, tolerant on read, {@code NON_NULL}) the production
 * {@code frapArtifactObjectMapper} bean uses, so the JSON round-trips behave exactly as in
 * production without spawning the native binary.</p>
 */
@DisplayName("ArtifactStore")
class ArtifactStoreTest {

    /** Real temporary work directory injected by JUnit; acts as {@code frap.io.work-dir}. */
    @TempDir
    private Path workDir;

    /** Store under test, constructed over the temp dir. */
    private ArtifactStore store;

    @BeforeEach
    void setUp() {
        store = new ArtifactStore(productionMapper(), workDir.toString());
    }

    /** Builds the same mapper config as the {@code frapArtifactObjectMapper} prod bean. */
    private static ObjectMapper productionMapper() {
        return new ObjectMapper()
            .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)
            .disable(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES)
            .setSerializationInclusion(JsonInclude.Include.NON_NULL);
    }

    // --- fixtures -----------------------------------------------------------

    /** Builds an element map with one LIST cluster (2 members) and one SINGLE cluster. */
    private static ElementMap sampleMap() {
        final ElementNode rowOne = new ElementNode(
            "row-1", "tr:nth-child(1)", "tr:nth-child(1)", "tr", null, "list-1",
            0.91, new LocatorRecommendation("tr:nth-child(1)", "css", 0.91, null, null, null, null));
        final ElementNode rowTwo = new ElementNode(
            "row-2", "tr:nth-child(2)", "tr:nth-child(2)", "tr", null, "list-1",
            0.85, new LocatorRecommendation("tr:nth-child(2)", "css", 0.85, null, null, null, null));
        final ElementNode submit = new ElementNode(
            "submit", "#submit", "#submit", "button", null, "single-1",
            0.70, new LocatorRecommendation("#submit", "id", 0.70, null, null, null, null));

        final Cluster listCluster = new Cluster(
            "list-1", ClusterType.LIST, List.of("row-1", "row-2"), "tr", "tr", "tbody > tr");
        final Cluster singleCluster = new Cluster(
            "single-1", ClusterType.SINGLE, List.of("submit"), "button", "button", "#submit");

        final MapMetadata metadata = new MapMetadata("https://example.test", 3, 2, 1700000000000L);

        return new ElementMap(List.of(rowOne, rowTwo, submit), List.of(listCluster, singleCluster), metadata);
    }

    // --- round-trip ---------------------------------------------------------

    @Test
    @DisplayName("writeJson then readJson round-trips an ElementMap")
    void writeThenReadElementMapRoundTrips() {
        // given
        final ElementMap original = sampleMap();

        // when
        final String path = store.writeJson("element-map", original);
        final ElementMap restored = store.readJson(path, ElementMap.class);

        // then
        assertThat(restored.elements())
            .hasSize(3)
            .extracting(ElementNode::id)
            .containsExactly("row-1", "row-2", "submit");
        assertThat(restored.clusters())
            .hasSize(2)
            .extracting(Cluster::clusterType)
            .containsExactly(ClusterType.LIST, ClusterType.SINGLE);
        assertThat(restored.metadata().url()).isEqualTo("https://example.test");
        assertThat(restored.elements().get(0).confidence()).isEqualTo(0.91);
        assertThat(restored.elements().get(0).locator().strategy()).isEqualTo("css");
    }

    // --- summarize ----------------------------------------------------------

    @Test
    @DisplayName("summarize computes cluster, confidence and strategy statistics")
    void summarizeComputesClusterAndConfidenceStats() {
        // given
        final ElementMap map = sampleMap();

        // when
        final ElementMapSummary summary = store.summarize(map);

        // then
        assertThat(summary.elementCount()).isEqualTo(3);
        assertThat(summary.clusterCount()).isEqualTo(2);
        assertThat(summary.singleClusters()).isEqualTo(1);
        assertThat(summary.listClusters()).isEqualTo(1);
        assertThat(summary.largestListSize()).isEqualTo(2);
        // (0.91 + 0.85 + 0.70) / 3 = 0.82
        assertThat(summary.confAvg()).isEqualTo((0.91 + 0.85 + 0.70) / 3.0);
        // confidence >= 0.80 → row-1 (0.91), row-2 (0.85); submit (0.70) excluded
        assertThat(summary.locatorsGe080()).isEqualTo(2);
        assertThat(summary.strategyCounts())
            .containsEntry("css", 2)
            .containsEntry("id", 1);
    }

    // --- writeJson contract -------------------------------------------------

    @Test
    @DisplayName("writeJson returns an absolute path located under the work dir")
    void writeJsonReturnsAbsolutePathUnderWorkDir() {
        // given
        final ElementMap map = sampleMap();

        // when
        final String written = store.writeJson("element-map", map);

        // then
        final Path path = Path.of(written);
        assertThat(path.isAbsolute()).isTrue();
        assertThat(path.normalize()).startsWith(workDir.toAbsolutePath().normalize());
    }

    @Test
    @DisplayName("writeJson generates unique file names across calls")
    void writeJsonGeneratesUniquePaths() {
        // given
        final ElementMap map = sampleMap();

        // when
        final String first = store.writeJson("element-map", map);
        final String second = store.writeJson("element-map", map);

        // then
        assertThat(first).isNotEqualTo(second);
    }

    // --- writeText contract -------------------------------------------------

    @Test
    @DisplayName("writeText preserves a nested relative path under the work dir")
    void writeTextPreservesNestedRelativePath() {
        // given
        final String relativePath = "com/example/pages/PaymentsPage.java";
        final String content = "package com.example.pages; class PaymentsPage {}";

        // when
        final String written = store.writeText(relativePath, content);

        // then
        final Path path = Path.of(written);
        assertThat(path.isAbsolute()).isTrue();
        assertThat(path.normalize()).startsWith(workDir.toAbsolutePath().normalize());
        assertThat(path).endsWith(Path.of("com", "example", "pages", "PaymentsPage.java"));
        assertThat(path).hasContent(content);
    }

    // --- error handling -----------------------------------------------------

    @Test
    @DisplayName("readJson on a missing path throws IllegalStateException")
    void readJsonOnMissingPathThrowsIllegalState() {
        // given
        final String missing = workDir.resolve("does-not-exist.json").toString();

        // when & then
        assertThatThrownBy(() -> store.readJson(missing, ElementMap.class))
            .isInstanceOf(IllegalStateException.class)
            .hasMessageContaining("cannot read input file");
    }
}
