package io.github.kotlerdev.frap.core.dto;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.PropertyNamingStrategies;
import io.github.kotlerdev.frap.core.context.*;
import io.github.kotlerdev.frap.core.rca.CauseDetails;
import io.github.kotlerdev.frap.core.rca.PrimaryCause;
import io.github.kotlerdev.frap.core.rca.RcaReport;
import io.github.kotlerdev.frap.core.semantics.HealOutcome;
import io.github.kotlerdev.frap.core.semantics.HealPolicy;
import io.github.kotlerdev.frap.core.semantics.HealTrigger;
import io.github.kotlerdev.frap.core.semantics.HealingSemantics;
import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.Map;

import static org.assertj.core.api.Assertions.assertThat;

/**
 * Tests for DTO JSON serialization/deserialization.
 */
class DtoSerializationTest {

    private final ObjectMapper objectMapper = new ObjectMapper()
        .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE);

    @Test
    void testDOMTokenSerialization() throws JsonProcessingException {
        DOMToken token = new DOMToken("button", "submit", 0);
        String json = objectMapper.writeValueAsString(token);

        assertThat(json).contains("\"tag\":\"button\"");
        assertThat(json).contains("\"role\":\"submit\"");
        assertThat(json).contains("\"depth\":0");

        DOMToken parsed = objectMapper.readValue(json, DOMToken.class);
        assertThat(parsed.tag()).isEqualTo("button");
        assertThat(parsed.role()).isEqualTo("submit");
        assertThat(parsed.depth()).isZero();
    }

    @Test
    void testSignatureSerialization() throws JsonProcessingException {
        DOMToken token = new DOMToken("button", "submit", 0);
        Signature sig = new Signature(
            List.of(token),
            "button:submit",
            Map.of("data-testid", "pay-btn"),
            "Pay",
            null,
            0L,
            1
        );

        String json = objectMapper.writeValueAsString(sig);
        assertThat(json).contains("\"stable_attrs\":{\"data-testid\":\"pay-btn\"}");

        Signature parsed = objectMapper.readValue(json, Signature.class);
        assertThat(parsed.stableAttrs()).containsEntry("data-testid", "pay-btn");
        assertThat(parsed.path()).hasSize(1);
    }

    @Test
    void testHealResultSerialization() throws JsonProcessingException {
        DOMToken token = new DOMToken("button", "submit", 0);
        Signature sig = new Signature(
            List.of(token),
            "button:submit",
            Map.of("data-testid", "pay-btn"),
            1
        );

        HealResult result = HealResult.healed(
            "[data-testid='pay-btn']",
            "[data-testid='checkout-pay']",
            0.92,
            "Healed to checkout-pay",
            List.of(),
            sig
        );

        String json = objectMapper.writeValueAsString(result);
        assertThat(json).contains("\"healed\":true");
        assertThat(json).contains("\"confidence\":0.92");

        HealResult parsed = objectMapper.readValue(json, HealResult.class);
        assertThat(parsed.healed()).isTrue();
        assertThat(parsed.confidence()).isEqualTo(0.92);
    }

    @Test
    void testDOMSnapshotSerialization() throws JsonProcessingException {
        DOMElementInfo element = new DOMElementInfo(
            "[data-testid='btn']",
            "button",
            Map.of("data-testid", "btn"),
            "Click me",
            List.of("button:action"),
            null
        );

        DOMSnapshot snapshot = new DOMSnapshot(
            "<button data-testid='btn'>Click me</button>",
            List.of(element)
        );

        String json = objectMapper.writeValueAsString(snapshot);
        DOMSnapshot parsed = objectMapper.readValue(json, DOMSnapshot.class);
        assertThat(parsed.elements()).hasSize(1);
        assertThat(parsed.elements().get(0).tag()).isEqualTo("button");
    }

    @Test
    void testHealRequestSerialization() throws JsonProcessingException {
        DOMToken token = new DOMToken("button", "submit", 0);
        Signature sig = new Signature(
            List.of(token),
            "button:submit",
            Map.of("data-testid", "pay-btn"),
            1
        );

        DOMElementInfo element = new DOMElementInfo(
            "[data-testid='checkout-pay']",
            "button",
            Map.of("data-testid", "checkout-pay"),
            "Pay",
            List.of("button:submit"),
            null
        );

        DOMSnapshot snapshot = new DOMSnapshot(
            "<button data-testid='checkout-pay'>Pay</button>",
            List.of(element)
        );

        HealRequest request = new HealRequest(
            "[data-testid='pay-btn']",
            sig,
            snapshot,
            0.85
        );

        String json = objectMapper.writeValueAsString(request);
        assertThat(json).contains("\"primary_selector\":\"[data-testid='pay-btn']\"");
        assertThat(json).contains("\"min_confidence\":0.85");

        HealRequest parsed = objectMapper.readValue(json, HealRequest.class);
        assertThat(parsed.primarySelector()).isEqualTo("[data-testid='pay-btn']");
        assertThat(parsed.domSnapshot().elements()).hasSize(1);
    }

    @Test
    void testContextEventSerialization() throws JsonProcessingException {
        // UI event
        ContextEvent.UiEvent uiEvent = new ContextEvent.UiEvent(
            1000L,
            "test-1",
            "[data-testid='btn']",
            "click"
        );

        String json = objectMapper.writeValueAsString(uiEvent);
        assertThat(json).contains("\"kind\":\"ui\"");
        assertThat(json).contains("\"element\":\"[data-testid='btn']\"");

        // Network event
        NetworkEventPayload netPayload = new NetworkEventPayload(
            "GET",
            "https://api.example.com/data",
            NetworkPhase.RESPONSE
        );

        ContextEvent.NetworkEvent netEvent = new ContextEvent.NetworkEvent(
            2000L,
            "test-1",
            netPayload
        );

        String netJson = objectMapper.writeValueAsString(netEvent);
        assertThat(netJson).contains("\"kind\":\"network\"");
    }

    @Test
    void testContextTimelineSerialization() throws JsonProcessingException {
        ContextTimeline timeline = new ContextTimeline();
        timeline.addEvent(new ContextEvent.UiEvent(1000L, "btn", "click"));
        timeline.addEvent(new ContextEvent.UiEvent(2000L, "btn", "failure"));

        String json = objectMapper.writeValueAsString(timeline);
        assertThat(json).contains("\"events\"");

        ContextTimeline parsed = objectMapper.readValue(json, ContextTimeline.class);
        assertThat(parsed.events()).hasSize(2);
    }

    @Test
    void testRcaReportSerialization() throws JsonProcessingException {
        CauseDetails details = new CauseDetails(
            "/api/payment",
            500,
            "[data-testid='pay-btn']",
            null,
            0.85,
            "checkout"
        );

        RcaReport report = new RcaReport(
            PrimaryCause.API_ERROR,
            0.85,
            "Check payment API endpoint /api/payment",
            details
        );

        String json = objectMapper.writeValueAsString(report);
        assertThat(json).contains("\"primary_cause\":\"api_error\"");
        assertThat(json).contains("\"confidence\":0.85");

        RcaReport parsed = objectMapper.readValue(json, RcaReport.class);
        assertThat(parsed.primaryCause()).isEqualTo(PrimaryCause.API_ERROR);
        assertThat(parsed.details().endpoint()).isEqualTo("/api/payment");
    }

    @Test
    void testHealingSemanticsSerialization() throws JsonProcessingException {
        HealingSemantics semantics = HealingSemantics.classify(
            HealTrigger.SELECTOR_MISSING,
            HealPolicy.ALLOW,
            true,
            true
        );

        String json = objectMapper.writeValueAsString(semantics);
        assertThat(json).contains("\"trigger\":\"selector_missing\"");
        assertThat(json).contains("\"policy\":\"allow\"");
        assertThat(json).contains("\"outcome\":\"healed\"");

        HealingSemantics parsed = objectMapper.readValue(json, HealingSemantics.class);
        assertThat(parsed.outcome()).isEqualTo(HealOutcome.HEALED);
    }

    @Test
    void testLocatorRecommendationDeserializesValueField() throws JsonProcessingException {
        String json = "{\"selector\":\"[href=\\\"/x\\\"]\",\"strategy\":\"href\",\"confidence\":0.85,\"value\":\"/x\"}";

        LocatorRecommendation parsed = objectMapper.readValue(json, LocatorRecommendation.class);

        assertThat(parsed.selector()).isEqualTo("[href=\"/x\"]");
        assertThat(parsed.strategy()).isEqualTo("href");
        assertThat(parsed.confidence()).isEqualTo(0.85);
        assertThat(parsed.value()).isEqualTo("/x");
    }

    @Test
    void testLocatorRecommendationWithoutValueFieldIsBackwardCompatible() throws JsonProcessingException {
        String json = "{\"selector\":\"button.cta\",\"strategy\":\"css\",\"confidence\":0.4}";

        LocatorRecommendation parsed = objectMapper.readValue(json, LocatorRecommendation.class);

        assertThat(parsed.selector()).isEqualTo("button.cta");
        assertThat(parsed.strategy()).isEqualTo("css");
        assertThat(parsed.confidence()).isEqualTo(0.4);
        assertThat(parsed.value()).isNull();
    }

    @Test
    void testLocatorRecommendationDeserializesPipelineFields() throws JsonProcessingException {
        String json = "{\"selector\":\"button.cta\",\"strategy\":\"css\",\"confidence\":0.4,"
            + "\"value\":\"buy\",\"scope\":\"#cart\",\"filter_text\":\"Buy now\",\"match_count\":3}";

        LocatorRecommendation parsed = objectMapper.readValue(json, LocatorRecommendation.class);

        assertThat(parsed.selector()).isEqualTo("button.cta");
        assertThat(parsed.strategy()).isEqualTo("css");
        assertThat(parsed.confidence()).isEqualTo(0.4);
        assertThat(parsed.value()).isEqualTo("buy");
        assertThat(parsed.scope()).isEqualTo("#cart");
        assertThat(parsed.filterText()).isEqualTo("Buy now");
        assertThat(parsed.matchCount()).isEqualTo(3);
    }

    @Test
    void testLocatorRecommendationWithoutPipelineFieldsIsBackwardCompatible() throws JsonProcessingException {
        String json = "{\"selector\":\"button.cta\",\"strategy\":\"css\",\"confidence\":0.4,\"value\":\"buy\"}";

        LocatorRecommendation parsed = objectMapper.readValue(json, LocatorRecommendation.class);

        assertThat(parsed.selector()).isEqualTo("button.cta");
        assertThat(parsed.value()).isEqualTo("buy");
        assertThat(parsed.scope()).isNull();
        assertThat(parsed.filterText()).isNull();
        assertThat(parsed.matchCount()).isNull();
    }
}
