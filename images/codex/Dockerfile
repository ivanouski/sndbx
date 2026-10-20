ARG TAG_PREFIX=sndbx
FROM ${TAG_PREFIX}:base

RUN mise use --global codex@latest

ENV CODEX_HOME=${HOME}/.codex

WORKDIR /workspace

# The container is the sandbox: codex's own, bubblewrap, needs user namespaces, which docker's seccomp profile blocks
CMD ["codex", "--sandbox", "danger-full-access"]
