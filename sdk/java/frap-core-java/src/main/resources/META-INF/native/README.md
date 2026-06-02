# Bundled native binaries (`frap-core-rpc`)

`frap-core-java` ships the Rust `frap-core-rpc` binary inside this jar and resolves the
correct platform at runtime (`FrapRpcClient.getBundledBinaryName()`), extracts it to a
temp file, and spawns it over the stdin/stdout NDJSON protocol.

## Expected files (6 platforms)

| Artifact file                          | OS / arch              | Rust target                   |
|----------------------------------------|------------------------|-------------------------------|
| `frap-core-rpc-linux-x86_64`           | Linux x86_64 (glibc)   | `x86_64-unknown-linux-gnu`    |
| `frap-core-rpc-linux-x86_64-musl`      | Linux x86_64 (musl)    | `x86_64-unknown-linux-musl`   |
| `frap-core-rpc-linux-aarch64`          | Linux ARM64            | `aarch64-unknown-linux-gnu`   |
| `frap-core-rpc-macos-aarch64`          | macOS Apple Silicon    | `aarch64-apple-darwin`        |
| `frap-core-rpc-macos-x86_64`           | macOS Intel            | `x86_64-apple-darwin`         |
| `frap-core-rpc-win-x64.exe`            | Windows x64            | `x86_64-pc-windows-msvc`      |

## How files get here (automated via CI)

The binaries are **git-ignored** (`.gitignore`: `META-INF/native/frap-core-rpc-*`) — they
are never committed. They are produced and bundled at build time by
`.github/workflows/build-native-binaries.yml`:

1. Six build jobs (one per target) compile `frap-core-rpc` on the matching native
   runner (Linux glibc/musl/aarch64, Windows MSVC, macOS x86_64/aarch64) and upload
   each as an artifact named after its canonical file name.
2. The final `publish-binaries` job downloads all 6 artifacts, stages them into this
   folder, runs `mvn package` so the MCP jars bundle **every platform**, then uploads
   the jars as workflow artifacts (and, on `v*` tags, attaches them to the GitHub
   Release).

Triggers: pushes to `main` that touch `crates/**` (or the workflow file), version tags
`v*`, and manual `workflow_dispatch`.

**End-user path:** none of this matters to consumers — they download the published
`frap-mcp-stdio.jar` (all 6 binaries baked in) and just connect the MCP server
(`java -jar frap-mcp-stdio.jar`); `FrapRpcClient` extracts the right binary for the
current platform automatically.

For local development you can drop a freshly built binary here by hand (e.g.
`frap-core-rpc-macos-aarch64` on Apple Silicon) or point `FRAP_CORE_BIN` /
`crates/target/release` at a dev build — those take precedence over the bundled resource.
