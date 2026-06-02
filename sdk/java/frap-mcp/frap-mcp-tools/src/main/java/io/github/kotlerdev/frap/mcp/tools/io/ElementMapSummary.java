package io.github.kotlerdev.frap.mcp.tools.io;

import com.fasterxml.jackson.annotation.JsonProperty;

import java.util.Map;

/**
 * Compact digest of an {@code ElementMap}, returned alongside the artifact path so a
 * weak LLM gets a useful signal without reading the (potentially large) map file.
 *
 * @param elementCount   total number of elements in the map
 * @param clusterCount   total number of clusters
 * @param singleClusters number of {@code SINGLE} clusters
 * @param listClusters   number of {@code LIST} clusters
 * @param largestListSize largest member count among {@code LIST} clusters (0 if none)
 * @param confAvg        mean confidence across all elements (0 if no elements)
 * @param locatorsGe080  number of elements with confidence &gt;= 0.80
 * @param strategyCounts count of elements per recommended locator strategy
 */
public record ElementMapSummary(
    @JsonProperty("element_count") int elementCount,
    @JsonProperty("cluster_count") int clusterCount,
    @JsonProperty("single_clusters") int singleClusters,
    @JsonProperty("list_clusters") int listClusters,
    @JsonProperty("largest_list_size") int largestListSize,
    @JsonProperty("conf_avg") double confAvg,
    @JsonProperty("locators_ge_080") int locatorsGe080,
    @JsonProperty("strategy_counts") Map<String, Integer> strategyCounts
) {}
