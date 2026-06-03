# frap-mcp (Java)

Spring Boot / Spring AI implementation of the frap MCP server. It wraps the
native `frap-core-rpc` binary and exposes frap's self-healing selector engine as
**6 MCP tools** over three runners:

| Module | Transport | I/O mode |
|--------|-----------|----------|
| `frap-mcp-stdio` | stdio (local) | **file** |
| `frap-mcp-http`  | streamable-http | **inline** |
| `frap-mcp-http-local` | streamable-http (local) | **file** |
| `frap-mcp-tools` | shared `@McpTool` beans + core client | — |

All 6 tools (`frap_help`, `frap_snapshot_script`, `frap_build_element_map`,
`frap_filter_element_map`, `frap_generate_page_object`, `frap_heal`) are present
on **all** runners; only the way large artefacts are passed differs.
`frap_help` returns a mode-aware beginner guide; `frap_snapshot_script` returns
the browser-side capture JS — both are always-on (not gated by `frap.io.mode`).

## I/O modes (`frap.io.mode`)

The mode is set per-transport in each runner's `application.properties` and gates
the tool beans via `@ConditionalOnProperty`:

| `frap.io.mode` | Runner | Beans active | Artefact passing |
|----------------|--------|--------------|------------------|
| `file`   | stdio | `FrapFileTools` + `FrapSnapshotTool` + `FrapHelpTool` | by **absolute path + digest** |
| `file`   | http-local | `FrapFileTools` + `FrapSnapshotTool` + `FrapHelpTool` | by **absolute path + digest** (over HTTP; needs a shared FS) |
| `inline` | http  | `FrapTools` + `FrapSnapshotTool` + `FrapHelpTool`     | **inline** JSON (content in the request/response) |

`FrapSnapshotTool` (`frap_snapshot_script`) and `FrapHelpTool` (`frap_help`) are
always-on and not gated, so each runner exposes exactly one 4-tool set **plus**
the two always-on tools = **6 tools**.

`FrapTools` is gated `@ConditionalOnProperty(name="frap.io.mode",
havingValue="inline", matchIfMissing=true)`, so inline is also the default if the
property is absent.

### Why two modes

