FROM example.invalid/tool:1.2.3@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
RUN set -eux; \
    curl --fail https://example.invalid/releases/download/v1.2.3/install.sh | sh
