package io.github.kotlerdev.frap.mcp.tools;

import static org.assertj.core.api.Assertions.assertThat;
import static org.mockito.ArgumentMatchers.any;
import static org.mockito.ArgumentMatchers.eq;
import static org.mockito.Mockito.verify;
import static org.mockito.Mockito.when;

import io.github.kotlerdev.frap.core.dto.DOMSnapshot;
import io.github.kotlerdev.frap.core.dto.ElementMap;
import io.github.kotlerdev.frap.core.dto.FilterSpec;
import io.github.kotlerdev.frap.core.dto.GeneratedArtifact;
import io.github.kotlerdev.frap.core.dto.GenerateOptions;
import io.github.kotlerdev.frap.core.dto.HealRequest;
import io.github.kotlerdev.frap.core.dto.HealResult;
import io.github.kotlerdev.frap.core.dto.MapOptions;

import java.util.Collections;
import java.util.List;

import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.extension.ExtendWith;
import org.mockito.ArgumentCaptor;
import org.mockito.Captor;
import org.mockito.Mock;
import org.mockito.junit.jupiter.MockitoExtension;

/**
 * Unit tests for {@link FrapTools}: the inline (HTTP) @McpTool layer.
 *
 * <p>After the file-mode refactor {@link FrapTools} no longer talks to the native client
 * directly — it delegates to {@link FrapToolService}, the single source of business logic.
 * These tests mock that service and verify that the inline tool signatures and delegation
 * are unchanged: each tool forwards its arguments verbatim, maps options correctly and
 * returns the service result inline (HTTP schema 1:1 as before).</p>
 */
@ExtendWith(MockitoExtension.class)
@DisplayName("FrapTools (inline)")
class FrapToolsTest {

    /** Mocked business logic (delegation target). */
    @Mock
    private FrapToolService service;

    @Captor
    private ArgumentCaptor<GenerateOptions> generateOptionsCaptor;

    /** Tool under test. */
    private FrapTools tools;

    @BeforeEach
    void setUp() {
        tools = new FrapTools(service);
    }

    // --- helpers: minimal valid DTO instances -------------------------------

    private static ElementMap emptyMap() {
        return new ElementMap(Collections.emptyList(), Collections.emptyList(), null);
    }

    // --- frap_build_element_map ---------------------------------------------

    @Test
    @DisplayName("frap_build_element_map delegates to service.buildElementMap and returns inline")
    void frapBuildElementMapDelegatesToService() {
        // given
        final DOMSnapshot snap = new DOMSnapshot("<html></html>");
        final MapOptions opts = MapOptions.defaults();
        final ElementMap expected = emptyMap();
        when(service.buildElementMap(snap, opts)).thenReturn(expected);

        // when
        final ElementMap actual = tools.frapBuildElementMap(snap, opts);

        // then
        assertThat(actual).isSameAs(expected);
        verify(service).buildElementMap(snap, opts);
    }

    // --- frap_generate_page_object ------------------------------------------

    @Test
    @DisplayName("frap_generate_page_object maps language/class/package into GenerateOptions")
    void frapGeneratePageObjectMapsLanguageIntoGenerateOptions() {
        // given
        final ElementMap map = emptyMap();
        final GeneratedArtifact expected = new GeneratedArtifact(Collections.emptyList());
        when(service.generatePageObject(eq(map), any(GenerateOptions.class))).thenReturn(expected);

        // when
        final GeneratedArtifact actual =
            tools.frapGeneratePageObject(map, "java_playwright", "PaymentsPage", "com.example.pages");

        // then
        assertThat(actual).isSameAs(expected);
        verify(service).generatePageObject(eq(map), generateOptionsCaptor.capture());

        final GenerateOptions captured = generateOptionsCaptor.getValue();
        assertThat(captured.language()).isEqualTo("java_playwright");
        assertThat(captured.className()).isEqualTo("PaymentsPage");
        assertThat(captured.packageName()).isEqualTo("com.example.pages");
        assertThat(captured.includeSignatures()).isTrue();
    }

    // --- frap_filter_element_map --------------------------------------------

    @Test
    @DisplayName("frap_filter_element_map delegates to service.filterElementMap")
    void frapFilterElementMapDelegatesToService() {
        // given
        final ElementMap map = emptyMap();
        final FilterSpec spec = new FilterSpec(true, 2, List.of("button"));
        final ElementMap expected = emptyMap();
        when(service.filterElementMap(map, spec)).thenReturn(expected);

        // when
        final ElementMap actual = tools.frapFilterElementMap(map, spec);

        // then
        assertThat(actual).isSameAs(expected);
        verify(service).filterElementMap(map, spec);
    }

    // --- frap_heal ----------------------------------------------------------

    @Test
    @DisplayName("frap_heal delegates to service.heal and returns the result inline")
    void frapHealDelegatesToService() {
        // given
        final HealRequest request = new HealRequest("#old", null, new DOMSnapshot("<html></html>"));
        final HealResult expected = HealResult.noHeal("#old", null);
        when(service.heal(request)).thenReturn(expected);

        // when
        final HealResult actual = tools.frapHeal(request);

        // then
        assertThat(actual).isSameAs(expected);
        verify(service).heal(request);
    }
}
