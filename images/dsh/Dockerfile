ARG TAG_PREFIX=sndbx
FROM ${TAG_PREFIX}:base

ARG NODE_VERSION=26.9.0

RUN mise use --global node@${NODE_VERSION} pnpm@latest \
  && mise cache clear

RUN git clone --depth 1 https://github.com/deepseek-ai/deepseek-harness ~/dsh

WORKDIR ${HOME}/dsh

RUN pnpm install \
  && pnpm run build

EXPOSE 3080/tcp

WORKDIR /workspace

CMD exec node ~/dsh/apps/cli/lib/bin.js web --no-open --host "$(hostname -i | cut -d' ' -f1)"
