FROM ubuntu:26.10

ARG USER=ubuntu
ARG NODE_VERSION=26.9.0
ARG PYTHON_VERSION=3.14.7
ARG RUBY_VERSION=4.0.7
ARG RUST_VERSION=1.99

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

ENV DEBIAN_FRONTEND=noninteractive

RUN apt update && apt upgrade -y && apt install -y --no-install-recommends \
  curl ca-certificates git openssh-client jq iputils-ping gcc gpg make \
  build-essential libpq-dev postgresql \
  libglib2.0-0t64 libnspr4 libnss3 libatk1.0-0t64 libatk-bridge2.0-0t64 libcups2t64 \
  libxkbcommon0 libasound2t64 libgbm1 libxcomposite1 libxdamage1 \
  libxfixes3 libxrandr2 libatspi2.0-0t64 librsvg2-bin \
  && rm -rf /var/lib/apt/lists/*

USER ${USER}
ENV HOME=/home/${USER}

RUN curl -fsSL https://mise.run | bash

RUN eval "$(~/.local/bin/mise activate bash)" \
  && mise use --global \
    node@${NODE_VERSION} \
    python@${PYTHON_VERSION} \
    ruby@${RUBY_VERSION} \
    rust@${RUST_VERSION} \
  && npm install -g npm@latest pnpm chromium playwright

ENV PATH=${HOME}/.local/share/mise/shims:$PATH

WORKDIR ${HOME}
