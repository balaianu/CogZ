FROM rust:slim-bookworm@sha256:24707e70a4f1e23d9a7b25202ec43126d28a52294b96aad5c670df1fd1f1d400 AS build
RUN apt-get update && apt-get install -y --no-install-recommends g++ \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
RUN cargo build --release --locked

FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc
LABEL io.modelcontextprotocol.server.name="io.github.balaianu/cogz"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /usr/sbin/nologin cogz
# git is intentionally absent — co-change mining uses git2 (libgit2).
USER cogz
COPY --from=build /app/target/release/cogz /usr/local/bin/cogz
ENTRYPOINT ["cogz", "mcp-stdio"]
