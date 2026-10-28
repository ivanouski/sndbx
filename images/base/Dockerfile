FROM ubuntu:26.10 AS system

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

ENV DEBIAN_FRONTEND=noninteractive
ENV LANG=C.UTF-8 LC_ALL=C.UTF-8

RUN apt update && apt upgrade -y && apt install -y --no-install-recommends \
  ca-certificates tzdata tini curl wget git openssh-client \
  jq ripgrep fd-find less file time unzip zip iproute2 sqlite3 \
  build-essential pkg-config libffi-dev libyaml-dev zlib1g-dev libpq-dev libsqlite3-dev libvips42t64 \
  libglib2.0-0t64 libnspr4 libnss3 libatk1.0-0t64 libatspi2.0-0t64 libxkbcommon0 \
  libxcomposite1 libxdamage1 libxfixes3 libxrandr2 libgbm1 libasound2t64 \
  fonts-liberation fonts-noto-color-emoji \
  && rm -rf /var/lib/apt/lists/* \
  && ln -s /usr/bin/fdfind /usr/local/bin/fd

ENTRYPOINT ["/usr/bin/tini", "--"]

FROM system AS tools

ARG USER=ubuntu
ARG NODE_VERSION=26.9.0
ARG PYTHON_VERSION=3.14.7
ARG RUBY_VERSION=4.0.7
ARG RUST_VERSION=1.99
ARG POSTGRES_VERSION=18.6
# sndbx passes the host user's on Linux, so the agent can write to the mounts
ARG UID=1000
ARG GID=1000

# Before the tools, so they're owned by the user too; -o allows ids the image already has.
# usermod moves the user's record in lastlog and faillog to an offset of uid × record size, which a layer stores in
# full, so a large uid would make it huge
RUN rm -f /var/log/lastlog /var/log/faillog \
  && groupmod -o -g "${GID}" "${USER}" \
  && usermod -o -u "${UID}" "${USER}" \
  && chown -R "${UID}:${GID}" "/home/${USER}"

USER ${USER}
ENV HOME=/home/${USER}

RUN curl -fsSL https://mise.run | bash

RUN eval "$(~/.local/bin/mise activate bash)" \
  && mise use --global \
    node@${NODE_VERSION} \
    pnpm@latest \
    playwright@latest \
    python@${PYTHON_VERSION} \
    ruby@${RUBY_VERSION} \
    "rust[profile=minimal]@${RUST_VERSION}" \
    uv@latest \
    postgres@${POSTGRES_VERSION} \
    gem:rubocop@latest \
  && rustup component add clippy rustfmt \
  && playwright install --only-shell chromium \
  && mise cache clear \
  && npm cache clean --force

ENV PATH=${HOME}/.local/bin:${HOME}/.local/share/mise/shims:$PATH

WORKDIR ${HOME}
