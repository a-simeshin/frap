package io.github.kotlerdev.frap.mcp.tools.io;

import com.fasterxml.jackson.databind.ObjectMapper;

import io.github.kotlerdev.frap.core.dto.Cluster;
import io.github.kotlerdev.frap.core.dto.ClusterType;
import io.github.kotlerdev.frap.core.dto.ElementMap;
import io.github.kotlerdev.frap.core.dto.ElementNode;
import io.github.kotlerdev.frap.core.dto.LocatorRecommendation;

import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;

import org.springframework.beans.factory.annotation.Qualifier;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.stereotype.Component;
import org.springframework.util.Assert;
import org.springframework.util.StringUtils;

/**
 * Filesystem-backed artifact store for the local (stdio) transport.
 *
 * <p>Resolves a configurable base work directory ({@code frap.io.work-dir}, default
 * {@code ${java.io.tmpdir}/frap}), reads/writes frap artifacts under unique names and
 * returns absolute paths. No auto-cleanup is performed — the local single-user model
 * leaves artifact lifetime to the operator.</p>
 */
@Component
public class ArtifactStore {

    /** Confidence threshold a locator must reach to count as "strong". */
    private static final double STRONG_CONFIDENCE = 0.80;

    /** Jackson mapper configured for the frap JSON contract (snake_case, tolerant, non-null). */
    private final ObjectMapper objectMapper;

    /** Absolute base directory under which all artifacts are written. */
    private final Path workDir;

    /**
     * @param objectMapper the frap artifact mapper (see {@code FrapCoreConfig})
     * @param workDirProperty value of {@code frap.io.work-dir}; blank → {@code ${java.io.tmpdir}/frap}
     */
    public ArtifactStore(
        @Qualifier("frapArtifactObjectMapper") final ObjectMapper objectMapper,
        @Value("${frap.io.work-dir:}") final String workDirProperty
    ) {
        this.objectMapper = objectMapper;
        this.workDir = resolveWorkDir(workDirProperty);
    }

    private static Path resolveWorkDir(final String workDirProperty) {
        if (StringUtils.hasText(workDirProperty)) {
            return Path.of(workDirProperty).toAbsolutePath().normalize();
        }
        return Path.of(System.getProperty("java.io.tmpdir"), "frap").toAbsolutePath().normalize();
    }

    /**
     * Returns the absolute base work directory.
     *
     * @return the resolved work directory
     */
    public String workDir() {
        return workDir.toString();
    }

    /**
     * Reads and parses a JSON file into the given type.
     *
     * @param absolutePath absolute path to the JSON file
     * @param type         target type
     * @param <T>          target type
     * @return the parsed value
     * @throws IllegalStateException if the file is missing or unreadable
     */
    public <T> T readJson(final String absolutePath, final Class<T> type) {
        Assert.hasText(absolutePath, "absolutePath cannot be blank");
        Assert.notNull(type, "type cannot be null");

        final Path path = Path.of(absolutePath);
        if (!Files.isReadable(path)) {
            throw new IllegalStateException("frap cannot read input file: " + absolutePath);
        }
        try {
            return objectMapper.readValue(path.toFile(), type);
        } catch (final IOException e) {
            throw new IllegalStateException(
                "frap failed to parse input file " + absolutePath + ": " + e.getMessage(), e);
        }
    }

    /**
     * Writes a value as JSON under the work directory using a unique name
     * {@code <kind>-<UUID>.json}.
     *
     * @param kind  artifact kind, used as the filename prefix
     * @param value the value to serialize
     * @return the absolute path of the written file
     */
    public String writeJson(final String kind, final Object value) {
        Assert.hasText(kind, "kind cannot be blank");
        Assert.notNull(value, "value cannot be null");

        final Path target = ensureWorkDir().resolve(kind + "-" + UUID.randomUUID() + ".json");
        try {
            objectMapper.writeValue(target.toFile(), value);
            return target.toAbsolutePath().toString();
        } catch (final IOException e) {
            throw new UncheckedIOException("frap failed to write artifact " + target + ": " + e.getMessage(), e);
        }
    }

