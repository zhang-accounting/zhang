FROM gcr.io/distroless/cc-debian12
LABEL org.opencontainers.image.source https://github.com/kilerd/zhang


COPY --chmod=755 zhang /application/
# the official MCP plugin, to copy into a ledger (or declare by this path): see the MCP guide of the docs
COPY plugins/mcp/mcp.lua /usr/share/zhang/plugins/mcp.lua

ENV ZHANG_AUTH=""
ENV RUST_LOG="info"
WORKDIR application
VOLUME "/data"
EXPOSE 8000

ENTRYPOINT ["./zhang", "serve", "/data", "--port", "8000"]
