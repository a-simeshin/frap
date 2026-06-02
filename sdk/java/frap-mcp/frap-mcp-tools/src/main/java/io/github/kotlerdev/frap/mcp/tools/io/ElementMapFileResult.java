package io.github.kotlerdev.frap.mcp.tools.io;

import com.fasterxml.jackson.annotation.JsonProperty;

/**
 * Result of a file-based tool that writes an {@code ElementMap} to disk.
 *
 * @param elementMapPath absolute path to the written element-map JSON file
 * @param summary        compact digest of the written map
 */
public record ElementMapFileResult(
    @JsonProperty("element_map_path") String elementMapPath,
    @JsonProperty("summary") ElementMapSummary summary
) {}
