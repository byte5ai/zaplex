FROM example.invalid/tool:1.2.3@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
RUN set -eux; \
    curl --fail https://example.invalid/project/HEAD/install.sh --output /tmp/install.sh; \
    echo "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  /tmp/install.sh" | sha256sum --check -
