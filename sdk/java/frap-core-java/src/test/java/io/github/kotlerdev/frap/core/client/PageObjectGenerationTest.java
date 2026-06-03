package io.github.kotlerdev.frap.core.client;

import io.github.kotlerdev.frap.core.dto.DOMElementInfo;
import io.github.kotlerdev.frap.core.dto.DOMSnapshot;
import io.github.kotlerdev.frap.core.dto.ElementMap;
import io.github.kotlerdev.frap.core.dto.GenerateOptions;
import io.github.kotlerdev.frap.core.dto.GeneratedArtifact;
import io.github.kotlerdev.frap.core.dto.MapOptions;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.util.List;
import java.util.Map;

import static org.assertj.core.api.Assertions.assertThat;

class PageObjectGenerationTest {

    private FrapCoreClient client;

    @BeforeEach
    void setUp() throws IOException {
        client = new FrapRpcClient(FrapCoreBinaryResolver.resolve());
    }

    @AfterEach
    void tearDown() {
        if (client != null) {
            client.close();
        }
    }

    @Test
    void generatesJavaPlaywrightPageObject() throws IOException {
        DOMSnapshot snapshot = new DOMSnapshot(
            "",
            List.of(
                new DOMElementInfo(
                    "[data-testid='submit']",
                    "button",
                    Map.of("data-testid", "submit"),
                    "OK",
                    List.of("button:-"),
                    null
                )
            )
        );

        ElementMap map = client.buildElementMap(snapshot, MapOptions.defaults());
        GeneratedArtifact artifact = client.generatePageObject(
            map,
            GenerateOptions.javaPlaywright("CheckoutPage", "com.example.pages")
        );

        assertThat(artifact.files()).isNotEmpty();
        String source = artifact.files().get(0).content();
        assertThat(source).contains("public class CheckoutPage");
        assertThat(source).contains("package com.example.pages");
        // data-testid='submit' is now emitted via the semantic Playwright API (getByTestId),
        // not a raw page.locator CSS selector.
        assertThat(source).contains("public Locator");
        assertThat(source).contains("getByTestId(\"submit\")");
    }

}
