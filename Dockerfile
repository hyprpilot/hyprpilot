# The image ships the whole CLI; the caller picks the subcommand, e.g.
#   docker run ghcr.io/hyprpilot/hyprpilot mcp passthrough --transport http \
#     --listen 0.0.0.0:8080 --allow-remote --tool '<json>'
# Nothing is EXPOSEd: the transport and port are flags, not image facts.

# Same Debian release as the runtime below, so the glibc the binary links
# against is the one it runs on.
FROM docker.io/library/rust:1-trixie AS build

WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY src ./src

# `task release` is `cargo build --release`; go-task is not in this image.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
    && install -D target/release/hyprpilot /out/hyprpilot

# distroless/cc carries glibc, libgcc and the CA bundle reqwest's rustls
# platform verifier reads for an `https` upstream, and no shell.
FROM gcr.io/distroless/cc-debian13:nonroot

COPY --from=build /out/hyprpilot /usr/local/bin/hyprpilot

USER nonroot:nonroot

ENTRYPOINT ["/usr/local/bin/hyprpilot"]
# Bare `hyprpilot` is the interactive launcher, which has no business in a
# container.
CMD ["--help"]
