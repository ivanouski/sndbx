ARG TAG_PREFIX=sndbx
FROM ${TAG_PREFIX}:base

RUN curl -fsSL https://cursor.com/install | bash

# cursor-agent keeps its config in ~/.cursor
# but its login in ~/.config/cursor, so both land in the mounted config
RUN mkdir -p ~/.config && ln -s ~/.cursor ~/.config/cursor

WORKDIR /workspace

# The container is the sandbox, like codex's
CMD ["cursor-agent", "--sandbox", "disabled"]
