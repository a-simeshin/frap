package io.github.kotlerdev.frap.core.dto;

import com.fasterxml.jackson.annotation.JsonProperty;

public record LocatorRecommendation(
    @JsonProperty("selector") String selector,
    @JsonProperty("strategy") String strategy,
    @JsonProperty("confidence") double confidence,
    @JsonProperty("value") String value,
    @JsonProperty("scope") String scope,
    @JsonProperty("filter_text") String filterText,
    @JsonProperty("match_count") Integer matchCount
) {}
