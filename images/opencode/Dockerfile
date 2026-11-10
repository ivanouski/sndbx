ARG TAG_PREFIX=sndbx
FROM ${TAG_PREFIX}:base

RUN mkdir -p ~/.config ~/.local/share
# aqua installs the release binary; mise's default, npm, needs node for its postinstall
RUN mise use --global aqua:anomalyco/opencode@latest

WORKDIR /workspace

CMD ["opencode"]
