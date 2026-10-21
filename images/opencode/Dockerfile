ARG TAG_PREFIX=sndbx
FROM ${TAG_PREFIX}:base

RUN mkdir -p ~/.config ~/.local/share
RUN mise use --global opencode@latest

WORKDIR /workspace

CMD ["opencode"]
