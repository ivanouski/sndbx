ARG TAG_PREFIX=sndbx
FROM ${TAG_PREFIX}:base

RUN mise use --global codex@latest

ENV CODEX_HOME=${HOME}/.codex

WORKDIR /workspace

CMD ["codex"]
