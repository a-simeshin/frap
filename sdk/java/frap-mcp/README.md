# frap-mcp (Java)

Spring Boot / Spring AI implementation of the frap MCP server. It wraps the
native `frap-core-rpc` binary and exposes frap's self-healing selector engine as
**6 MCP tools** over two transports:

| Module | Transport | I/O mode |
|--------|-----------|----------|
| `frap-mcp-stdio` | stdio (local) | **file** |
| `frap-mcp-http`  | streamable-http | **inline** |
| `frap-mcp-tools` | shared `@McpTool` beans + core client | — |

All 6 tools (`frap_help`, `frap_snapshot_script`, `frap_build_element_map`,
`frap_filter_element_map`, `frap_generate_page_object`, `frap_heal`) are present
on **both** transports; only the way large artefacts are passed differs.
`frap_help` returns a mode-aware beginner guide; `frap_snapshot_script` returns
the browser-side capture JS — both are always-on (not gated by `frap.io.mode`).

## I/O modes (`frap.io.mode`)

The mode is set per-transport in each runner's `application.properties` and gates
the tool beans via `@ConditionalOnProperty`:

| `frap.io.mode` | Runner | Beans active | Artefact passing |
|----------------|--------|--------------|------------------|
| `file`   | stdio | `FrapFileTools` + `FrapSnapshotTool` + `FrapHelpTool` | by **absolute path + digest** |
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
remote with no shared FS, so HTTP stays **inline** — behaviour is unchanged.

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

## Build & test

```bash
# Build the native binary first (resolved via FRAP_CORE_BIN or repo-relative path)
cargo build -p frap-core --bin frap-core-rpc --release

# Unit layer (shared tools)
mvn -f sdk/java/frap-mcp/pom.xml -pl frap-mcp-tools test

# Integration layer (stdio, real native binary)
mvn -f sdk/java/frap-mcp/pom.xml -pl frap-mcp-stdio verify

# Full build of both runners
mvn -f sdk/java/frap-mcp/pom.xml verify
```
