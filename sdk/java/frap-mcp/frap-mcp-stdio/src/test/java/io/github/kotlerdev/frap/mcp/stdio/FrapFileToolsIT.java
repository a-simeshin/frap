package io.github.kotlerdev.frap.mcp.stdio;

import static org.assertj.core.api.Assertions.assertThat;

import io.github.kotlerdev.frap.core.dto.MapOptions;
import io.github.kotlerdev.frap.mcp.tools.FrapFileTools;
import io.github.kotlerdev.frap.mcp.tools.io.ElementMapFileResult;
import io.github.kotlerdev.frap.mcp.tools.io.GeneratedArtifactFileResult;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.boot.test.context.SpringBootTest;
import org.springframework.test.context.DynamicPropertyRegistry;
import org.springframework.test.context.DynamicPropertySource;

/**
 * Integration tests for the FILE-MODE frap MCP tools against the REAL bundled native binary.
 *
 * <p>{@code @SpringBootTest} boots the full {@link StdioApplication} context. The stdio runner
 * is the file-mode runner ({@code frap.io.mode=file} in its {@code application.properties}); the
 * test re-asserts that mode explicitly and points {@code frap.io.work-dir} at a JUnit
 * {@link TempDir} via {@link DynamicPropertySource}, so every artefact is written under an
 * isolated temp directory.</p>
 *
 * <p>The context component-scans {@code io.github.kotlerdev.frap.mcp.tools} and, because the mode
 * is {@code file}, activates the {@link FrapFileTools} bean (gated with
 * {@code @ConditionalOnProperty(name="frap.io.mode", havingValue="file")}) on top of the (lazy)
 * {@code FrapCoreClient}. That client is created via {@code FrapRpcClient.create()}, which spawns
 * the genuine Rust {@code frap-core-rpc} binary bundled inside {@code frap-core-java}. So these
 * tests exercise the true Spring &rarr; frap-core &rarr; native-binary path AND real file I/O, not
 * a mock.</p>
 *
 * <p>File-mode tools take/return absolute file paths. The DOM snapshot fixture
 * ({@code sample-snapshot.json}, synthetic and PII-free) is copied from the test classpath to a
 * real file under the temp work-dir first, because the tool reads a path, not the classpath.</p>
 */
@SpringBootTest(classes = StdioApplication.class, webEnvironment = SpringBootTest.WebEnvironment.NONE)
class FrapFileToolsIT {

    /** Static temp dir so it is resolvable from the static {@link DynamicPropertySource} hook. */
    @TempDir
    static Path workDir;

    /** Absolute path to the snapshot fixture materialised on disk (tools read a path). */
    private static Path snapshotPath;

    @Autowired
    private FrapFileTools tools;

    /**
     * Forces file-mode and routes all artefacts to the test temp dir. The stdio runner already
     * sets {@code frap.io.mode=file}, but we set it explicitly here for clarity and to keep the
     * test self-describing.
     */
    @DynamicPropertySource
    static void frapProperties(DynamicPropertyRegistry registry) {
        registry.add("frap.io.mode", () -> "file");
        registry.add("frap.io.work-dir", () -> workDir.toAbsolutePath().toString());
    }

    @BeforeAll
    static void materializeFixture() throws Exception {
        snapshotPath = workDir.resolve("sample-snapshot.json").toAbsolutePath();
        try (InputStream in = FrapFileToolsIT.class.getResourceAsStream("/sample-snapshot.json")) {
            assertThat(in).as("sample-snapshot.json must be on the test classpath").isNotNull();
            Files.copy(in, snapshotPath);
        }
        assertThat(Files.size(snapshotPath))
            .as("materialised snapshot fixture must be non-empty")
            .isPositive();
    }

    @Test
    void buildElementMap_writes_map_file_and_returns_absolute_path_with_summary() throws Exception {
        ElementMapFileResult result =
            tools.frapBuildElementMap(snapshotPath.toString(), MapOptions.defaults());

        assertThat(result).isNotNull();

        Path mapPath = Path.of(result.elementMapPath());
        assertThat(mapPath.isAbsolute())
            .as("returned element-map path must be absolute: %s", result.elementMapPath())
            .isTrue();
        assertThat(Files.exists(mapPath))
            .as("the real binary's element-map file must exist on disk: %s", mapPath)
            .isTrue();
        assertThat(Files.size(mapPath))
            .as("the written element-map file must be non-empty")
            .isPositive();

        assertThat(result.summary())
            .as("file result must carry a digest summary")
            .isNotNull();
        assertThat(result.summary().elementCount())
            .as("summary element count must be > 0 from the real binary")
            .isGreaterThan(0);
        assertThat(result.summary().strategyCounts())
            .as("summary must report a non-empty locator-strategy breakdown")
            .isNotNull()
            .isNotEmpty();
    }

    @Test
    void generatePageObject_writes_java_files_to_workdir() throws Exception {
        // Chain: build the element-map file first, then feed its path into generate.
        ElementMapFileResult built =
            tools.frapBuildElementMap(snapshotPath.toString(), MapOptions.defaults());
        assertThat(built).isNotNull();

        GeneratedArtifactFileResult result =
            tools.frapGeneratePageObject(built.elementMapPath(), "java_playwright", "SampleIT", "it.pkg");

        assertThat(result).isNotNull();
        assertThat(result.filePaths())
            .as("real native generator must write at least one source file")
            .isNotNull()
            .isNotEmpty();
        assertThat(result.fileCount())
            .as("file_count must match the number of returned paths")
            .isEqualTo(result.filePaths().size());

        boolean classNameSeen = false;
        for (String filePath : result.filePaths()) {
            Path path = Path.of(filePath);
            assertThat(path.isAbsolute())
                .as("each generated file path must be absolute: %s", filePath)
                .isTrue();
            assertThat(Files.exists(path))
                .as("each generated file must exist on disk: %s", path)
                .isTrue();
            assertThat(Files.size(path))
                .as("each generated file must be non-empty: %s", path)
                .isPositive();
            if (Files.readString(path).contains("SampleIT")) {
                classNameSeen = true;
            }
        }
        assertThat(classNameSeen)
            .as("at least one generated file must reference the requested class name 'SampleIT'")
            .isTrue();
    }
}