On stdio the server and the agent share one filesystem, so streaming large blobs
(a single page's ElementMap can be tens of KB) through the agent context wastes
tokens and latency. File-mode passes **paths**, not content. HTTP clients may be
remote with no shared FS, so the default HTTP runner (`frap-mcp-http`) stays
**inline** — behaviour is unchanged.

When the HTTP client *does* share a filesystem with the server (a local agent on
the same host), use `frap-mcp-http-local` — same streamable-http transport, but
file-mode semantics: artefacts move by **absolute path + digest** instead of
inline JSON. This avoids inline clients that truncate long strings (and force the
agent to reconstruct a corrupted ElementMap) while keeping HTTP for environments
where stdio is unavailable.

## file-mode (stdio): paths + digest

In file-mode the artefact-producing tools take **absolute file paths** as input
and return an **absolute path plus a compact digest** (summary) — the agent gets
a signal without reading the file.

- `frap_build_element_map(domSnapshotPath, options)` → reads `{html,elements}`
  from the file, builds the map, writes it, returns
  `{ elementMapPath, summary }`.
- `frap_filter_element_map(elementMapPath, filter)` → reads the map, filters it,
  writes the reduced map, returns `{ elementMapPath, summary }`.
- `frap_generate_page_object(elementMapPath, language, className, packageName)`
  → reads the map, generates the Page Object, writes every file to disk, returns
  `{ filePaths, fileCount, workDir }`.
- `frap_heal(domSnapshotPath, primarySelector, originalSignature, minConfidence)`
  → reads the snapshot from the file, returns the `HealResult` **inline** (small,
  no file needed).

The `summary` digest includes at least: `elementCount`, `clusterCount`,
`singleClusters`, `listClusters`, `largestListSize`, `confAvg`,
`locatorsGe080`, `strategyCounts`.

**Pass the path; do not edit the file.** Returned paths are absolute; nothing is
auto-deleted.

### Snapshot contract in file-mode

`frap_snapshot_script` returns the `SNAPSHOT_JS` string. The agent runs it in its
own browser tool (`evaluate`) to get a `{ html, elements: [...] }` object. In
file-mode:

1. Run the `frap_snapshot_script` JS in your browser tool.
2. Save the returned `{ html, elements }` object to a JSON file.
3. Pass that file's **absolute path** to `frap_build_element_map`.

(In inline-mode you pass the returned object directly to `frap_build_element_map`
instead — see `adapters/mcp/README.md` for the inline tool shapes.)

### Typical file-mode chain (stdio)

```
1. frap_snapshot_script            -> SNAPSHOT_JS (string)
2. (agent) browser.evaluate(JS)    -> { html, elements }  -> save to snapshot.json
3. frap_build_element_map(snapshot.json) -> { elementMapPath, summary }
4. frap_generate_page_object(elementMapPath, ...) -> { filePaths, fileCount, workDir }
```

## `frap.io.work-dir`

In file-mode, artefacts are written under `frap.io.work-dir`. It defaults to the
OS temp directory (`${java.io.tmpdir}/frap`) when unset; override in
`application.properties` or via `-Dfrap.io.work-dir=...`. The directory is not
cleaned automatically.

## Connect to an MCP client (Claude Code)

Two ways to wire the server in, matching the two transports.

### A) stdio runner via `java -jar` (file mode) — recommended for local

The client launches the jar itself; stdout carries JSON-RPC. Build it first:
`mvn -f sdk/java/frap-mcp/pom.xml -pl frap-mcp-stdio -am package -DskipTests`.

CLI:

```bash
claude mcp add frap-stdio --transport stdio -- \
  java -jar /ABS/PATH/sdk/java/frap-mcp/frap-mcp-stdio/target/frap-mcp-stdio.jar
```

JSON (`.mcp.json` in the project, or `~/.claude.json`):

```json
{
  "mcpServers": {
    "frap-stdio": {
      "command": "java",
      "args": [
        "-jar",
        "/ABS/PATH/sdk/java/frap-mcp/frap-mcp-stdio/target/frap-mcp-stdio.jar"
      ]
    }
  }
}
```

That minimal config is enough on a platform whose native binary is bundled in the
jar — `FrapRpcClient` extracts it automatically (resolution order:
`FRAP_CORE_BIN` env → `crates/target/{release,debug}/frap-core-rpc` relative to the
launch dir → **bundled binary extracted from the jar**). No env needed.

Optional extras:

```json
{
  "mcpServers": {
    "frap-stdio": {
      "command": "java",
      "args": [
        "-jar", "/ABS/PATH/.../frap-mcp-stdio.jar",
        "--frap.io.work-dir=/tmp/frap"
      ],
      "env": { "FRAP_CORE_BIN": "/ABS/PATH/crates/target/release/frap-core-rpc" }
    }
  }
}
```

- `--frap.io.work-dir=...` — optional (Spring Boot accepts `--prop=value`); default `${java.io.tmpdir}/frap`.
- `env.FRAP_CORE_BIN` — **only needed** when your platform's binary is not bundled (the jar currently ships only `macos-aarch64`, so Linux/Windows need it — or add the binaries to `META-INF/native/`), or to point at a freshly built dev binary.
- This runner is **file mode**: tools take/return absolute paths + digest.

### B) streamable-http runner (inline mode)

Start the server manually, then register its URL:

```bash
java -jar /ABS/PATH/sdk/java/frap-mcp/frap-mcp-http/target/frap-mcp-http.jar   # serves http://localhost:8080/mcp
claude mcp add frap-http --transport http http://localhost:8080/mcp
```

JSON:

```json
{
  "mcpServers": {
    "frap-http": { "type": "http", "url": "http://localhost:8080/mcp" }
  }
}
```

- This runner is **inline mode**: tools take/return objects (content in the JSON). Behaviour is unchanged from before.
- Override the port with `--server.port=NNNN` if 8080 is taken.

### C) streamable-http runner, file mode (`frap-mcp-http-local`)

Same transport as (B) but **file mode** — for a local agent that shares the
filesystem with the server. Tools take/return **absolute paths + digest** instead
of inline JSON, so a client that truncates long strings can't corrupt the
ElementMap. The default port is **8765** (so it can coexist with the inline
`frap-mcp-http` on 8080). Build it first:
`mvn -f sdk/java/frap-mcp/pom.xml -pl frap-mcp-http-local -am package -DskipTests`.

Start the server manually, then register its URL:

```bash
java -jar /ABS/PATH/sdk/java/frap-mcp/frap-mcp-http-local/target/frap-mcp-http-local.jar   # serves http://localhost:8765/mcp
claude mcp add frap-local --transport http http://localhost:8765/mcp
```

JSON:

```json
{
  "mcpServers": {
    "frap-local": { "type": "http", "url": "http://localhost:8765/mcp" }
  }
}
```

- This runner is **file mode**: tools take/return absolute paths + digest (same
  signatures as the stdio runner).
- **Requires a shared filesystem** between client and server. The agent must be
  able to read/write the paths the server returns — run both on the same host and
  point them at the same `frap.io.work-dir` (default `${java.io.tmpdir}/frap` on
  both, so nothing extra is needed when they share one temp dir).
- Override the port with `--server.port=NNNN` if 8765 is taken; override the
  shared working directory with `--frap.io.work-dir=/abs/path` (Spring Boot
  accepts `--prop=value`):

  ```bash
  java -jar /ABS/PATH/.../frap-mcp-http-local.jar \
    --server.port=8765 --frap.io.work-dir=/abs/shared/frap
  ```

#### Snapshot flow in `frap-mcp-http-local`

So that no large content reaches the agent context even on the snapshot step,
`frap_snapshot_script` is **mode-aware**:

- In **file mode** it returns a **Node-context** capture script (for a browser
  `run_code` tool that exposes Node `fs`). The script collects
  `{ html, elements }`, **writes it itself** to
  `<work-dir>/snapshot-<ts>-<rand>.json` (the server injects the resolved
  `frap.io.work-dir` into the script text), and returns only
  `{ "snapshot_path": "<absolute path>" }`. Pass that path straight to
  `frap_build_element_map(domSnapshotPath=...)` — the snapshot never enters the
  agent context.
- **Fallback** for clients without a Node-context `run_code` (e.g. a page-context
  `evaluate` only): run the JS, take the returned `{ html, elements }` object, and
  **save it to a JSON file yourself**, then pass that file's absolute path to
  `frap_build_element_map`. `frap_help` documents this fallback.

```
1. frap_snapshot_script               -> Node capture JS (string)
2. (agent) browser run_code(JS)        -> writes snapshot.json, returns { snapshot_path }
3. frap_build_element_map(snapshot_path) -> { elementMapPath, summary }
4. frap_generate_page_object(elementMapPath, ...) -> { filePaths, fileCount, workDir }
```

## Build & test

```bash
# Build the native binary first (resolved via FRAP_CORE_BIN or repo-relative path)
cargo build -p frap-core --bin frap-core-rpc --release

# Unit layer (shared tools)
mvn -f sdk/java/frap-mcp/pom.xml -pl frap-mcp-tools test

# Integration layer (stdio, real native binary)
mvn -f sdk/java/frap-mcp/pom.xml -pl frap-mcp-stdio verify

# Integration layer (http-local, real jar over HTTP JSON-RPC)
mvn -f sdk/java/frap-mcp/pom.xml -pl frap-mcp-http-local verify

# Full build of all runners
mvn -f sdk/java/frap-mcp/pom.xml verify
```
