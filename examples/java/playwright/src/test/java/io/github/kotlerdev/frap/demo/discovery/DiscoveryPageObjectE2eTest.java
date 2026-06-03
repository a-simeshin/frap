package io.github.kotlerdev.frap.demo.discovery;

import com.microsoft.playwright.Browser;
import com.microsoft.playwright.BrowserType;
import com.microsoft.playwright.Page;
import com.microsoft.playwright.Playwright;
import io.github.kotlerdev.frap.core.dto.ClusterType;
import io.github.kotlerdev.frap.core.dto.ElementMap;
import io.github.kotlerdev.frap.core.dto.GenerateOptions;
import io.github.kotlerdev.frap.core.dto.MapOptions;
import io.github.kotlerdev.frap.playwright.extension.FrapExtension;
import io.github.kotlerdev.frap.playwright.wrapper.Frap;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Tag;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.extension.ExtendWith;
import org.junit.jupiter.api.io.TempDir;

import javax.tools.JavaCompiler;
import javax.tools.StandardJavaFileManager;
import javax.tools.ToolProvider;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.assertj.core.api.Assertions.assertThat;

/**
 * Acceptance for F004 / {@link Frap#discover} and {@link Frap#generatePageObject} (VERIFICATION.md §7).
 */
@Tag("e2e")
@ExtendWith(FrapExtension.class)
@DisplayName("Discovery and Page Object generation")
class DiscoveryPageObjectE2eTest {

    private static final String CATALOG_HTML = """
        <div class="catalog">
          <article data-testid="product-card">
            <h3>Phone</h3>
            <button class="buy">Buy</button>
          </article>
          <article data-testid="product-card">
            <h3>Tablet</h3>
            <button class="buy">Buy</button>
          </article>
        </div>
        """;

    static Playwright playwright;
    static Browser browser;
    Page page;

    @BeforeAll
    static void beforeAll() {
        playwright = Playwright.create();
        browser = playwright.chromium().launch(new BrowserType.LaunchOptions().setHeadless(true));
    }

    @AfterAll
    static void afterAll() {
        browser.close();
        playwright.close();
        Frap.clearClient();
    }

    @BeforeEach
    void beforeEach() {
        page = browser.newPage();
        page.setContent(CATALOG_HTML);
    }

    @AfterEach
    void afterEach() {
        page.close();
    }

    @Test
    void discoverFindsListClustersWithRepeatedElements() throws IOException {
        ElementMap map = Frap.discover(page, MapOptions.defaults());

        assertThat(map.elements()).isNotEmpty();

        long listClusters = map.clusters().stream()
            .filter(c -> c.clusterType() == ClusterType.LIST)
            .filter(c -> c.elementIds().size() >= 2)
            .count();

        assertThat(listClusters).as("LIST clusters with size >= 2 on catalog page").isGreaterThanOrEqualTo(1);
    }

    @Test
    void generatePageObjectWritesCompilableJava(@TempDir Path outputDir) throws IOException {
        List<Path> paths = Frap.generatePageObject(
            page,
            outputDir,
            GenerateOptions.javaPlaywright("CatalogPage", "com.example.pages")
        );

        assertThat(paths).isNotEmpty();
        Path source = paths.get(0);
        assertThat(Files.readString(source))
            .contains("package com.example.pages")
            .contains("public class CatalogPage")
            .contains("public Locator ");

        assertCompiles(source);
    }

    private static void assertCompiles(Path source) throws IOException {
        JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
        assertThat(compiler).as("JDK compiler required for compile check").isNotNull();

        try (StandardJavaFileManager fileManager = compiler.getStandardFileManager(null, null, null)) {
            Iterable<? extends javax.tools.JavaFileObject> units = fileManager.getJavaFileObjects(source);
            JavaCompiler.CompilationTask task = compiler.getTask(
                null,
                fileManager,
                diagnostics -> { },
                List.of("-classpath", playwrightClasspath()),
                null,
                units
            );
            assertThat(task.call())
                .as("generated Page Object should compile: %s", source)
                .isTrue();
        }
    }

    private static String playwrightClasspath() {
        String home = System.getProperty("user.home");
        Path m2 = Path.of(home, ".m2", "repository", "com", "microsoft", "playwright", "playwright", "1.44.0", "playwright-1.44.0.jar");
        if (Files.isRegularFile(m2)) {
            return m2.toString();
        }
        throw new IllegalStateException("Playwright JAR not found at " + m2 + "; run mvn install in sdk/java first");
    }
}