    /**
     * Writes a raw JSON string verbatim under the work directory using a unique name
     * {@code <kind>-<UUID>.json}.
     *
     * <p>Unlike {@link #writeJson(String, Object)} this performs no serialization: the
     * supplied string is written byte-for-byte (UTF-8). Used for ingesting externally
     * produced JSON (e.g. a browser-posted DOM snapshot) without round-tripping it
     * through a DTO.</p>
     *
     * @param kind artifact kind, used as the filename prefix
     * @param json raw JSON content to write verbatim
     * @return the absolute path of the written file
     */
    public String writeRawJson(final String kind, final String json) {
        Assert.hasText(kind, "kind cannot be blank");
        Assert.notNull(json, "json cannot be null");

        final Path target = ensureWorkDir().resolve(kind + "-" + UUID.randomUUID() + ".json");
        try {
            Files.writeString(target, json, StandardCharsets.UTF_8);
            return target.toAbsolutePath().toString();
        } catch (final IOException e) {
            throw new UncheckedIOException("frap failed to write artifact " + target + ": " + e.getMessage(), e);
        }
    }

    /**
     * Writes text content under the work directory, preserving the given relative path
     * (used for generated source files whose package layout matters).
     *
     * @param relativePath path relative to the work directory
     * @param content      file content
     * @return the absolute path of the written file
     */
    public String writeText(final String relativePath, final String content) {
        Assert.hasText(relativePath, "relativePath cannot be blank");
        Assert.notNull(content, "content cannot be null");

        final Path target = ensureWorkDir().resolve(relativePath).normalize();
        try {
            Files.createDirectories(target.getParent());
            Files.writeString(target, content, StandardCharsets.UTF_8);
            return target.toAbsolutePath().toString();
        } catch (final IOException e) {
            throw new UncheckedIOException("frap failed to write file " + target + ": " + e.getMessage(), e);
        }
    }

    private Path ensureWorkDir() {
        try {
            return Files.createDirectories(workDir);
        } catch (final IOException e) {
            throw new UncheckedIOException("frap failed to create work dir " + workDir + ": " + e.getMessage(), e);
        }
    }

    /**
     * Computes a compact digest of an element map.
     *
     * @param map the element map
     * @return the summary digest
     */
    public ElementMapSummary summarize(final ElementMap map) {
        Assert.notNull(map, "map cannot be null");

        final List<ElementNode> elements = nullSafe(map.elements());
        final List<Cluster> clusters = nullSafe(map.clusters());

        return new ElementMapSummary(
            elements.size(),
            clusters.size(),
            countByType(clusters, ClusterType.SINGLE),
            countByType(clusters, ClusterType.LIST),
            largestListSize(clusters),
            confAvg(elements),
            countStrongLocators(elements),
            strategyCounts(elements)
        );
    }

    private static int countByType(final List<Cluster> clusters, final ClusterType type) {
        return (int) clusters.stream()
            .filter(cluster -> cluster.clusterType() == type)
            .count();
    }

    private static int largestListSize(final List<Cluster> clusters) {
        return clusters.stream()
            .filter(cluster -> cluster.clusterType() == ClusterType.LIST)
            .mapToInt(cluster -> nullSafe(cluster.elementIds()).size())
            .max()
            .orElse(0);
    }

    private static double confAvg(final List<ElementNode> elements) {
        return elements.stream()
            .mapToDouble(ElementNode::confidence)
            .average()
            .orElse(0.0);
    }

    private static int countStrongLocators(final List<ElementNode> elements) {
        return (int) elements.stream()
            .filter(element -> element.confidence() >= STRONG_CONFIDENCE)
            .count();
    }

    private static Map<String, Integer> strategyCounts(final List<ElementNode> elements) {
        final Map<String, Integer> counts = new LinkedHashMap<>();
        elements.stream()
            .map(ElementNode::locator)
            .filter(locator -> locator != null && locator.strategy() != null)
            .map(LocatorRecommendation::strategy)
            .forEach(strategy -> counts.merge(strategy, 1, Integer::sum));
        return counts;
    }

    private static <T> List<T> nullSafe(final List<T> list) {
        return list == null ? List.of() : list;
    }
}
