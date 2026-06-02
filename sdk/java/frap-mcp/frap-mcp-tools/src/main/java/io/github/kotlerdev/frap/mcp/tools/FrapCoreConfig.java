package io.github.kotlerdev.frap.mcp.tools;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.PropertyNamingStrategies;

import io.github.kotlerdev.frap.core.client.FrapCoreClient;
import io.github.kotlerdev.frap.core.client.FrapRpcClient;

import java.io.IOException;

import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;
import org.springframework.context.annotation.Lazy;

/**
 * Spring configuration that wires the {@link FrapCoreClient} bean.
 *
 * <p>The client is created via {@link FrapRpcClient#create()}, which spawns the
 * native {@code frap-core-rpc} binary bundled inside {@code frap-core-java}. The
 * bean uses {@code destroyMethod = "close"} so the subprocess is terminated when the
 * application context shuts down (no separate {@code @PreDestroy} required, since
 * {@link FrapCoreClient} extends {@link AutoCloseable}).</p>
 */
@Configuration
public class FrapCoreConfig {

    /**
     * Creates the long-lived frap core client backed by the bundled native binary.
     *
     * @return an open {@link FrapCoreClient}
     * @throws IOException if the native binary cannot be extracted or started
     */
    @Bean(destroyMethod = "close")
    @Lazy
    public FrapCoreClient frapCoreClient() throws IOException {
        return FrapRpcClient.create();
    }

    /**
     * Jackson mapper used by {@code ArtifactStore} to read/write frap artifacts on disk.
     *
     * <p>Configured exactly like {@link FrapRpcClient}'s internal mapper (and the
     * {@code FrapToolsIT} fixture): {@code SNAKE_CASE} naming to match the frap JSON
     * contract, tolerant of unknown properties on read, and {@code NON_NULL} inclusion
     * so written artifacts stay compact. Named distinctly to avoid clashing with any
     * Spring Boot auto-configured {@code ObjectMapper}.</p>
     *
     * @return a snake-case, tolerant, non-null {@link ObjectMapper}
     */
    @Bean
    public ObjectMapper frapArtifactObjectMapper() {
        return new ObjectMapper()
            .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)
            .disable(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES)
            .setSerializationInclusion(JsonInclude.Include.NON_NULL);
    }
}
