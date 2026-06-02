package io.github.kotlerdev.frap.mcp.tools.io;

import com.fasterxml.jackson.annotation.JsonProperty;

import java.util.List;

/**
 * Result of a file-based generate tool that writes Page Object source files to disk.
 *
 * @param filePaths absolute paths to every written source file
 * @param fileCount number of files written
 * @param workDir   absolute base work directory the files were written under
 */
public record GeneratedArtifactFileResult(
    @JsonProperty("file_paths") List<String> filePaths,
    @JsonProperty("file_count") int fileCount,
    @JsonProperty("work_dir") String workDir
) {}
