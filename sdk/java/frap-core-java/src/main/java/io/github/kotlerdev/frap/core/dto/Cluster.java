package io.github.kotlerdev.frap.core.dto;

import com.fasterxml.jackson.annotation.JsonProperty;

import java.util.List;

public record Cluster(
    @JsonProperty("id") String id,
    @JsonProperty("cluster_type") ClusterType clusterType,
    @JsonProperty("element_ids") List<String> elementIds,
    @JsonProperty("prefix_signature") String prefixSignature,
    @JsonProperty("member_tag") String memberTag,
    @JsonProperty("shared_selector") String sharedSelector
) {}
