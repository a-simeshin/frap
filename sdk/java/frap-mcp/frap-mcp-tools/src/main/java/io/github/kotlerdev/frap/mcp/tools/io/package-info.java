/**
 * File-based I/O support for the frap MCP tools (local / stdio transport).
 *
 * <p>On the local transport the server and the agent share one filesystem, so large
 * artifacts (DOM snapshots, element maps, generated source) are passed by absolute
 * path instead of inline through the agent context. This package hosts the
 * {@code ArtifactStore} (read/write/summarize) plus the compact result records
 * ({@code ElementMapSummary}, {@code ElementMapFileResult},
 * {@code GeneratedArtifactFileResult}) returned by the file-based tools.</p>
 */
package io.github.kotlerdev.frap.mcp.tools.io;
