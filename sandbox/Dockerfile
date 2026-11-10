# OmniForge sandbox image: the toolchain available to agents.
#
# Sandboxes run with `--network none`: everything agents need (test runner, linter,
# structural search) must be baked in here. Extend this image for your own stack.
FROM python:3.12-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Test runner, linter and AST-level code search (ast-grep).
RUN pip install --no-cache-dir pytest==8.* ruff ast-grep-cli

# Optional Node.js toolchain for frontend tasks: --build-arg WITH_NODE=true
ARG WITH_NODE=false
RUN if [ "$WITH_NODE" = "true" ]; then \
        apt-get update && apt-get install -y --no-install-recommends nodejs npm && rm -rf /var/lib/apt/lists/*; \
    fi

# Sandboxes run as an arbitrary unprivileged uid (the API's user): keep HOME writable and
# let git operate on a bind-mounted workspace it does not "own".
ENV HOME=/tmp PYTHONDONTWRITEBYTECODE=1 PIP_DISABLE_PIP_VERSION_CHECK=1
RUN git config --system --add safe.directory /workspace

WORKDIR /workspace
CMD ["sleep", "infinity"]
