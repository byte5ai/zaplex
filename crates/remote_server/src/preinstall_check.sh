#!/usr/bin/env bash
# Zaplex remote-server binary pre-installation check.
#
# stdout outputs structured key=value summary. Exit code 0 means detection complete;
# non-zero means detection process failed, client will treat as `status=unknown` and fail open.
#
# IMPORTANT: Zaplex Linux remote-server is now statically linked via zap_release.yml with
# `x86_64-unknown-linux-musl` target (static-musl). Artifacts don't depend on
# host's dynamic libc, can run on any Linux x86_64 host — including old glibc
# distributions (CentOS 7 = 2.17, Amazon Linux 2 = 2.26, Ubuntu 20.04 / Debian 11
# = 2.31) and musl distributions (Alpine etc).
#
# Since the binary is static, libc detection is no longer used as a "gating check", only kept as telemetry.

set -u

# Legacy field: keep required_glibc for backward compatibility with old client parsing logic.
# Static musl binary actually has no glibc lower bound, output here is only for backward compatibility,
# no longer participates in the status determination below.
required_glibc="2.17"
echo "required_glibc=${required_glibc}"

# 1. Identify libc family, and identify version in glibc scenario (pure telemetry, doesn't affect status).
libc_family="unknown"
libc_version=""

if version=$(getconf GNU_LIBC_VERSION 2>/dev/null); then
    # Output like: "glibc 2.35"
    libc_family="glibc"
    libc_version="${version##* }"
elif ldd_out=$(ldd --version 2>&1 | head -n1); then
    case "$ldd_out" in
        *musl*)   libc_family="musl"   ;;
        *uClibc*) libc_family="uclibc" ;;
        *)
            v=$(printf '%s\n' "$ldd_out" | grep -oE '[0-9]+\.[0-9]+' | head -n1)
            if [ -n "$v" ]; then
                libc_family="glibc"
                libc_version="$v"
            fi
            ;;
    esac
fi

echo "libc_family=${libc_family}"
[ -n "$libc_version" ] && echo "libc_version=${libc_version}"

# 2. Determine support status.
#
# remote-server is a static musl binary, doesn't link host libc, so any glibc version
# (including 2.35 and below) and musl / uclibc hosts can run it. As long as successfully
# identify this is a Linux x86_64 host, report `supported`; when unable to detect any libc clues
# (even getconf and ldd fail), fall back to `unknown`, let client fail open and try normal installation.
status="unknown"
reason=""

if [ "$libc_family" = "glibc" ] \
   || [ "$libc_family" = "musl" ] \
   || [ "$libc_family" = "uclibc" ] \
   || [ "$libc_family" = "bionic" ]; then
    status="supported"
fi

echo "status=${status}"
if [ -n "$reason" ]; then
    echo "reason=${reason}"
fi
