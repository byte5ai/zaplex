FROM example.invalid/tool:1.2.3@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
RUN set -eux; \
    curl --fail https://example.invalid/releases/download/v1.2.3/tool.tar.gz --output /tmp/tool.tar.gz; \
    tar -xzf /tmp/tool.tar.gz; \
    echo "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  /tmp/tool.tar.gz" | sha256sum --check -
