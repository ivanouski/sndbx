FROM ubuntu:26.10

ARG USER=ubuntu
ARG NODE_VERSION=26.9.0
ARG PYTHON_VERSION=3.14.7
ARG RUBY_VERSION=4.0.7
ARG RUST_VERSION=1.99
ARG POSTGRES_VERSION=18.6

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

ENV DEBIAN_FRONTEND=noninteractive
ENV LANG=C.UTF-8 LC_ALL=C.UTF-8

RUN apt update && apt upgrade -y && apt install -y --no-install-recommends \
  curl ca-certificates git openssh-client jq \
  build-essential libpq-dev \
  libglib2.0-0t64 libnspr4 libnss3 libatk1.0-0t64 \
  libxkbcommon0 libasound2t64 libgbm1 libxcomposite1 libxdamage1 \
  libxfixes3 libxrandr2 libatspi2.0-0t64 \
  fonts-liberation fonts-noto-color-emoji tini \
  pkg-config libyaml-dev zlib1g-dev libffi-dev sqlite3 libsqlite3-dev libvips42t64 \
  unzip zip less file iproute2 wget ripgrep fd-find \
  && rm -rf /var/lib/apt/lists/* \
  && ln -s /usr/bin/fdfind /usr/local/bin/fd

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

ENTRYPOINT ["/usr/bin/tini", "--"]

WORKDIR ${HOME}
